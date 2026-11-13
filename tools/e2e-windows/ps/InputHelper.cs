// Real key presses with exact timings (SendInput) and a few window helpers
// for the Windows e2e harness. Loaded with Add-Type in the interactive
// session (agent.ps1). Injected input carries dwExtraInfo = 0, so Murmur's
// hook treats it like any other injected input (it skips only its own tag).
using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.Drawing;
using System.Drawing.Imaging;
using System.Runtime.InteropServices;
using System.Text;
using System.Threading;

namespace MurmurE2E
{
    public static class Keys
    {
        [StructLayout(LayoutKind.Sequential)]
        struct KEYBDINPUT { public ushort wVk, wScan; public uint dwFlags, time; public IntPtr dwExtraInfo; }
        [StructLayout(LayoutKind.Sequential)]
        struct MOUSEINPUT { public int dx, dy; public uint mouseData, dwFlags, time; public IntPtr dwExtraInfo; }
        [StructLayout(LayoutKind.Explicit)]
        struct InputUnion { [FieldOffset(0)] public MOUSEINPUT mi; [FieldOffset(0)] public KEYBDINPUT ki; }
        [StructLayout(LayoutKind.Sequential)]
        struct INPUT { public uint type; public InputUnion u; }

        [DllImport("user32.dll", SetLastError = true)] static extern uint SendInput(uint n, INPUT[] inputs, int size);
        [DllImport("user32.dll")] static extern uint MapVirtualKey(uint code, uint mapType);
        [DllImport("winmm.dll")] public static extern uint timeBeginPeriod(uint ms);
        [DllImport("winmm.dll")] public static extern uint timeEndPeriod(uint ms);

        const uint KEYUP = 0x2, EXTENDED = 0x1;

        // name -> (vk, extended)
        static readonly Dictionary<string, Tuple<ushort, bool>> Map = new Dictionary<string, Tuple<ushort, bool>>(StringComparer.OrdinalIgnoreCase)
        {
            { "RAlt", Tuple.Create((ushort)0xA5, true) }, { "LAlt", Tuple.Create((ushort)0xA4, false) },
            { "Alt", Tuple.Create((ushort)0xA4, false) },
            { "RCtrl", Tuple.Create((ushort)0xA3, true) }, { "LCtrl", Tuple.Create((ushort)0xA2, false) },
            { "Ctrl", Tuple.Create((ushort)0xA2, false) },
            { "LShift", Tuple.Create((ushort)0xA0, false) }, { "RShift", Tuple.Create((ushort)0xA1, false) },
            { "Shift", Tuple.Create((ushort)0xA0, false) },
            { "Win", Tuple.Create((ushort)0x5B, true) }, { "LWin", Tuple.Create((ushort)0x5B, true) },
            { "Space", Tuple.Create((ushort)0x20, false) }, { "Esc", Tuple.Create((ushort)0x1B, false) },
            { "Enter", Tuple.Create((ushort)0x0D, false) }, { "Tab", Tuple.Create((ushort)0x09, false) },
            { "Delete", Tuple.Create((ushort)0x2E, true) }, { "Backspace", Tuple.Create((ushort)0x08, false) },
            // The Left Ctrl an AltGr layout sends ahead of Right Alt (scan 0x21D).
            { "AltGrCtrl", Tuple.Create((ushort)0xA2, false) },
        };
        static readonly Dictionary<string, ushort> ScanOverride = new Dictionary<string, ushort>(StringComparer.OrdinalIgnoreCase)
        {
            { "AltGrCtrl", 0x21D },
        };

        static Tuple<ushort, bool> Resolve(string name)
        {
            Tuple<ushort, bool> k;
            if (Map.TryGetValue(name, out k)) return k;
            if (name.Length == 1 && char.IsLetterOrDigit(name[0])) return Tuple.Create((ushort)char.ToUpperInvariant(name[0]), false);
            if (name.StartsWith("F") && name.Length <= 3) return Tuple.Create((ushort)(0x6F + int.Parse(name.Substring(1))), false);
            throw new ArgumentException("unknown key: " + name);
        }

        static void Send(string name, bool up)
        {
            var k = Resolve(name);
            var i = new INPUT { type = 1 };
            i.u.ki.wVk = k.Item1;
            ushort scan;
            i.u.ki.wScan = ScanOverride.TryGetValue(name, out scan) ? scan : (ushort)MapVirtualKey(k.Item1, 0);
            i.u.ki.dwFlags = (up ? KEYUP : 0) | (k.Item2 ? EXTENDED : 0);
            if (SendInput(1, new[] { i }, Marshal.SizeOf(typeof(INPUT))) != 1)
                throw new Exception("SendInput failed (" + Marshal.GetLastWin32Error() + "): is the desktop locked?");
        }

        static readonly Stopwatch Clock = Stopwatch.StartNew();
        static double Now() { return Clock.Elapsed.TotalMilliseconds; }

        /// Sleep until `t` (ms on Clock): Sleep for the bulk, spin the last 2 ms.
        static void Until(double t)
        {
            for (;;)
            {
                double left = t - Now();
                if (left <= 0) return;
                if (left > 3) Thread.Sleep((int)(left - 2)); else Thread.SpinWait(200);
            }
        }

        public static void Down(string[] chord) { foreach (var k in chord) Send(k, false); }
        public static void Up(string[] chord) { for (int i = chord.Length - 1; i >= 0; i--) Send(chord[i], true); }

        /// Press `chord` (keys in order), hold `holdMs`, release (reverse
        /// order). Returns the measured hold in ms.
        public static double Hold(string[] chord, int holdMs)
        {
            timeBeginPeriod(1);
            try
            {
                double t0 = Now();
                Down(chord);
                Until(t0 + holdMs);
                Up(chord);
                return Now() - t0;
            }
            finally { timeEndPeriod(1); }
        }

        /// Two taps: hold `tapMs`, release, wait `gapMs` (release to second
        /// press), hold `tapMs`, release. Returns {tap1, gap, tap2} measured.
        public static double[] Double(string[] chord, int tapMs, int gapMs) { return Double(chord, tapMs, gapMs, tapMs); }

        /// As above with a different hold for the second tap.
        public static double[] Double(string[] chord, int tapMs, int gapMs, int tap2Ms)
        {
            timeBeginPeriod(1);
            try
            {
                double t0 = Now();
                Down(chord);
                Until(t0 + tapMs);
                Up(chord);
                double t1 = Now();
                Until(t1 + gapMs);
                Down(chord);
                double t2 = Now();
                Until(t2 + tap2Ms);
                Up(chord);
                double t3 = Now();
                return new[] { t1 - t0, t2 - t1, t3 - t2 };
            }
            finally { timeEndPeriod(1); }
        }

        /// Double() that only presses while window `guard` (0 = any) is in
        /// the foreground. Returns {tap1, gap, tap2, abortedAt}: abortedAt 0 =
        /// done, 1 = nothing pressed (not foreground at the start), 2 = the
        /// first tap went out but the second did not, 3 = the foreground
        /// changed during the second tap (it is released anyway).
        public static double[] DoubleGuarded(string[] chord, int tapMs, int gapMs, int tap2Ms, long guard)
        {
            Func<bool> ok = () => guard == 0 || Windows.GetForegroundWindow().ToInt64() == guard;
            timeBeginPeriod(1);
            try
            {
                if (!ok()) return new double[] { 0, 0, 0, 1 };
                double t0 = Now();
                Down(chord);
                Until(t0 + tapMs);
                Up(chord);
                double t1 = Now();
                Until(t1 + gapMs);
                if (!ok()) return new double[] { t1 - t0, 0, 0, 2 };
                Down(chord);
                double t2 = Now();
                Until(t2 + tap2Ms);
                bool still = ok();
                Up(chord);
                double t3 = Now();
                return new double[] { t1 - t0, t2 - t1, t3 - t2, still ? 0 : 3 };
            }
            finally { timeEndPeriod(1); }
        }

        /// Type text with KEYEVENTF_UNICODE (for filling a target field).
        public static void Type(string text)
        {
            foreach (char c in text)
            {
                var d = new INPUT { type = 1 };
                d.u.ki.wScan = c; d.u.ki.dwFlags = 0x4;
                var u = d; u.u.ki.dwFlags = 0x4 | KEYUP;
                SendInput(2, new[] { d, u }, Marshal.SizeOf(typeof(INPUT)));
                Thread.Sleep(5);
            }
        }
    }

    public class WinInfo
    {
        public long Hwnd;
        public string Title;
        public string Class;
        public int Pid;
        public string Process;
        public int X, Y, W, H;
    }

    public static class Windows
    {
        [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
        delegate bool EnumProc(IntPtr h, IntPtr p);
        [DllImport("user32.dll")] static extern bool EnumWindows(EnumProc cb, IntPtr p);
        [DllImport("user32.dll")] static extern bool IsWindowVisible(IntPtr h);
        [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern int GetWindowText(IntPtr h, StringBuilder s, int n);
        [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern int GetClassName(IntPtr h, StringBuilder s, int n);
        [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr h, out int pid);
        [DllImport("user32.dll")] static extern bool GetWindowRect(IntPtr h, out RECT r);
        [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
        [DllImport("user32.dll")] static extern bool SetForegroundWindow(IntPtr h);
        [DllImport("user32.dll")] static extern bool ShowWindow(IntPtr h, int cmd);
        [DllImport("user32.dll")] static extern bool BringWindowToTop(IntPtr h);
        [DllImport("user32.dll")] static extern bool AttachThreadInput(uint a, uint b, bool attach);
        [DllImport("kernel32.dll")] static extern uint GetCurrentThreadId();
        [DllImport("user32.dll")] static extern IntPtr PostMessage(IntPtr h, uint msg, IntPtr w, IntPtr l);
        [DllImport("user32.dll")] static extern IntPtr OpenInputDesktop(uint flags, bool inherit, uint access);
        [DllImport("user32.dll")] static extern bool CloseDesktop(IntPtr h);
        [DllImport("user32.dll")] static extern bool SetProcessDPIAware();

        static Windows() { SetProcessDPIAware(); }

        static WinInfo Info(IntPtr h)
        {
            var t = new StringBuilder(512); GetWindowText(h, t, 512);
            var c = new StringBuilder(256); GetClassName(h, c, 256);
            int pid; GetWindowThreadProcessId(h, out pid);
            RECT r; GetWindowRect(h, out r);
            string pn = "";
            try { pn = Process.GetProcessById(pid).ProcessName; } catch { }
            return new WinInfo { Hwnd = h.ToInt64(), Title = t.ToString(), Class = c.ToString(), Pid = pid, Process = pn, X = r.L, Y = r.T, W = r.R - r.L, H = r.B - r.T };
        }

        /// Visible top-level windows (optionally only those of `process`).
        public static List<WinInfo> List(string process)
        {
            var all = new List<WinInfo>();
            EnumWindows((h, p) =>
            {
                if (IsWindowVisible(h))
                {
                    var i = Info(h);
                    if (string.IsNullOrEmpty(process) || string.Equals(i.Process, process, StringComparison.OrdinalIgnoreCase)) all.Add(i);
                }
                return true;
            }, IntPtr.Zero);
            return all;
        }

        public static WinInfo Foreground() { return Info(GetForegroundWindow()); }

        [DllImport("user32.dll")] static extern IntPtr GetKeyboardLayout(uint thread);
        [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern IntPtr LoadKeyboardLayout(string klid, uint flags);

        /// The keyboard layout (HKL, hex) of the thread that owns `hwnd`.
        public static string Layout(long hwnd)
        {
            int pid;
            uint tid = GetWindowThreadProcessId(new IntPtr(hwnd), out pid);
            return GetKeyboardLayout(tid).ToInt64().ToString("X");
        }

        /// Ask `hwnd` to switch its input language: `layout` is a KLID
        /// ("0000040D" = Hebrew) or an HKL in hex as Layout() returns it.
        /// Returns the layout after the switch.
        public static string SetLayout(long hwnd, string layout)
        {
            IntPtr hkl = layout.Length == 8 && layout.StartsWith("0000")
                ? LoadKeyboardLayout(layout, 0)
                : new IntPtr(Convert.ToInt64(layout, 16));
            PostMessage(new IntPtr(hwnd), 0x50 /* WM_INPUTLANGCHANGEREQUEST */, IntPtr.Zero, hkl);
            Thread.Sleep(300);
            return Layout(hwnd);
        }

        /// Bring `hwnd` to the foreground (AttachThreadInput trick).
        public static bool Focus(long hwnd)
        {
            var h = new IntPtr(hwnd);
            int pid;
            uint fg = GetWindowThreadProcessId(GetForegroundWindow(), out pid);
            uint me = GetCurrentThreadId();
            AttachThreadInput(me, fg, true);
            ShowWindow(h, 9); // SW_RESTORE
            BringWindowToTop(h);
            SetForegroundWindow(h);
            AttachThreadInput(me, fg, false);
            Thread.Sleep(150);
            return GetForegroundWindow() == h;
        }

        public static void Close(long hwnd) { PostMessage(new IntPtr(hwnd), 0x10, IntPtr.Zero, IntPtr.Zero); }

        [DllImport("user32.dll")] static extern bool EnumChildWindows(IntPtr parent, EnumProc cb, IntPtr p);
        [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern IntPtr SendMessage(IntPtr h, uint msg, IntPtr w, StringBuilder l);
        [DllImport("user32.dll", EntryPoint = "SendMessageW")] static extern IntPtr SendMessageLen(IntPtr h, uint msg, IntPtr w, IntPtr l);

        /// Text of the first EDIT-class child of `hwnd` (WM_GETTEXT), or null.
        public static string EditText(long hwnd)
        {
            IntPtr edit = IntPtr.Zero;
            EnumChildWindows(new IntPtr(hwnd), (h, p) =>
            {
                var c = new StringBuilder(256); GetClassName(h, c, 256);
                if (c.ToString().IndexOf("EDIT", StringComparison.OrdinalIgnoreCase) >= 0) { edit = h; return false; }
                return true;
            }, IntPtr.Zero);
            if (edit == IntPtr.Zero) return null;
            int n = SendMessageLen(edit, 0x0E, IntPtr.Zero, IntPtr.Zero).ToInt32(); // WM_GETTEXTLENGTH
            var sb = new StringBuilder(n + 1);
            SendMessage(edit, 0x0D, new IntPtr(n + 1), sb); // WM_GETTEXT
            return sb.ToString();
        }

        /// False while the PC is locked or the UAC desktop has the input:
        /// the session is locked (WTS), or the input desktop is not
        /// Default (OpenInputDesktop can still succeed on a locked
        /// session). Nothing can take the foreground then
        /// (GetForegroundWindow = 0, shown as fg=Idle).
        public static bool InputDesktopUsable()
        {
            if (SessionLocked()) return false;
            var d = OpenInputDesktop(0, false, 0x0100); // DESKTOP_SWITCHDESKTOP
            if (d == IntPtr.Zero) return false;
            try
            {
                // A GetUserObjectInformation failure counts as usable: only
                // a name we could read and that is not Default rules it out.
                var name = new StringBuilder(256);
                int needed;
                if (GetUserObjectInformation(d, 2 /* UOI_NAME */, name, name.Capacity * 2, out needed)
                    && !string.Equals(name.ToString(), "Default", StringComparison.OrdinalIgnoreCase)) return false;
            }
            finally { CloseDesktop(d); }
            return true;
        }

        /// True when WTS reports this session locked (WTSSessionInfoEx,
        /// SessionFlags == WTS_SESSIONSTATE_LOCK). Unlike a LogonUI process
        /// check, it does not linger after unlock. Unknown counts as unlocked.
        public static bool SessionLocked()
        {
            IntPtr buf;
            int bytes;
            if (!WTSQuerySessionInformation(IntPtr.Zero /* WTS_CURRENT_SERVER_HANDLE */, -1 /* WTS_CURRENT_SESSION */, 25 /* WTSSessionInfoEx */, out buf, out bytes)) return false;
            try
            {
                // WTSINFOEXW: DWORD Level, then the 8-byte-aligned union
                // WTSINFOEX_LEVEL1_W { ULONG SessionId; WTS_CONNECTSTATE_CLASS
                // SessionState; LONG SessionFlags; ... }.
                if (bytes < 20 || Marshal.ReadInt32(buf, 0) != 1) return false;
                return Marshal.ReadInt32(buf, 16) == 0; // WTS_SESSIONSTATE_LOCK
            }
            finally { WTSFreeMemory(buf); }
        }

        [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern bool GetUserObjectInformation(IntPtr h, int index, StringBuilder info, int len, out int needed);
        [DllImport("wtsapi32.dll", CharSet = CharSet.Unicode)] static extern bool WTSQuerySessionInformation(IntPtr server, int session, int infoClass, out IntPtr buf, out int bytes);
        [DllImport("wtsapi32.dll")] static extern void WTSFreeMemory(IntPtr p);

        [DllImport("user32.dll")] static extern bool PrintWindow(IntPtr h, IntPtr hdc, uint flags);

        /// PNG of one window's own content (PrintWindow with
        /// PW_RENDERFULLCONTENT): never the screen, so windows of the owner's
        /// that cover it are not captured.
        public static void Shot(long hwnd, string path)
        {
            RECT r; GetWindowRect(new IntPtr(hwnd), out r);
            int w = Math.Max(1, r.R - r.L), hgt = Math.Max(1, r.B - r.T);
            using (var bmp = new Bitmap(w, hgt))
            {
                using (var g = Graphics.FromImage(bmp))
                {
                    IntPtr dc = g.GetHdc();
                    try { PrintWindow(new IntPtr(hwnd), dc, 2); }
                    finally { g.ReleaseHdc(dc); }
                }
                bmp.Save(path, ImageFormat.Png);
            }
        }

        /// The first visible top-level window above `hwnd` in z-order that
        /// overlaps it (the user cannot fully see `hwnd`), or null.
        [DllImport("user32.dll")] static extern IntPtr GetWindow(IntPtr h, uint cmd);
        [DllImport("user32.dll")] static extern bool IsIconic(IntPtr h);
        public static WinInfo CoveredBy(long hwnd)
        {
            var h = new IntPtr(hwnd);
            RECT me; GetWindowRect(h, out me);
            for (IntPtr p = GetWindow(h, 3 /* GW_HWNDPREV */); p != IntPtr.Zero; p = GetWindow(p, 3))
            {
                if (!IsWindowVisible(p) || IsIconic(p)) continue;
                RECT o; GetWindowRect(p, out o);
                if (o.R - o.L < 100 || o.B - o.T < 100) continue; // tooltips, tray flyouts
                if (o.L < me.R && o.R > me.L && o.T < me.B && o.B > me.T) return Info(p);
            }
            return null;
        }
    }

    public class ChildInfo
    {
        public long Hwnd, Parent;
        public string Class, Text;
        public int Id;
        public uint Style;
        public bool Visible, Enabled;
    }

    /// Win32 dialogs of another process (the NSIS uninstaller): every child
    /// window, recursively, and a button's check state / click by message.
    /// UI Automation did not see the confirm page's check box.
    public static class Dlg
    {
        delegate bool EnumProc(IntPtr h, IntPtr p);
        [DllImport("user32.dll")] static extern bool EnumChildWindows(IntPtr parent, EnumProc cb, IntPtr p);
        [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern int GetClassName(IntPtr h, StringBuilder s, int n);
        [DllImport("user32.dll")] static extern int GetDlgCtrlID(IntPtr h);
        [DllImport("user32.dll")] static extern IntPtr GetParent(IntPtr h);
        [DllImport("user32.dll")] static extern int GetWindowLong(IntPtr h, int i);
        [DllImport("user32.dll")] static extern bool IsWindowVisible(IntPtr h);
        [DllImport("user32.dll")] static extern bool IsWindowEnabled(IntPtr h);
        [DllImport("user32.dll")] static extern bool IsWindow(IntPtr h);
        [DllImport("user32.dll")] static extern bool PostMessage(IntPtr h, uint msg, IntPtr w, IntPtr l);
        [DllImport("user32.dll")] static extern IntPtr SendMessageTimeout(IntPtr h, uint msg, IntPtr w, IntPtr l, uint flags, uint ms, out IntPtr result);
        [DllImport("user32.dll", CharSet = CharSet.Unicode, EntryPoint = "SendMessageTimeoutW")] static extern IntPtr SendMessageTimeoutText(IntPtr h, uint msg, IntPtr w, StringBuilder l, uint flags, uint ms, out IntPtr result);
        const uint SMTO_ABORTIFHUNG = 2, WM_GETTEXT = 0x0D, WM_COMMAND = 0x111, BM_GETCHECK = 0xF0, BM_SETCHECK = 0xF1;

        static string Text(IntPtr h)
        {
            var sb = new StringBuilder(512);
            IntPtr r;
            return SendMessageTimeoutText(h, WM_GETTEXT, new IntPtr(512), sb, SMTO_ABORTIFHUNG, 2000, out r) == IntPtr.Zero ? "" : sb.ToString();
        }

        public static List<ChildInfo> Children(long top)
        {
            var all = new List<ChildInfo>();
            EnumChildWindows(new IntPtr(top), (h, p) =>
            {
                var c = new StringBuilder(256); GetClassName(h, c, 256);
                all.Add(new ChildInfo
                {
                    Hwnd = h.ToInt64(), Parent = GetParent(h).ToInt64(), Class = c.ToString(), Id = GetDlgCtrlID(h),
                    Text = Text(h), Style = (uint)GetWindowLong(h, -16), Visible = IsWindowVisible(h), Enabled = IsWindowEnabled(h),
                });
                return true;
            }, IntPtr.Zero);
            return all;
        }

        /// A visible BUTTON with a check-box style (BS_CHECKBOX, BS_AUTOCHECKBOX, BS_3STATE, BS_AUTO3STATE).
        public static bool IsCheckBox(ChildInfo c)
        {
            uint t = c.Style & 0xF;
            return c.Visible && string.Equals(c.Class, "Button", StringComparison.OrdinalIgnoreCase) && (t == 2 || t == 3 || t == 5 || t == 6);
        }

        /// BM_GETCHECK: 0 off, 1 on, 2 indeterminate; -1 when the message failed.
        public static int GetCheck(long h)
        {
            IntPtr r;
            return SendMessageTimeout(new IntPtr(h), BM_GETCHECK, IntPtr.Zero, IntPtr.Zero, SMTO_ABORTIFHUNG, 2000, out r) == IntPtr.Zero ? -1 : r.ToInt32();
        }

        public static void SetCheck(long h, bool on)
        {
            IntPtr r;
            SendMessageTimeout(new IntPtr(h), BM_SETCHECK, new IntPtr(on ? 1 : 0), IntPtr.Zero, SMTO_ABORTIFHUNG, 2000, out r);
        }

        /// What a click on button `btn` (control id `id`) of dialog `top` sends: WM_COMMAND(BN_CLICKED).
        public static bool Click(long top, int id, long btn)
        {
            return PostMessage(new IntPtr(top), WM_COMMAND, new IntPtr(id & 0xFFFF), new IntPtr(btn));
        }

        public static bool Alive(long h) { return IsWindow(new IntPtr(h)); }
    }
}
