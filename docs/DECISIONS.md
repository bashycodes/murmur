# Decisions (defaults chosen autonomously 2026-10-20; user may override)

User goal: clone Wispr Flow (Windows + Android), all STT/LLM local, zero third-party network traffic.

| # | Question (win 10 §8 / android 10 §8) | Default | Why |
|---|---|---|---|
| D1 | Framework | Tauri 2 / Rust (Windows shell), Kotlin shell (Android) | spec recommendation; one Rust core |
| D2 | Shared core | Yes — `flow-core` Rust crate, §9 v2 API | both specs agree |
| D3 | ASR engine | whisper.cpp via `whisper-rs`, multilingual; engine behind a trait so Parakeet/sherpa-onnx can be added | languages unknown -> multilingual is the safe superset |
| D4 | Models | desktop: large-v3-turbo q5 (GPU) / small q5 (CPU); Android: base q5. Tests: tiny | dev box has no GPU |
| D5 | LLM cleanup | llama.cpp, optional, OFF by default; deterministic post-processing always on | CPU-only target; privacy unaffected |
| D6 | Min Android | 13 (API 33) — a11y InputMethod | matches Wispr |
| D7 | Distribution | sideload / F-Droid; no INTERNET permission in core APK | privacy + Play a11y policy |
| D8 | Reconstructed proto (windows-research) | keep as-is, do not extend; clone never speaks it | not needed by clone |
| D9 | Network | none at runtime; model download only via explicit user action (separate optional flavor on Android) | goal |
| D10 | LLM packaging | llama.cpp lives in the `flow-llm` cdylib plugin, loaded at runtime by flow-core's `llm` feature; never linked into the same binary as whisper.cpp | whisper-rs-sys and llama-cpp-sys-2 each bundle a different ggml → duplicate-symbol link failure; a cdylib exports only `flow_llm_*` |
| D11 | Recommended LLM | Qwen2.5-0.5B-Instruct Q4_K_M for the 1.5 s CPU budget; 1.5B when the budget or hardware allows (8/8 vs 4/8 on the messy-dictation set, ~2× latency) | measured, `tests/llm_e2e.rs`; final pick pending `docs/model-benchmarks.md` |
