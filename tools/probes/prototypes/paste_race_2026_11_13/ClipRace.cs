// Paste-race probe (ticket "paste reports success but nothing lands").
// Prototype; the finding it backs is in docs/findings/2026-11-13-windows-paste-race.md.
//
// Question: after Murmur puts the text on the clipboard, how often does a
// standard EDIT control's paste find the clipboard still held by a
// clipboard listener (Win+V history, Phone Link, PowerToys...)? EDIT's
// WM_PASTE calls OpenClipboard once and silently does nothing if it fails.
//
// Each trial: clear an EDIT (WinForms TextBox, own STA thread), put text on
// the clipboard exactly as Murmur 1.3 does (CF_UNICODETEXT + the four
// privacy formats, NULL owner), wait `delay` ms, send WM_PASTE, read the
// EDIT back. A watcher thread records which process holds the clipboard
// (GetOpenClipboardWindow) and when, from the write until 300 ms after.
// No keyboard input: runs on a locked desktop too.
using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.Runtime.InteropServices;
using System.Text;
using System.Threading;
using System.Windows.Forms;

namespace PasteRace
{
    public static class Probe
    {
        [DllImport("user32.dll", SetLastError = true)] static extern bool OpenClipboard(IntPtr h);
        [DllImport("user32.dll")] static extern bool CloseClipboard();
        [DllImport("user32.dll")] static extern bool EmptyClipboard();
        [DllImport("user32.dll")] static extern IntPtr SetClipboardData(uint f, IntPtr h);
        [DllImport("user32.dll")] static extern IntPtr GetClipboardData(uint f);
        [DllImport("user32.dll")] static extern IntPtr GetOpenClipboardWindow();
        [DllImport("user32.dll")] static extern uint GetClipboardSequenceNumber();
        [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern uint RegisterClipboardFormatW(string n);
        [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
        [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern IntPtr SendMessageW(IntPtr h, uint m, IntPtr w, string l);
        [DllImport("user32.dll")] static extern IntPtr SendMessageW(IntPtr h, uint m, IntPtr w, IntPtr l);
        [DllImport("kernel32.dll")] static extern IntPtr GlobalAlloc(uint f, UIntPtr n);
        [DllImport("kernel32.dll")] static extern IntPtr GlobalLock(IntPtr h);
        [DllImport("kernel32.dll")] static extern bool GlobalUnlock(IntPtr h);
        [DllImport("winmm.dll")] static extern uint timeBeginPeriod(uint ms);

        const uint CF_UNICODETEXT = 13, WM_SETTEXT = 0x0C, WM_GETTEXTLENGTH = 0x0E, WM_PASTE = 0x302;
        static readonly string[] Privacy = { "ExcludeClipboardContentFromMonitorProcessing", "CanIncludeInClipboardHistory", "CanUploadToCloudClipboard", "Clipboard Viewer Ignore" };
        static readonly Stopwatch Clock = Stopwatch.StartNew();
        static double Now() { return Clock.Elapsed.TotalMilliseconds; }

        public static int LastError;

        static bool Open()
        {
            for (int i = 0; i < 50; i++) { if (OpenClipboard(IntPtr.Zero)) return true; LastError = Marshal.GetLastWin32Error(); Thread.Sleep(5); }
            return false;
        }

        static void SetBytes(uint fmt, byte[] b)
        {
            IntPtr g = GlobalAlloc(2, (UIntPtr)Math.Max(1, b.Length));
            IntPtr p = GlobalLock(g);
            Marshal.Copy(b, 0, p, b.Length);
            GlobalUnlock(g);
            SetClipboardData(fmt, g);
        }

        /// Murmur 1.3 paste::win::set_text_excluded_from_history.
        public static bool SetLikeMurmur(string text)
        {
            if (!Open()) return false;
            EmptyClipboard();
            SetBytes(CF_UNICODETEXT, Encoding.Unicode.GetBytes(text + "\0"));
            foreach (var n in Privacy) SetBytes(RegisterClipboardFormatW(n), new byte[4]);
            CloseClipboard();
            return true;
        }

        public static string ReadText()
        {
            if (!Open()) return null;
            try
            {
                IntPtr h = GetClipboardData(CF_UNICODETEXT);
                if (h == IntPtr.Zero) return null;
                IntPtr p = GlobalLock(h);
                string s = Marshal.PtrToStringUni(p);
                GlobalUnlock(h);
                return s;
            }
            finally { CloseClipboard(); }
        }

        static string Holder(IntPtr h)
        {
            if (h == IntPtr.Zero) return null;
            uint pid; GetWindowThreadProcessId(h, out pid);
            try { return Process.GetProcessById((int)pid).ProcessName; } catch { return "pid" + pid; }
        }

        static void SpinUntil(double t) { while (Now() < t) { if (t - Now() > 2) Thread.Sleep(1); } }

        /// Wait for the listener burst our write sets off: until nobody has
        /// held the clipboard for `quietMs` (first `minMs`, at most `capMs`).
        static double Settle(double minMs, double quietMs, double capMs)
        {
            double t0 = Now(), quietSince = t0;
            SpinUntil(t0 + minMs);
            while (Now() - t0 < capMs)
            {
                if (GetOpenClipboardWindow() != IntPtr.Zero) quietSince = Now();
                else if (Now() - quietSince >= quietMs && Now() - t0 >= minMs) break;
                Thread.Sleep(1);
            }
            return Now() - t0;
        }

        /// Runs `trials` per strategy. Strategies: "d<ms>" = fixed delay,
        /// "settle" = Settle(5, 25, 200). One JSON line per trial.
        public static List<string> Run(string[] strategies, int trials)
        {
            timeBeginPeriod(1);
            var outLines = new List<string>();
            IntPtr edit = IntPtr.Zero;
            var ready = new ManualResetEvent(false);
            Form form = null;
            var ui = new Thread(() =>
            {
                form = new Form { ShowInTaskbar = false, Opacity = 0 };
                var box = new TextBox { Multiline = true };
                form.Controls.Add(box);
                form.Load += (s, e) => { edit = box.Handle; ready.Set(); };
                Application.Run(form);
            });
            ui.SetApartmentState(ApartmentState.STA);
            ui.IsBackground = true;
            ui.Start();
            ready.WaitOne();
            string saved = ReadText();
            try
            {
                foreach (var strat in strategies)
                {
                    for (int i = 0; i < trials; i++)
                    {
                        SendMessageW(edit, WM_SETTEXT, IntPtr.Zero, "");
                        var holders = new List<string>();
                        bool stop = false;
                        double tWrite = 0;
                        var watch = new Thread(() =>
                        {
                            IntPtr last = IntPtr.Zero;
                            while (!Volatile.Read(ref stop))
                            {
                                IntPtr h = GetOpenClipboardWindow();
                                if (h != last)
                                {
                                    lock (holders) holders.Add(string.Format("{0:F1}:{1}", Now() - tWrite, Holder(h) ?? "-"));
                                    last = h;
                                }
                                Thread.SpinWait(200);
                            }
                        });
                        tWrite = Now();
                        watch.Start();
                        bool set = SetLikeMurmur("probe " + strat + " " + i);
                        double tClosed = Now();
                        double waited;
                        if (strat == "settle") waited = Settle(5, 25, 200);
                        else { SpinUntil(tClosed + double.Parse(strat.Substring(1))); waited = Now() - tClosed; }
                        string atPaste = Holder(GetOpenClipboardWindow());
                        SendMessageW(edit, WM_PASTE, IntPtr.Zero, IntPtr.Zero);
                        int len = (int)SendMessageW(edit, WM_GETTEXTLENGTH, IntPtr.Zero, IntPtr.Zero);
                        Thread.Sleep(300);
                        Volatile.Write(ref stop, true);
                        watch.Join();
                        string hs;
                        lock (holders) hs = string.Join(",", holders.ToArray());
                        outLines.Add(string.Format(
                            "{{\"strategy\":\"{0}\",\"i\":{1},\"set\":{2},\"err\":{7},\"waited_ms\":{3:F1},\"holder_at_paste\":\"{4}\",\"pasted\":{5},\"holders\":\"{6}\"}}",
                            strat, i, set ? "true" : "false", waited, atPaste ?? "", len > 0 ? "true" : "false", hs, LastError));
                        Thread.Sleep(250);
                    }
                }
            }
            finally
            {
                if (saved != null) SetLikeMurmur(saved);
                form.Invoke(new Action(() => form.Close()));
            }
            return outLines;
        }
    }
}
