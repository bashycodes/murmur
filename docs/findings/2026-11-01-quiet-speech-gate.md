# 2026-11-01: quiet speech dropped as `no_speech` / `blank_asr`

**Question.** On the phone bench of 2026-10-31 (run `r3`), 15 of the 49 FLEURS he_il clips
came back EMPTY on both Hebrew models: 12 `no_speech` and 3 `blank_asr`. Was that the
dataset or the gate?

**Answer: both, and the gate is the bug.**

* **Dataset.** The 15 clips really are that quiet in the original FLEURS float32 files
  (`dev.tar.gz`), not because of our PCM16 conversion. Clip RMS is 20–49 (PCM16) for the 12
  `no_speech` clips and 75–92 for the 3 `blank_asr` clips, so about −64 to −51 dBFS. Most
  clips are around 5000.
* **But they hold clear speech.** Per 20 ms frame, the 95th-percentile RMS sits 11–65× (21–36
  dB) above the 10th-percentile floor, which is often under 1 LSB. Speech at this level
  would also come from a soft voice or a low-gain microphone.
* **Two absolute gates dropped it.**
  1. `no_speech`: `flow_core::vad` never counted a frame under `ABS_MIN = 0.004`
     (≈ −48 dBFS, 131 PCM16), whatever the floor.
  2. `blank_asr`: the VAD passed the 3 louder clips, but only on their loudest syllables
     (180–720 ms of speech). The sherpa engines then got the trimmed span at an RMS of
     82–96 and returned "" through their own whole-clip `SILENCE_RMS = 0.003` (98 PCM16).
     `flow_core::whisper` has the same gate.
* **Every platform shares both gates.** Android (FFI), Windows and flow-server all run
  `flow_core::engine` with `SessionConfig::vad = true`, and the engines are the same
  crates. The fix is in `flow-core`, so it covers all three. The Windows GPU whisper plugin
  (`flow-whisper`) has no RMS gate of its own.

## Change (`crates/flow-core/src/vad.rs`, `engine.rs`)

1. **A relative bar for quiet recordings.**
   * When a clip's noise floor is very low, `ABS_MIN` drops to `REL_FACTOR` (8×, 18 dB) ×
     the floor, but never below `QUIET_MIN = 0.0005` (≈ −66 dBFS, 16 PCM16).
   * Digital silence (frames ≤ half an LSB) is left out of this floor. Otherwise hiss
     after a muted start would look like speech.
   * The bars move only when the floor is under ≈ −66 dBFS RMS. Normal-level dictation is
     classified as before.
   * Stationary noise never reaches 8× its own floor, so noise-only recordings stay
     `no_speech`. `MIN_RUN` still rejects clicks.
2. **`vad::asr_level` before the ASR.**
   * A span the VAD accepted with RMS < 0.006 is scaled so that its peak reaches 0.5,
     with at most +40 dB of gain.
   * This applies to the commit, the segmented pieces, the previews and the language ID.
   * The log-mel features hardly depend on level, so a quiet clip now gets past the
     engines' silence gate instead of being blanked.
   * With VAD off nothing changes, and the engines' gate still guards against
     hallucinations on silence.

## Host-side check (no phone)

`cargo run -p flow-core --features wav --example vad_gate -- [--dump DIR] WAV...` runs the
commit gate on wavs. It reports `no_speech`, `blank_asr` (the ASR input is still under
0.003 RMS) or `asr`. Over the 49 bundle clips, the build before the change reproduced the
phone exactly: 34 `asr`, 12 `no_speech`, 3 `blank_asr`.

| clip | clip RMS (PCM16) | before | after |
|---|---|---|---|
| `10608586173590639256` | 28.9 | no_speech | asr (3220 ms speech, ASR input RMS 2529) |
| `12767039217536701376` | 28.5 | no_speech | asr (3800 ms, 2588) |
| `14452061739782480884` | 38.3 | no_speech | asr (860 ms, 2627) |
| `17840303524627035052` | 34.2 | no_speech | asr (1020 ms, 2434) |
| `2648447422672567017` | 25.0 | no_speech | asr (4140 ms, 1646) |
| `3368303141056942` | 26.2 | no_speech | asr (8080 ms, 2078) |
| `4915037727089100042` | 31.4 | no_speech | asr (6060 ms, 2130) |
| `4966721154804137661` | 22.2 | no_speech | asr (1800 ms, 2604) |
| `5757384795812249135` | 48.5 | no_speech | asr (6000 ms, 2634) |
| `5851382689603334877` | 25.1 | no_speech | asr (2060 ms, 2435) |
| `6120441663233860803` | 33.1 | no_speech | asr (4880 ms, 2712) |
| `8298366086940843754` | 19.9 | no_speech | asr (2940 ms, 2095) |
| `14516256554794068658` | 80.3 | blank_asr | asr (2360 ms, 2325) |
| `3426829353476331308` | 91.7 | blank_asr | asr (2600 ms, 2242) |
| `6991387072176301191` | 75.5 | blank_asr | asr (3360 ms, 3236) |

All 49 Hebrew clips now reach the ASR.

**Decoded on the host** with zipformer-he: the 15 dumped ASR inputs were decoded with
`flow-asr-sherpa`'s `online_check`.

| set | before | after |
|---|---|---|
| the 15 formerly EMPTY clips (278 words) | 100 % WER (empty) | **21.2 %** |
| the 12 formerly `no_speech` clips | 100 % WER (empty) | 16.6 % |
| the 8 quiet clips that were transcribed but trimmed short (phone r3 → host) | 28.2 % | 16.5 % |

* On the 15 clips, the VAD-trimmed and levelled audio scores better than the untrimmed,
  peak-normalised clip (22.7 %). So the trimming no longer cuts words.
* Before the change, 8 quiet clips that did get text had been trimmed to their loudest
  syllables. One of them kept only 460–1640 ms of a 4 s clip. The relative bar keeps their
  whole sentence.
* Projected Hebrew WER on the phone: about 25 % instead of 51 %, assuming the other 34
  clips stay at r3's 26.6 %. Confirm this with an `r4` phone run.
* English (LibriSpeech): all 40 clips still reach the ASR. On 4 of them the kept span grows
  by 20–280 ms at the edges. Nothing is cut.

## Tests

* `vad::tests`:
  * quiet speech at about −60 dBFS over a −90 dBFS floor counts, and its span is kept
    whole;
  * hiss at 4 low levels is still not speech;
  * hiss after digital silence is still not speech;
  * a quiet click on a clean floor is still not speech;
  * normal floors keep the old bars;
  * `asr_level` raises quiet speech only, with its gain capped.
* `tests/session.rs`, using an ASR stub with the engines' 0.003 gate:
  * quiet speech gives a RawTranscript, and the ASR gets RMS ≥ 0.003;
  * silence and quiet noise give `no_speech` without calling the ASR.

## Risk

* A low-level recording with non-stationary sound (a faint TV, typing) at more than 18 dB
  above a very clean floor now reaches the ASR, where it used to be skipped. The
  hallucination guard (`speech_ms`, Whisper's `no_speech_prob`) still applies.
* Phone microphones with AGC rarely have a floor under −66 dBFS, so on real dictation the
  quiet path should be rare.
