# Call transcription feasibility (2026-11-02)

Owner tasks: "Transcribe phone calls" and its subtask "Reverse Android's call
transcription feature". This is research only, with no product code.

Target: the owner's Galaxy S25+ (SM-S936B), One UI 9 / Android 16, not rooted.
Our app (`dev.murmur`) is sideloaded and already has an enabled
accessibility service, a `microphone|specialUse` foreground service, and
on-device Hebrew + English STT.

Trust labels:
- **[AOSP]**: read from AOSP `main` source on 2026-11-02. Samsung may patch it.
- **[DOC]**: from an official Android, Samsung or Google page.
- **[PROJ]**: from a project's own repo or site.
- **[LOW]**: XDA, blogs, forums or vendor marketing.
- **UNVERIFIED**: nothing better than inference. The device-check list (§7)
  is there to settle these points.

## 1. Executive summary

1. Real call audio (the `VOICE_CALL` / `VOICE_UPLINK` / `VOICE_DOWNLINK` sources) needs `CAPTURE_AUDIO_OUTPUT` (`signature|privileged|role`). Samsung's Phone app and Pixel's Call Notes get it by being system apps or system-only role holders. A sideloaded app cannot get it. Neither the default-dialer role nor `InCallService` grants it [AOSP].
2. The AOSP shell user (uid 2000) **does** hold `CAPTURE_AUDIO_OUTPUT` [AOSP]. Through Shizuku, ShizuCallRecorder records both sides as stereo on Android 12-17 without root [PROJ]. This works only if Samsung kept that grant (UNVERIFIED), and Shizuku has to be re-armed after every reboot.
3. Our accessibility service, while it runs a foreground service, counts as "a11y on top" in AOSP's capture policy. That lets it capture the mic (`VOICE_RECOGNITION`) during a call [AOSP]. It only reliably gets the owner's side, plus the other side if speakerphone is on (UNVERIFIED on Samsung).
4. One UI's own call recorder (offered in Israel, and since One UI 7 with Galaxy AI "Transcript assist") saves both sides to shared storage. That is most likely `Recordings/Call/*.m4a` [LOW, UNVERIFIED]. Our app can read it through MediaStore (`READ_MEDIA_AUDIO`), SAF or All files access.
5. Samsung's transcript text lives inside Samsung's Phone and Voice Recorder apps. There is no documented API for it. The only ways out are the manual share or "Add to Samsung Notes" flows [DOC]. Whether the text is embedded in the m4a is UNVERIFIED.
6. **Recommendation:** watch Samsung's call-recording folder and transcribe new files offline with our STT. This gets both sides, needs no root, no new privileges and no audio hacks, and gives us Hebrew where Galaxy AI may not have it. Keep Shizuku as an optional "power" path and the a11y mic as a last-resort, one-sided fallback.

## 2. How the OEM features capture call audio (Q1)

### 2.1 Platform rules

- **`MediaRecorder.AudioSource`**
  - `VOICE_UPLINK` (2), `VOICE_DOWNLINK` (3) and `VOICE_CALL` (4) are the call-audio sources. They require `CAPTURE_AUDIO_OUTPUT`.
  - `VOICE_RECOGNITION`, `VOICE_COMMUNICATION` and `MIC` are microphone sources and need only `RECORD_AUDIO`.
  - [DOC: developer.android.com AudioSource reference. The fetch was truncated, so the constants are from the reference plus AOSP.]
- **Protection levels** in `frameworks/base/core/res/AndroidManifest.xml` [AOSP]:
  - `CAPTURE_AUDIO_OUTPUT`, `CAPTURE_VOICE_COMMUNICATION_OUTPUT` and `CONTROL_INCALL_EXPERIENCE` are `signature|privileged|role`.
  - `CALL_AUDIO_INTERCEPTION` is `signature|privileged|role`.
- **Roles that grant these** (`PermissionController/res/xml/roles.xml`) [AOSP]:
  - `android.app.role.SYSTEM_AUDIO_INTELLIGENCE` grants `CAPTURE_AUDIO_OUTPUT`, `CAPTURE_VOICE_COMMUNICATION_OUTPUT`, `CONTROL_INCALL_EXPERIENCE`, `READ_CALL_LOG` and more.
  - `android.app.role.SYSTEM_AMBIENT_AUDIO_INTELLIGENCE` grants `CAPTURE_AUDIO_OUTPUT` and related permissions.
  - `android.app.role.SYSTEM_CALL_STREAMING` grants `CALL_AUDIO_INTERCEPTION`.
  - All three roles are `systemOnly="true"`, so they are out of reach for us.
  - `android.app.role.DIALER` grants only the phone, contacts, sms, microphone, camera and notifications permission sets plus `SYSTEM_ALERT_WINDOW`. It does **not** grant `CAPTURE_AUDIO_OUTPUT`.
- **Concurrent-capture policy** (`frameworks/av/.../AudioPolicyService.cpp`, `updateUidStates_l`) [AOSP]:
  - During `MODE_IN_CALL`, an ordinary client is silenced unless `canBypassConcurrentPolicy` is set. That flag comes from `CAPTURE_AUDIO_OUTPUT` or `CONCURRENT_AUDIO_RECORD_BYPASS`.
  - Virtual sources (`VOICE_CALL`/`UPLINK`/`DOWNLINK`, `REMOTE_SUBMIX`) are always allowed once a client may open them.
  - There is a special case for accessibility services. If the a11y uid is "on top", meaning its process state is between `PROCESS_STATE_TOP` and `PROCESS_STATE_BOUND_FOREGROUND_SERVICE` (a foreground service counts), its `VOICE_RECOGNITION` or `HOTWORD` capture is allowed **even in a call**.
  - The Android docs' "Sharing audio input" page says the same: during a call "the app can capture audio if it is an accessibility service". A privileged app with `CAPTURE_AUDIO_OUTPUT` can capture the call itself through `VOICE_UPLINK`/`VOICE_DOWNLINK` or `AudioDeviceInfo.TYPE_TELEPHONY` [DOC].
- **History:**
  - Android 9 blocked call-audio capture for third-party apps.
  - Android 10 added the concurrent-capture policy above.
  - In 2022, Play policy banned using accessibility for call recording [PROJ: Cube ACR update page]. That is a Play policy, not a platform block, so it does not bind a sideloaded app.

### 2.2 Samsung (Phone app call recording, Transcript assist, Live Translate, Call assist)

- Recording lives in the Samsung Phone app under Phone > Settings > Record calls (auto-record, recorded calls list). Recordings also show up in Samsung Voice Recorder [DOC: samsung.com/us ANS10004613].
- One UI 7+ adds Transcript assist. It can auto-transcribe recorded calls, summarize them, and use a "Process data only on device" switch. Language packs are downloaded and about 20 languages are listed [DOC].
  - Hebrew is **not** on the language lists we saw (UNVERIFIED).
  - The other party hears an audio alert when recording starts and stops, at least on the US build [DOC]. Israeli builds historically had no announcement [LOW, UNVERIFIED for One UI 9].
- Live Translate and Call assist run inside the Samsung Phone / InCallUI system apps, and Samsung's dialer is a preinstalled privileged app. How they capture audio is UNVERIFIED (Samsung publishes nothing). The likely route is `CAPTURE_AUDIO_OUTPUT` plus `VOICE_CALL` / `VOICE_DOWNLINK`, or a Samsung-private HAL path. §7 checks the permission grant.

### 2.3 Pixel (Call Notes, Call Screen, call recording)

- Call Notes transcribes and summarizes the call with on-device Gemini Nano, and the participants are notified [DOC: support.google.com/phoneapp/answer/15257579; device range from LOW sources].
- The mechanism is UNVERIFIED. The obvious fit is that the Phone by Google app (privileged) or the `SYSTEM_AUDIO_INTELLIGENCE` role holder captures through `CAPTURE_AUDIO_OUTPUT`. That role exists precisely to give on-device audio AI call access.
- Call Screen uses the public `CallScreeningService` only to decide whether to screen a call. The live audio and TTS pipeline is Google's privileged code. `CallScreeningService` gives a third-party app **no** audio.

## 3. What a sideloaded, non-root app can do (Q2)

### 3.1 Accessibility service + mic (what our app already is)

- Our a11y uid is in `mA11yUids`. While our foreground service runs, AOSP counts us as "a11y on top", so `VOICE_RECOGNITION` capture during a cellular call is **not** silenced [AOSP].
- What the mic hears:
  - **Earpiece:** the owner's voice, and the far end only through leakage. Expect it to be faint or absent.
  - **Speakerphone:** both sides, but the call's AEC/NS may duck the far end in our stream. That is HAL-dependent (UNVERIFIED on S25+).
- This is the method Cube ACR's "app connector" used: callers on Android 9+, the other party on Android 10+, and VoIP [PROJ: cubeacr.app/helper.html, 2023]. Samsung users report mixed results [LOW]. Cube no longer gets this path on Play but offers a sideload APK [PROJ].
- Android 13+ "restricted settings" for sideloaded a11y services are already handled by our install flow.

### 3.2 Default dialer (InCallService as the default phone app)

- The DIALER role adds no capture permission (§2.1), and `canBypassConcurrentPolicy` stays false. Being the dialer only gives call events and UI.
- It is a very large amount of work (a full dialer UI) and gets no audio. **Rejected.**
- A non-UI `InCallService` just for call events is possible without being the dialer, using the `MANAGE_ONGOING_CALLS` appop granted through shell. That is the trick ShizuCallRecorder uses [PROJ].

### 3.3 Shizuku / ADB-granted

- `pm grant` cannot grant `CAPTURE_AUDIO_OUTPUT`, because it is not a `development` permission. An appop on `RECORD_AUDIO` does not unlock `VOICE_CALL` either: the check is the permission, not the op.
- The AOSP Shell package does declare `CAPTURE_AUDIO_OUTPUT`, `CALL_AUDIO_INTERCEPTION` and `CONTROL_INCALL_EXPERIENCE` (for GTS/CTS tests) [AOSP: `frameworks/base/packages/Shell/AndroidManifest.xml`]. Code that runs as uid 2000 through a Shizuku UserService, or through an `app_process` launched over ADB, can open `AudioRecord(VOICE_CALL / VOICE_UPLINK / VOICE_DOWNLINK)`.
- ShizuCallRecorder does exactly this [PROJ: github.com/kitsumed/ShizuCallRecorder, source read 2026-11-02]:
  - It uses `RecordingPlanResolver` with `VOICE_CALL` by default and can split uplink and downlink into stereo.
  - `AudioRecordCapture` runs as shell uid 2000, built by reflection.
  - It uses a non-UI `InCallService` through the `MANAGE_ONGOING_CALLS` appop.
  - It supports Android 12-17, cellular first, and VoIP best-effort.
  - It warns about OEM breakage, and its code notes that Samsung has broken `InCallService` callbacks in the past.
- **Both sides: yes.** On Samsung it is **UNVERIFIED**, because Samsung could strip the shell grant or the HAL could return silence.
- Cost: wireless debugging has to be re-paired, or Shizuku restarted, after every reboot.

### 3.4 Other

- **Speakerphone forced through accessibility plus mic:** this is §3.1 with better far-end capture. It is intrusive for the user.
- **`REMOTE_SUBMIX` / MediaProjection playback capture:** cellular call audio is not a playback stream that apps can capture. No.

### 3.5 Legal and consent

- In Israel, under the Wiretapping Law 5739-1979 as commonly read, a participant may record their own call without the other side's consent (one-party consent). This is **not legal advice.**
- Other jurisdictions differ. Samsung's US build announces recording to both parties [DOC], and ShizuCallRecorder explicitly leaves consent to the user [PROJ].
- If we ship this, show a first-run notice and keep transcripts on the device, as our STT already does.

## 4. Options table

| Option | Both sides? | Root/ADB? | Reliability on Samsung S25+ | Effort for us | Privacy |
|---|---|---|---|---|---|
| **A. Watch Samsung's call-recording folder, transcribe offline** | Yes (Samsung records the line) | No | High if region/CSC exposes recording (Israel: yes historically) and the folder is indexed. Path UNVERIFIED | Low: MediaStore observer + queue + existing STT | Audio stays on device; we never touch telephony |
| B. Shizuku shell-uid `VOICE_CALL` (ShizuCallRecorder-style) | Yes, even split stereo | ADB once per boot (Shizuku) | UNVERIFIED: depends on Samsung keeping shell's `CAPTURE_AUDIO_OUTPUT` and HAL support | Medium-high: Shizuku UserService, hidden APIs, call detection | On device; Shizuku is a broad privilege |
| C. Our a11y service + mic (`VOICE_RECOGNITION`) | Owner side only; far side only on speakerphone, maybe ducked | No | Medium for owner side; poor for far side (UNVERIFIED) | Medium: call detection, in-call capture, speaker toggle | On device |
| D. Default dialer app | No (no capture right) | No | n/a | Very high | n/a |
| E. Root / BCR / system app | Yes | Root (BCR is a Magisk module; stock unrooted not supported) [PROJ] | n/a, owner is not rooted | n/a | n/a |
| F. Read Samsung's own transcript | Yes (text) | No | Low: private app data, no API; manual share only | Low (manual) / hacky (a11y scraping) | Galaxy AI may use cloud unless "on device only" is set |

## 5. Recommendation and the cheapest path (Q3, Q4)

**Option A first.**

- **Owner setup:** Phone > Settings > Record calls > Auto record (all calls or selected numbers). This is Samsung's own feature.
- **Our side:** a "Call transcripts" feature.
  1. Read the recordings through `MediaStore.Audio` with `READ_MEDIA_AUDIO` (API 33+). Filter on `RELATIVE_PATH LIKE 'Recordings/Call%'` (to be confirmed in §7) and use a `ContentObserver` on the audio collection. `FileObserver` on FUSE-backed shared storage is unreliable for other apps' writes.
  2. If the folder holds a `.nomedia` or is not indexed, fall back as follows:
     - SAF `ACTION_OPEN_DOCUMENT_TREE` on that folder, with a persisted URI permission. That folder is not in SAF's blocked list (storage root, `Download/` root, `Android/data`, `Android/obb`).
     - Otherwise `MANAGE_EXTERNAL_STORAGE`, which is fine for a sideloaded app.
  3. Decode the m4a (AAC) to 16 kHz PCM with `MediaExtractor`/`MediaCodec`, run our offline STT (Hebrew/English), and save the transcript next to Notes/Scratchpad.
  4. Optional: cleanup or summarization through the existing cleanup routes.
- **Why A:**
  - It is the only path that gets both sides with zero privilege risk.
  - It survives reboots, needs no Shizuku and doesn't rely on audio-policy quirks.
  - It runs entirely offline on audio Samsung already captures.
  - It gives us Hebrew, which Transcript assist probably lacks.
- **Trade-off:** we rely on Samsung keeping call recording in the Israeli CSC, and the result is post-call only, not live.
- **Next step:** B (Shizuku) is an opt-in "advanced" mode, worth trying only if §7 shows shell still holds `CAPTURE_AUDIO_OUTPUT`. It would also enable live, in-call transcription.
- **Last resort:** C (a11y mic) is a one-sided fallback for VoIP or for regions without Samsung recording.
- **Q4 (Samsung transcript):** treat it as inaccessible.
  - The transcript stays in the Voice Recorder / Phone apps, and the documented exports are share and "Add to Samsung Notes" [DOC].
  - No exported content provider is known (UNVERIFIED, see §7).
  - Our accessibility service *could* scrape the transcript screen, but that is fragile and not recommended.
  - If §7 shows the transcript embedded in the m4a (for example a custom atom, as Voice Recorder STT files may have), we could parse it for free (UNVERIFIED).

## 6. Open questions for the owner

1. Does your Phone app show **Settings > Record calls** today on One UI 9? Is auto-record on, and does it announce to the other party?
2. Is post-call transcription enough, or do you need live, in-call text? Live text forces B or C.
3. Do you want only cellular calls, or WhatsApp/VoIP calls too? Samsung's recorder does not cover VoIP, so VoIP needs C or B.
4. Are you willing to run Shizuku (re-armed after each reboot through wireless debugging) for an advanced mode?
5. Which calls should be transcribed: all, contacts only, or chosen numbers? Should audio be deleted after transcription?
6. Is Galaxy AI Transcript assist on, and does it already give acceptable Hebrew? If so, you might only want the m4a → our-STT path for Hebrew calls.
7. Are any of your call partners in other jurisdictions (two-party consent)? If so, should we show a reminder?

## 7. Device-check list (run these for me)

Run each with the phone connected. `<ADB>` is the debug-bridge binary. Steps
that pull files are marked; files are small (about 1 MB per minute of m4a), so
pull one at most.

**Build and region:**
```
<ADB> shell getprop ro.product.model
<ADB> shell getprop ro.build.version.release
<ADB> shell getprop ro.build.version.oneui
<ADB> shell getprop ro.csc.sales_code
<ADB> shell getprop ro.csc.country_code
```

**Who holds the call-audio permissions:**
```
<ADB> shell dumpsys package com.samsung.android.dialer | grep -E "versionName|CAPTURE_AUDIO_OUTPUT|CALL_AUDIO_INTERCEPTION|CONTROL_INCALL_EXPERIENCE"
<ADB> shell dumpsys package com.samsung.android.incallui | grep -E "versionName|CAPTURE_AUDIO_OUTPUT|CALL_AUDIO_INTERCEPTION"
<ADB> shell dumpsys package com.sec.android.app.voicenote | grep -E "versionName|CAPTURE_AUDIO_OUTPUT"
<ADB> shell cmd role get-role-holders android.app.role.DIALER
<ADB> shell cmd role get-role-holders android.app.role.SYSTEM_AUDIO_INTELLIGENCE
<ADB> shell cmd role get-role-holders android.app.role.SYSTEM_CALL_STREAMING
```

**Option B viability (does Samsung's shell keep the AOSP grants?):**
```
<ADB> shell dumpsys package com.android.shell | grep -E "CAPTURE_AUDIO_OUTPUT|CALL_AUDIO_INTERCEPTION|CONTROL_INCALL_EXPERIENCE|MANAGE_ONGOING_CALLS"
```

**Option A: the recording folder and format:**
```
<ADB> shell settings list system | grep -iE "record|transcri"
<ADB> shell settings list secure | grep -iE "record|transcri"
<ADB> shell settings list global | grep -iE "record|transcri"
<ADB> shell find /sdcard -maxdepth 3 -type d -iname "*call*"
<ADB> shell ls -la /sdcard/Recordings /sdcard/Recordings/Call /sdcard/Call /sdcard/Sounds
<ADB> shell ls -la /sdcard/Recordings/Call/.nomedia
<ADB> shell content query --uri content://media/external/audio/media --projection _display_name:relative_path:owner_package_name:mime_type:duration:is_pending --where "relative_path LIKE '%Call%'"
```

After one short test call with auto-record on, run these. Replace `<FILE>` with a name from `ls`.
```
<ADB> shell ls -la --full-time /sdcard/Recordings/Call
<ADB> shell stat "/sdcard/Recordings/Call/<FILE>"
<ADB> shell "head -c 48 '/sdcard/Recordings/Call/<FILE>' | od -c"
<ADB> shell "grep -c -a -iE 'stt|transcri' '/sdcard/Recordings/Call/<FILE>'"
```
Optional, PULL (one file): `<ADB> pull "/sdcard/Recordings/Call/<FILE>" ./call-sample.m4a`, then locally `ffprobe -show_format -show_streams call-sample.m4a`. This shows the codec, sample rate, channels and any embedded transcript atom. **Don't commit it.**

**Q4: is the transcript exposed?**
```
<ADB> shell dumpsys package providers | grep -iE "voicenote|dialer|incallui|transcri"
<ADB> shell dumpsys package com.sec.android.app.voicenote | grep -iE "Provider|authority|exported"
```

**Option C: does our a11y + foreground service survive in-call capture?** Start a call, then start dictation from our bubble during the call.
```
<ADB> shell dumpsys media.audio_policy | grep -iE "Accessibility UIDs|A11y" -A3
<ADB> shell pm list packages -U dev.murmur
<ADB> shell dumpsys audio | grep -iE "mode|RecordActivityMonitor|silenced" | head -40
<ADB> shell dumpsys media.audio_policy | grep -iE "AudioRecordClient|silenced|source" | head -40
<ADB> shell dumpsys activity processes dev.murmur | grep -iE "procState|curProcState"
<ADB> shell appops get dev.murmur RECORD_AUDIO
```
Expected: our uid appears in "Accessibility UIDs", our client is not silenced,
and our process state is TOP or FGS.

## Sources

**AOSP**, read 2026-11-02 from `android.googlesource.com` `main`:
- `frameworks/av/services/audiopolicy/service/AudioPolicyService.cpp` (`updateUidStates_l`, `isA11yOnTop`, `isVirtualSource`).
- `frameworks/av/services/audiopolicy/service/AudioPolicyInterfaceImpl.cpp` (`canBypassConcurrentPolicy`).
- `frameworks/base/core/res/AndroidManifest.xml` (protection levels).
- `frameworks/base/packages/Shell/AndroidManifest.xml` (shell's `CAPTURE_AUDIO_OUTPUT`, `CALL_AUDIO_INTERCEPTION`, `CONTROL_INCALL_EXPERIENCE`).
- `packages/modules/Permission/PermissionController/res/xml/roles.xml` (DIALER, SYSTEM_AUDIO_INTELLIGENCE, SYSTEM_AMBIENT_AUDIO_INTELLIGENCE, SYSTEM_CALL_STREAMING).

**Android docs:**
- https://developer.android.com/media/platform/sharing-audio-input
- https://developer.android.com/reference/android/media/MediaRecorder.AudioSource

**Samsung:**
- https://www.samsung.com/us/support/answer/ANS10004613
- https://www.samsung.com/in/support/mobile-devices/how-to-use-call-transcript-feature-in-galaxy-devices/
- https://www.samsung.com/us/support/answer/ANS10000942/

**Google:**
- https://support.google.com/phoneapp/answer/15257579

**Projects:**
- https://github.com/kitsumed/ShizuCallRecorder (source read)
- https://github.com/chenxiaolong/BCR
- https://cubeacr.app/latest-update.html
- https://cubeacr.app/helper.html

**Lower trust [LOW]:**
- https://9to5google.com/2024/12/10/samsung-galaxy-call-recording-one-ui-7-update/
- https://www.xda-developers.com/how-to-record-calls-android/
- Samsung Community threads on Cube ACR.
- Blog posts giving `Recordings/Call`: these are the only sources for the folder path, so it is UNVERIFIED.
