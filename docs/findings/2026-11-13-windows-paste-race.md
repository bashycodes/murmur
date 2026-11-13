# Windows: paste reports success but nothing lands (2026-11-13)

Ticket "Windows · paste reports success but nothing lands (intermittent)".
Decision: D93. Probe: `tools/probes/prototypes/paste_race_2026_11_13/`.

## Symptom

On 1.3.0 the e2e harness (scenario c, the WinForms text pad) hit it 2 times
in about 6 dictations. The debug log said `insert: inserted (ctrl_v)` with
the right text, the pad was in the foreground at both checks, and it stayed
empty. A Hebrew clip had shown the same before. The failing run's results
folder was in a worktree that has since been removed, so its debug log is
gone. Only the harness console log is left: `c: FAIL 1/2 clips`.

## Why 1.3.0 says "inserted"

`paste::win::paste` set the clipboard, sent `[Ctrl↓,V↓] … [V↑,Ctrl↑]`,
and returned the `SendInput` count. That count only says the four events
were queued. Nothing checked that the target read the clipboard.

## Root cause (code path, and the laptop's facts)

The pad's TextBox is a native EDIT control. Its WM_PASTE calls
`OpenClipboard` **once**. If that fails, it pastes nothing and reports
nothing. Our Ctrl+V reaches the pad a few ms after our `CloseClipboard`.
That write is also when every clipboard format listener gets
WM_CLIPBOARDUPDATE and opens the clipboard to look at the formats. They
open it even when our opt-out formats (S14) tell them not to keep the text,
because they have to open it to see those formats. On the laptop:

- clipboard history is on (`HKCU\Software\Microsoft\Clipboard`
  `EnableClipboardHistory = 1`, service `cbdhsvc_*`)
- Phone Link runs (`PhoneExperienceHost.exe`, clipboard sync)
- PowerToys runs (Advanced Paste watches the clipboard)

So three listeners race the target's one `OpenClipboard`. The loss is
intermittent and has nothing to do with the language, which matches the
Hebrew clip.

The other suspects don't fit:

- **Restore too early.** The restore comes 500 ms after the chord, and the
  pad reads within ms. Restoring early would also paste the owner's old
  clipboard, not leave the pad empty.
- **Modifiers still down.** `release_before_paste` checks Alt and Win, and
  the harness's Right Alt is released seconds earlier.
- **Focus.** The harness checked the foreground both times.

## Evidence still to collect

Collecting it needs the laptop unlocked. On a locked desktop `OpenClipboard`
fails with ERROR_ACCESS_DENIED (5) for every process in the session, the
probe included. The harness also aborts every try there (`fg=Idle`).

1. **Probe.** The probe works with no keyboard input. For each trial it
   writes the clipboard the way 1.3.0 does, waits 0–100 ms, sends WM_PASTE
   to a real EDIT, and logs which process held the clipboard and when.
   Expected result: misses at 0–5 ms that line up with a listener process
   holding the clipboard, and none at 100 ms.

   ```
   scp ClipRace.cs clip-race.ps1 ../../../e2e-windows/ps/itask.ps1 <laptop>:murmur-e2e/paste-race/
   ssh <laptop> "cd murmur-e2e\paste-race && powershell -File itask.ps1 -Script clip-race.ps1 -Arguments \"-Out race.jsonl\" -Wait -TimeoutSec 600"
   ```

2. **Harness.** Run scenario c 10× on 1.3.0 (expect misses), then 10× on
   the CI installer of `feat/paste-confirm`. The target is at most 1 miss
   in 20 clips. A miss now has to show up as `paste_failed` with
   `NOT_CONFIRMED_MSG`, never as `inserted`.
