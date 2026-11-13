# Read-only probe: does the running app's process environment hold
# WEBVIEW2_* variables? Prints only WEBVIEW2_* names/values.
param([string]$Name = 'murmur-windows')
Add-Type @'
using System;
using System.Runtime.InteropServices;
public static class ProcEnv {
  [DllImport("ntdll.dll")] static extern int NtQueryInformationProcess(IntPtr h, int cls, ref PBI pbi, int len, out int ret);
  [DllImport("kernel32.dll")] static extern IntPtr OpenProcess(int access, bool inherit, int pid);
  [DllImport("kernel32.dll")] static extern bool ReadProcessMemory(IntPtr h, IntPtr addr, byte[] buf, IntPtr size, out IntPtr read);
  [StructLayout(LayoutKind.Sequential)] struct PBI { public IntPtr r1, Peb, r2a, r2b, Pid, r3; }
  static IntPtr ReadPtr(IntPtr h, IntPtr a) { var b = new byte[8]; IntPtr n; ReadProcessMemory(h, a, b, (IntPtr)8, out n); return (IntPtr)BitConverter.ToInt64(b, 0); }
  public static string Get(int pid) {
    IntPtr h = OpenProcess(0x0410, false, pid);
    var pbi = new PBI(); int ret;
    NtQueryInformationProcess(h, 0, ref pbi, Marshal.SizeOf(pbi), out ret);
    IntPtr pp = ReadPtr(h, pbi.Peb + 0x20);
    IntPtr env = ReadPtr(h, pp + 0x80);
    var sz = new byte[8]; IntPtr n; ReadProcessMemory(h, pp + 0x3F0, sz, (IntPtr)8, out n);
    long size = BitConverter.ToInt64(sz, 0);
    var buf = new byte[size]; ReadProcessMemory(h, env, buf, (IntPtr)size, out n);
    return System.Text.Encoding.Unicode.GetString(buf);
  }
}
'@
foreach ($p in Get-Process $Name -EA SilentlyContinue) {
    "pid=$($p.Id)"
    ([ProcEnv]::Get($p.Id) -split "`0") | Where-Object { $_ -like 'WEBVIEW2*' }
}
