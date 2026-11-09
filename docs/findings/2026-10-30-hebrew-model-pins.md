# 2026-10-30: Hebrew model pins and the sherpa online engine (H2)

Pinned downloads for the Hebrew models of `docs/proposals/hebrew-stt.md`, for the
Windows download catalogue (H4), `fetch-models.sh --hebrew` (H3) and Android import
(H5). Sizes are bytes; SHA-256 values were checked against the Hugging Face LFS
oids. URLs are `https://huggingface.co/<repo>/resolve/<revision>/<path>`.

## zipformer2-66m-streaming-he int8 (default Hebrew model)

* Repo `ivrit-ai/zipformer2-66m-streaming-he`, revision
  `ee769a2a13b887f91572eec349458bfb0fe79e7a`. CC-BY-4.0 (attribution: Yakov
  Kolani / ivrit.ai).
* Engine: `murmur_asr_sherpa::OnlineEngine` (sherpa `OnlineRecognizer`, model type
  auto-detected as zipformer2). Put the four files in one directory.

| path in repo | local name | size | SHA-256 |
|---|---|---|---|
| `sherpa-onnx/encoder.int8.onnx` | `encoder.int8.onnx` | 70478964 | `6c6febcb74a09435315a1d872a6539f6e7fc2c333533f14c74edc0e85c49ffcd` |
| `sherpa-onnx/decoder.int8.onnx` | `decoder.int8.onnx` | 1049105 | `feb03e7c8aa761593a9284a6fa1e91259be3702de75f630062256c1a59c5a906` |
| `sherpa-onnx/joiner.int8.onnx` | `joiner.int8.onnx` | 771805 | `1d2c6d8e6dabc5eb52de52e95718e9bdfbffbfe7ead3808fb5ffd74e83bd9cae` |
| `sherpa-onnx/tokens.txt` | `tokens.txt` | 17086 | `e4ebb5e72a6aeef47437a55fa2308ebbedb7c51c557de7a89bf965c23ea9af4a` |

Total 72316960 bytes (69 MiB). The repo's `bpe.model` and `onnx/` (fp32/fp16
exports) are not needed.

## parakeet-tdt-0.6b-v3-he int8 (selectable alternative)

* Repo `krut42/voice-parakeet-he-int8`, revision
  `39de3b473bed626c7796c156d71adc66161ebfe1`. CC-BY-4.0 (base model
  thewh1teagle/parakeet-tdt-0.6b-v3-he).
* Engine: the existing `ParakeetEngine` (offline `nemo_transducer`), no code
  change. Hotwords work through the `bpe.vocab` derived from `tokens.txt`.

| path | size | SHA-256 |
|---|---|---|
| `encoder.int8.onnx` | 653215173 | `7eaf80bc88fccd579a7d3950c25e50e4c7935e5b037ec2aa1eee29bca5ca3981` |
| `decoder.int8.onnx` | 12494234 | `3a1d0bbf3e393909095f826b7719fdaef42cd2e0b8b95c54518034ecc4b7d246` |
| `joiner.int8.onnx` | 7008293 | `056c5464464b38a9cbacdccb6a9ff2114919780badb5c48e0f0260af7f91ccf2` |
| `tokens.txt` | 107590 | `55a91c862d0a63e8b9587a3264ced4fc76943e66098468b4638e6a6c5642e364` |

Total 672825290 bytes (642 MiB).

## Measured (i7-6700, 4 threads, shared box, `nice`/`choom`)

Clips: FLEURS he_il dev, first 5 files of `dev.tar.gz` (not committed). Three of
the five are recorded very quietly (RMS 0.0007-0.05); the two quietest fall
under the engines' silence gate (RMS 0.003), so the test and `online_check
--norm` scale clips to peak 0.5 first. Real microphone audio is louder.

| model / search | clip | RTF | load |
|---|---|---|---|
| zipformer-he greedy | 13.3 s | 0.047 | 3.3-9 s |
| zipformer-he greedy | 16.1 s | 0.045 | |
| zipformer-he beam 4 + 1 hotword | 16.1 s | 0.067 | |
| parakeet-v3-he greedy | 16.1 s | 0.065 | 16.7 s (beam + greedy sessions) |
| parakeet-v3-he beam 4 | 16.1 s | 0.084 | |

* Feeding the whole buffer at once gives exactly the same text as feeding 100 ms
  chunks (checked on all 5 clips).
* Tail padding 0.8 s, 1.5 s and 3 s gave the same text; 1.5 s is the default
  (the model card's value). It costs about 70 ms per decode.

## Hotwords on the zipformer

They work, with the same `bpe.vocab` derivation as Parakeet (the model's
tokenizer is the English 500 pieces plus 1,024 Hebrew pieces in one table, so
score = -id stays monotonic inside each script). Example: greedy writes
"הארכי פלג"; with the hotword "הארכיפלג" (score 1.0, beam 4) the beam decode
writes "הארכיפלג".

They are weaker than on Parakeet: sherpa's *online* modified beam search adds the
context bonus **after** choosing the top `max_active_paths` candidates
(`online-transducer-modified-beam-search-decoder.cc`), so a hotword can only
re-rank paths the model already considers. It never inserts a word the acoustics
do not support (no effect for "פלפל" vs the model's "בלבל", even at score 10 and
beam 64), which also means unrelated hotwords are harmless. Scores of 4 and more
changed unrelated words, so the default stays 1.0.

## Reproduce

```sh
cargo run --release -p murmur-asr-sherpa --example online_check -- \
  MODEL_DIR --norm [--hotwords a,b] [--beam N] [--score F] clip.wav ...
MURMUR_ZIPFORMER_HE_DIR=… MURMUR_HE_WAV=…/10012360550181770926.wav \
  MURMUR_HE_HOTWORD=הארכיפלג cargo test -p murmur-asr-sherpa --test online -- --nocapture
MURMUR_PARAKEET_HE_DIR=… MURMUR_HE_WAV=… cargo test -p murmur-asr-sherpa --test model parakeet_he
```
