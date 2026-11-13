// Audio helpers for the Windows e2e harness, loaded with Add-Type:
// - list the audio endpoints and which are the defaults (Core Audio),
// - set a default endpoint (IPolicyConfig; used only to put the owner's
//   defaults back if a driver install moved them),
// - play a 16-bit PCM WAV into one named waveOut device (e.g. "CABLE Input")
//   without touching the default playback device.
using System;
using System.Collections.Generic;
using System.IO;
using System.Runtime.InteropServices;
using System.Threading;

namespace MurmurE2E
{
    [ComImport, Guid("A95664D2-9614-4F35-A746-DE8DB63617E6"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
    interface IMMDeviceEnumerator
    {
        int EnumAudioEndpoints(int dataFlow, int stateMask, out IMMDeviceCollection devices);
        int GetDefaultAudioEndpoint(int dataFlow, int role, out IMMDevice device);
    }

    [ComImport, Guid("0BD7A1BE-7A1A-44DB-8397-CC5392387B5E"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
    interface IMMDeviceCollection
    {
        int GetCount(out int count);
        int Item(int index, out IMMDevice device);
    }

    [ComImport, Guid("D666063F-1587-4E43-81F1-B948E807363F"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
    interface IMMDevice
    {
        int Activate(ref Guid iid, int clsCtx, IntPtr activationParams, [MarshalAs(UnmanagedType.IUnknown)] out object iface);
        int OpenPropertyStore(int access, out IPropertyStore store);
        int GetId([MarshalAs(UnmanagedType.LPWStr)] out string id);
        int GetState(out int state);
    }

    [ComImport, Guid("886d8eeb-8cf2-4446-8d02-cdba1dbdcf99"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
    interface IPropertyStore
    {
        int GetCount(out int count);
        int GetAt(int prop, out PropertyKey key);
        int GetValue(ref PropertyKey key, out PropVariant value);
    }

    [StructLayout(LayoutKind.Sequential)]
    struct PropertyKey
    {
        public Guid fmtid;
        public int pid;
    }

    [StructLayout(LayoutKind.Explicit)]
    struct PropVariant
    {
        [FieldOffset(0)] public short vt;
        [FieldOffset(8)] public IntPtr pointerValue;
    }

    [ComImport, Guid("f8679f50-850a-41cf-9c72-430f290290c8"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
    interface IPolicyConfig
    {
        int GetMixFormat();
        int GetDeviceFormat();
        int ResetDeviceFormat();
        int SetDeviceFormat();
        int GetProcessingPeriod();
        int SetProcessingPeriod();
        int GetShareMode();
        int SetShareMode();
        int GetPropertyValue();
        int SetPropertyValue();
        int SetDefaultEndpoint([MarshalAs(UnmanagedType.LPWStr)] string deviceId, int role);
        int SetEndpointVisibility();
    }

    [ComImport, Guid("5CDF2C82-841E-4546-9722-0CF74078229A"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
    interface IAudioEndpointVolume
    {
        int RegisterControlChangeNotify(IntPtr notify);
        int UnregisterControlChangeNotify(IntPtr notify);
        int GetChannelCount(out int count);
        int SetMasterVolumeLevel(float db, ref Guid ctx);
        int SetMasterVolumeLevelScalar(float level, ref Guid ctx);
        int GetMasterVolumeLevel(out float db);
        int GetMasterVolumeLevelScalar(out float level);
        int SetChannelVolumeLevel(int ch, float db, ref Guid ctx);
        int SetChannelVolumeLevelScalar(int ch, float level, ref Guid ctx);
        int GetChannelVolumeLevel(int ch, out float db);
        int GetChannelVolumeLevelScalar(int ch, out float level);
        int SetMute([MarshalAs(UnmanagedType.Bool)] bool mute, ref Guid ctx);
        int GetMute([MarshalAs(UnmanagedType.Bool)] out bool mute);
    }

    [ComImport, Guid("BCDE0395-E52F-467C-8E3D-C4579291692E")]
    class MMDeviceEnumeratorCom { }

    [ComImport, Guid("870af99c-171d-4f9e-af0d-e63df40c2bc9")]
    class PolicyConfigCom { }

    public class Endpoint
    {
        public string Id;
        public string Name;
        /// "render" (playback) or "capture" (recording).
        public string Flow;
        public bool DefaultConsole;
        public bool DefaultComms;
        public override string ToString()
        {
            return string.Format("{0}|{1}|{2}|{3}", Flow, Name, (DefaultConsole ? "default" : "") + (DefaultComms ? "+comms" : ""), Id);
        }
    }

    public static class Audio
    {
        const int Render = 0, Capture = 1, Active = 1;
        const int RoleConsole = 0, RoleMultimedia = 1, RoleComms = 2;

        static string FriendlyName(IMMDevice d)
        {
            IPropertyStore store;
            d.OpenPropertyStore(0, out store);
            var key = new PropertyKey { fmtid = new Guid("a45c254e-df1c-4efd-8020-67d146a850e0"), pid = 14 };
            PropVariant v;
            store.GetValue(ref key, out v);
            return v.vt == 31 ? Marshal.PtrToStringUni(v.pointerValue) : "?";
        }

        static string DefaultId(IMMDeviceEnumerator e, int flow, int role)
        {
            IMMDevice d;
            if (e.GetDefaultAudioEndpoint(flow, role, out d) != 0 || d == null) return null;
            string id;
            d.GetId(out id);
            return id;
        }

        public static List<Endpoint> List()
        {
            var e = (IMMDeviceEnumerator)new MMDeviceEnumeratorCom();
            var all = new List<Endpoint>();
            foreach (int flow in new[] { Render, Capture })
            {
                string defC = DefaultId(e, flow, RoleConsole), defM = DefaultId(e, flow, RoleComms);
                IMMDeviceCollection col;
                e.EnumAudioEndpoints(flow, Active, out col);
                int n;
                col.GetCount(out n);
                for (int i = 0; i < n; i++)
                {
                    IMMDevice d;
                    col.Item(i, out d);
                    string id;
                    d.GetId(out id);
                    all.Add(new Endpoint
                    {
                        Id = id,
                        Name = FriendlyName(d),
                        Flow = flow == Render ? "render" : "capture",
                        DefaultConsole = id == defC,
                        DefaultComms = id == defM,
                    });
                }
            }
            return all;
        }

        /// Make `id` the default for the console + multimedia roles, and also
        /// communications when `comms`.
        public static void SetDefault(string id, bool console, bool comms)
        {
            var p = (IPolicyConfig)new PolicyConfigCom();
            if (console)
            {
                Marshal.ThrowExceptionForHR(p.SetDefaultEndpoint(id, RoleConsole));
                Marshal.ThrowExceptionForHR(p.SetDefaultEndpoint(id, RoleMultimedia));
            }
            if (comms) Marshal.ThrowExceptionForHR(p.SetDefaultEndpoint(id, RoleComms));
        }

        // ------------------------------------------------------------- mute

        static IAudioEndpointVolume Volume(string id)
        {
            var e = (IMMDeviceEnumerator)new MMDeviceEnumeratorCom();
            IMMDeviceCollection col;
            foreach (int flow in new[] { Render, Capture })
            {
                e.EnumAudioEndpoints(flow, Active, out col);
                int n;
                col.GetCount(out n);
                for (int i = 0; i < n; i++)
                {
                    IMMDevice d;
                    col.Item(i, out d);
                    string did;
                    d.GetId(out did);
                    if (did != id) continue;
                    var iid = typeof(IAudioEndpointVolume).GUID;
                    object o;
                    Marshal.ThrowExceptionForHR(d.Activate(ref iid, 23 /* CLSCTX_ALL */, IntPtr.Zero, out o));
                    return (IAudioEndpointVolume)o;
                }
            }
            throw new ArgumentException("no active endpoint " + id);
        }

        public static bool GetMute(string id)
        {
            bool m;
            Marshal.ThrowExceptionForHR(Volume(id).GetMute(out m));
            return m;
        }

        public static void SetMute(string id, bool mute)
        {
            var ctx = Guid.Empty;
            Marshal.ThrowExceptionForHR(Volume(id).SetMute(mute, ref ctx));
        }

        // ---------------------------------------------------------- waveOut

        [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
        struct WaveOutCaps
        {
            public short wMid, wPid;
            public int vDriverVersion;
            [MarshalAs(UnmanagedType.ByValTStr, SizeConst = 32)] public string szPname;
            public int dwFormats;
            public short wChannels, wReserved1;
            public int dwSupport;
        }

        [StructLayout(LayoutKind.Sequential)]
        struct WaveFormatEx
        {
            public short wFormatTag, nChannels;
            public int nSamplesPerSec, nAvgBytesPerSec;
            public short nBlockAlign, wBitsPerSample, cbSize;
        }

        [StructLayout(LayoutKind.Sequential)]
        struct WaveHdr
        {
            public IntPtr lpData;
            public int dwBufferLength, dwBytesRecorded;
            public IntPtr dwUser;
            public int dwFlags, dwLoops;
            public IntPtr lpNext, reserved;
        }

        [DllImport("winmm.dll")] static extern int waveOutGetNumDevs();
        [DllImport("winmm.dll", CharSet = CharSet.Unicode)] static extern int waveOutGetDevCaps(IntPtr id, ref WaveOutCaps caps, int size);
        [DllImport("winmm.dll")] static extern int waveOutOpen(out IntPtr h, IntPtr id, ref WaveFormatEx fmt, IntPtr cb, IntPtr inst, int flags);
        [DllImport("winmm.dll")] static extern int waveOutPrepareHeader(IntPtr h, IntPtr hdr, int size);
        [DllImport("winmm.dll")] static extern int waveOutUnprepareHeader(IntPtr h, IntPtr hdr, int size);
        [DllImport("winmm.dll")] static extern int waveOutWrite(IntPtr h, IntPtr hdr, int size);
        [DllImport("winmm.dll")] static extern int waveOutClose(IntPtr h);

        public static string[] WaveOutNames()
        {
            int n = waveOutGetNumDevs();
            var names = new string[n];
            for (int i = 0; i < n; i++)
            {
                var c = new WaveOutCaps();
                waveOutGetDevCaps((IntPtr)i, ref c, Marshal.SizeOf(typeof(WaveOutCaps)));
                names[i] = c.szPname;
            }
            return names;
        }

        /// Play a PCM16 WAV file into the first waveOut device whose name
        /// contains `device` (case-insensitive). Blocks until it has played.
        /// Returns the device name used.
        public static string PlayWav(string path, string device)
        {
            int dev = -1;
            var names = WaveOutNames();
            for (int i = 0; i < names.Length; i++)
                if (names[i].IndexOf(device, StringComparison.OrdinalIgnoreCase) >= 0) { dev = i; break; }
            if (dev < 0) throw new Exception("no waveOut device matching '" + device + "': " + string.Join(", ", names));

            byte[] wav = File.ReadAllBytes(path);
            var fmt = new WaveFormatEx();
            int dataOff = -1, dataLen = 0, pos = 12;
            while (pos + 8 <= wav.Length)
            {
                string id = System.Text.Encoding.ASCII.GetString(wav, pos, 4);
                int len = BitConverter.ToInt32(wav, pos + 4);
                if (id == "fmt ")
                {
                    fmt.wFormatTag = BitConverter.ToInt16(wav, pos + 8);
                    fmt.nChannels = BitConverter.ToInt16(wav, pos + 10);
                    fmt.nSamplesPerSec = BitConverter.ToInt32(wav, pos + 12);
                    fmt.nAvgBytesPerSec = BitConverter.ToInt32(wav, pos + 16);
                    fmt.nBlockAlign = BitConverter.ToInt16(wav, pos + 20);
                    fmt.wBitsPerSample = BitConverter.ToInt16(wav, pos + 22);
                }
                else if (id == "data")
                {
                    dataOff = pos + 8;
                    // espeak-ng writes 0x7fffffff when it streams to a file.
                    dataLen = Math.Min(len < 0 ? int.MaxValue : len, wav.Length - dataOff);
                    break;
                }
                pos += 8 + len + (len & 1);
            }
            if (dataOff < 0 || fmt.wFormatTag != 1) throw new Exception("not a PCM WAV: " + path);
            fmt.cbSize = 0;

            IntPtr h;
            int r = waveOutOpen(out h, (IntPtr)dev, ref fmt, IntPtr.Zero, IntPtr.Zero, 0);
            if (r != 0) throw new Exception("waveOutOpen failed: " + r);
            IntPtr data = Marshal.AllocHGlobal(dataLen);
            IntPtr hdr = Marshal.AllocHGlobal(Marshal.SizeOf(typeof(WaveHdr)));
            try
            {
                Marshal.Copy(wav, dataOff, data, dataLen);
                var wh = new WaveHdr { lpData = data, dwBufferLength = dataLen };
                Marshal.StructureToPtr(wh, hdr, false);
                int sz = Marshal.SizeOf(typeof(WaveHdr));
                waveOutPrepareHeader(h, hdr, sz);
                waveOutWrite(h, hdr, sz);
                // WHDR_DONE = 1
                var deadline = DateTime.UtcNow.AddMilliseconds(dataLen * 1000.0 / fmt.nAvgBytesPerSec + 5000);
                while ((((WaveHdr)Marshal.PtrToStructure(hdr, typeof(WaveHdr))).dwFlags & 1) == 0 && DateTime.UtcNow < deadline)
                    Thread.Sleep(10);
                waveOutUnprepareHeader(h, hdr, sz);
            }
            finally
            {
                waveOutClose(h);
                Marshal.FreeHGlobal(data);
                Marshal.FreeHGlobal(hdr);
            }
            return names[dev];
        }
    }
}
