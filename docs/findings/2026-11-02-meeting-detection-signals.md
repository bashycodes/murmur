# Meeting detection signals on Windows (Zoom, Webex, Jabber), 2026-11-02

> **Status:** desk research for D48 (`apps/murmur-windows/src/meetings/detect.rs`). Built and
> unit-tested on synthetic fixtures (`apps/murmur-windows/tests/fixtures/meeting-detect/`).
> **Not yet checked on a real PC**: the Zoom window classes below come from community scripts,
> not Zoom docs, and no Webex/Jabber session list has been captured. The debug log line
> `meeting-detect: signals: …` (Developer → Debug log) prints what the probe sees, so the first
> real call confirms or corrects this table.

## What "in a call" looks like

A meeting client sits in the tray all day, so a running process never means a call by itself.
The probe uses three kinds of signal, cheapest first:

| Signal | Zoom | Webex | Jabber |
|---|---|---|---|
| Process (gate: the client is open) | `Zoom.exe` | `CiscoCollabHost.exe` (Webex App, several instances), `webexhost.exe` (joins meetings), classic Webex Meetings `atmgr.exe`, `webexmta.exe` | `CiscoJabber.exe` |
| Meeting-only window (visible, owned by the client) | `ZPContentViewWndClass` (meeting window, title "Zoom Meeting"), `ZPFloatToolbarClass` (floating toolbar while sharing) | none known | none known |
| Meeting-only helper process | `CptHost.exe` (screen-sharing host) while `Zoom.exe` runs | — | — |
| Audio session (WASAPI, per process) | capture session **active** = mic in use; render session active = sound playing | same | same |

Sources:

- Zoom window classes: community scripts that detect a running Zoom meeting by these two
  classes, e.g. [statik's gist](https://gist.github.com/statik/0c16fcc216a55ae556640ef5c17fd9d8)
  (`ZPContentViewWndClass` and `ZPFloatToolbarClass`; a UIA dump shows the meeting window as
  `ClassName: ZPContentViewWndClass … Name: Zoom Meeting`) and
  [ZoomStatus](https://github.com/oldjohngalt/ZoomStatus), which warns that the docked control
  window used while sharing can defeat class-based detection. Zoom does not document these
  classes; they can change in any release. The Meeting SDK's `GetMeetingUIWnd()` is the official
  route but needs the SDK inside the process
  ([Zoom dev forum](https://devforum.zoom.us/t/how-can-i-use-public-sdkerror-getmeetinguiwnd-valuetype-hfirstview-valuetype-hsecondview-to-get-the-window-handle-of-meeting-ui/486)).
- `CptHost.exe`: Zoom's sharing component, needed for screen sharing
  ([file.net](https://www.file.net/process/cpthost.exe.html); Zoom community threads about
  sharing errors name it, e.g.
  [this one](https://community.zoom.com:443/t5/Zoom-Meetings/zoom-cpthost-exe-is-using-exchcsp-dll-from-an-unknown-publisher/m-p/19167)).
- Webex executables: Cisco's
  [Executable files used by Webex App on Windows](https://help.webex.com/en-us/article/dluqwo/Executable-files-used-by-Webex-App-on-Windows):
  `CiscoCollabHost.exe` is the main host and runs several instances (UI, media, calling, MAPI);
  `CiscoWebExStart.exe` and `webexhost.exe` join meetings; `washost.exe` shares content.
  Classic Webex Meetings: `atmgr.exe` / `webexmta.exe` start with a meeting
  ([Nexthink library](https://docs-v6.nexthink.com/library/webex-operate),
  [Webex help WBX79434](https://help.webex.com/en-us/article/WBX79434)); they can linger after
  it ends ([WebAxe](https://github.com/Elestriel/WebAxe) exists to kill them), so they are not
  used as a call signal.
- Jabber: `CiscoJabber.exe` is the Jabber for Windows client exe (install guides, e.g.
  [this one](https://tarleton.edu/technology/jabber-for-windows)); no meeting-only window or
  helper is documented, so a Jabber call = Jabber's microphone stream.
- Audio sessions: `IAudioSessionManager2::GetSessionEnumerator` on each endpoint lists the
  sessions; `IAudioSessionControl2::GetProcessId` names the process and
  `IAudioSessionControl::GetState` = `AudioSessionStateActive` while a stream runs
  ([IAudioSessionEnumerator](https://learn.microsoft.com/en-us/windows/win32/api/audiopolicy/nn-audiopolicy-iaudiosessionenumerator),
  [GetSessionEnumerator](https://learn.microsoft.com/en-us/windows/win32/api/audiopolicy/nf-audiopolicy-iaudiosessionmanager2-getsessionenumerator);
  a [Microsoft Q&A answer](https://learn.microsoft.com/en-us/answers/questions/214055/is-it-possible-to-get-status-or-event-when-device)
  suggests the same: enumerate capture endpoints, check which sessions are active).
- Alternative not used: the registry
  `HKCU\Software\Microsoft\Windows\CurrentVersion\CapabilityAccessManager\ConsentStore\microphone`
  (`NonPackaged\<exe path>` keys; `LastUsedTimeStop == 0` = the app uses the mic now) is even
  cheaper and covers every device, but is undocumented and Teams-style packaged apps sit
  elsewhere in it ([Velociraptor artifact](https://docs.velociraptor.app/exchange/artifacts/pages/windows.registry.capabilityaccessmanager),
  [automattermostatus issue](https://gitlab.com/matclab/automattermostatus/-/issues/17)). It is
  the fallback if WASAPI sessions turn out unreliable on the laptop.

## How the detector uses them

- **Start** only on a call signal, never on "open": a meeting window or `CptHost.exe` for 2
  looks in a row (~3-6 s), or the client's microphone stream for 4 looks (~9-12 s; Zoom's
  "Test microphone" in Settings also opens the mic, and is normally shorter).
- **Keep going** while any signal of that client remains, including an active render session
  (a muted participant still hears the call). Muting in Zoom/Webex is expected to keep the
  capture stream open (Windows keeps showing the mic-in-use icon), but the detector does not
  rely on it.
- **End** after 4 looks (~12 s) with nothing: the meeting window gone and no active stream.
- **Cost**: one look every 3 s while the setting is on. With no client running a look is one
  `CreateToolhelp32Snapshot` walk (~200 processes). With a client running it adds one
  `EnumWindows` pass (class names of the client's windows only, never titles) and one session
  enumeration per active audio endpoint. With the setting off (default) nothing runs.
- **Privacy**: window titles are not read (a Zoom title can carry a meeting topic); the debug
  log line lists exe names, window classes and session states only.

## Open questions for the first real check

1. Does the current Zoom Workplace meeting window still use `ZPContentViewWndClass`? If not, the
   mic path still starts the recording (≈ 9-12 s later); add the new class to `PROFILES`.
2. Does Zoom keep its capture session active while muted and its render session active in a
   silent meeting? (Affects only the end delay, not the start.)
3. Which Webex App process owns the call's audio sessions (`CiscoCollabHost.exe` media
   instance or `webexhost.exe`)? Both are listed.
4. Jabber: which process owns a softphone call's mic stream (`CiscoJabber.exe` assumed)?
