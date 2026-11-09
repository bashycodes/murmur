# Windows CPU whisper 6–18× slower than whisper-cli, 2026-11-02

> **Status:** root cause found and fixed in CI (`feat/win-cpu-whisper-speed`), confirmed on
> the laptop: ivrit turbo on the CPU now takes 18 s, down from 267 s. Follows bug 1 of
> `2026-11-02-vulkan-he-laptop-check.md`.

## Symptom

On the laptop (Core Ultra 9 288V), with the 1.0.0 `murmur_whisper_vulkan.dll` forced to the CPU:

| model, 10.38 s Hebrew FLEURS clip | whisper-cli b5454 CPU | plugin, CPU |
|---|---|---|
| ivrit turbo q5_0, 4 threads | 15.1 s | 267 s (18×) |
| ggml-base f16, 4 threads | 1.60 s | 10.6 s (6.7×) |
| ggml-base f16, 1 thread | 5.28 s | 33.3 s (6.3×) |

`murmur-windows.exe` is built the same way, so its built-in CPU whisper is very likely just as slow.

## Root cause: ggml was compiled at MSVC `/Od` (no optimisation)

The `cmake` crate (0.1.58) runs CMake with the Visual Studio generator on MSVC. Its
`set_compiler` (`src/lib.rs`, "The visual studio generator apparently doesn't respect
`CMAKE_C_FLAGS`…") then **overwrites `CMAKE_<LANG>_FLAGS_<BUILD_TYPE>`** with cc's flags. It
builds those with `opt_level(0)`, and it strips every `-O`/`/O` flag anyway, so on MSVC they are
only `-nologo -MD -Brepro`. CMake's Release defaults (`/O2 /Ob2 /DNDEBUG`) are gone, and cl
falls back to its default, `/Od`. This is upstream bug cmake-rs#240. `llama-cpp-sys-2` works
around it: it re-adds `/O2 /DNDEBUG /Ob2` on MSVC, which is why the AI cleanup DLLs were fine.
`whisper-rs-sys` 0.15.0 does not.

| | `CMAKE_C_FLAGS_RELEASE` | `CMAKE_CXX_FLAGS_RELEASE` | vcxproj Release `<Optimization>` |
|---|---|---|---|
| before (1.0.0) | ` -nologo -MD -Brepro` | `/utf-8 -nologo -MD -Brepro` | not MaxSpeed (cl default `/Od`) |
| after | `-MD -O2 -Ob2 -DNDEBUG` | `-MD -O2 -Ob2 -DNDEBUG` (ggml adds `/utf-8` itself) | MaxSpeed (checked in CI) |

Both columns of "before" come from the cmake-rs and cc 1.9.0 sources (`cc` MSVC defaults:
`-nologo`, `-MD`, `-Brepro`; no `-O` at opt level 0). The CI log does not print the CMake
command line, so these values were not read from a CI run.

A side effect: `cargo test` (a Rust debug build) was fast. whisper-rs-sys then defines
`CMAKE_BUILD_TYPE=RelWithDebInfo`, so cmake-rs overwrites `…_FLAGS_RELWITHDEBINFO`, but it still
builds `--config Release`, which keeps CMake's default `/O2`. Only the release builds that ship
were unoptimised.

### Evidence in the shipped binary

`objdump -d` of the 1.0.0 `murmur_whisper_vulkan.dll` against whisper.cpp's own
`ggml-cpu-haswell.dll` (the whisper-cli build on the laptop, `/O2`):

| | ymm instructions | of them with a stack operand (`[rsp/rbp+…]`) | ymm stores to the stack | `vfmadd*` |
|---|---|---|---|---|
| plugin 1.0.0 | 17 583 | 8 374 (48 %) | 6 790 | 104 |
| ggml-cpu-haswell | 10 952 | 1 454 (13 %) | 1 237 | 297 |

The plugin's quant dot product is textbook `/Od` code. Every intrinsic loads its inputs from
the frame into `ymm0`, and stores the result straight back:

```
vmovdqu ymm0,YMMWORD PTR [rbp+0x200]
vpmaddubsw ymm0,ymm0,YMMWORD PTR [rbp+0x220]
vmovdqu YMMWORD PTR [rbp+0xc0],ymm0
vmovdqu ymm0,YMMWORD PTR [rbp+0x240]
```

There is no inlining at `/Od` either, so FMA helper intrinsics appear once rather than once per
call site. That explains the low `vfmadd` and `vcvtph2ps` counts that looked like missing
FMA/F16C.

### Suspects ruled out

* **AVX2/FMA/F16C flags.** They are set. `GGML_NATIVE=OFF` makes `INS_ENB=ON`, so
  `GGML_AVX2`/`FMA`/`F16C`/`BMI2` are on, and on MSVC ggml-cpu gets `/arch:AVX2` plus the defines
  `GGML_AVX2 GGML_FMA GGML_F16C __BMI2__` (`ggml/src/ggml-cpu/CMakeLists.txt`).
* **OpenMP off.** It is off (`GGML_OPENMP=OFF` without the whisper-rs `openmp` feature), and ggml
  uses its own thread pool instead. But single-threaded decoding is just as slow (6.3×), so this
  is not threading. OpenMP stays off: on MSVC it would add a `vcomp140.dll` dependency.
* **Thread count.** `threads` comes from prefs `asr_threads` or `default_threads()`, reaches
  `set_n_threads` unchanged (`murmur-core/src/whisper.rs`), and was 4 in the bench, the same as
  whisper-cli `-t 4`.

## Fix

`windows-build.yml` sets, at workflow level:

```
CMAKE_C_FLAGS_RELEASE: "-MD -O2 -Ob2 -DNDEBUG"
CMAKE_CXX_FLAGS_RELEASE: "-MD -O2 -Ob2 -DNDEBUG"
```

* **How it reaches CMake.** whisper-rs-sys forwards every `CMAKE_*` env var as a `-D` define,
  and cmake-rs leaves an already-defined `…_FLAGS_RELEASE` alone.
* **Flag details.**
  * `-MD` keeps the CRT that Rust links.
  * Dashes, not slashes, because the MSYS bash of the run steps rewrites a `/…` env value into a
    Windows path.
* **Why not `.cargo/config.toml`.** That file also applies on Linux, where gcc would reject
  these flags.
* **Other crates.** llama-cpp-sys-2 also forwards them, so the AI cleanup DLLs get the same
  flags as before.
* **Unchanged.** `GGML_NATIVE=OFF` (D42's portable AVX2 baseline) and the link set
  (`vulkan-1.dll` only in the plugin).
* **Cache.** The rust-cache key is bumped (`ggml-native-off-o2`), because whisper-rs-sys does not
  track these env vars.
* **Regression guard.** `apps/murmur-windows/tools/ci/check-ggml-opt.sh` runs after the plugin build
  and after the exe build. It prints `CMAKE_*_FLAGS_RELEASE` from each whisper-rs-sys
  `CMakeCache.txt`, and fails unless the Release `<Optimization>` of `ggml-cpu`, `ggml-base` and
  `whisper` is `MaxSpeed`.

## Laptop check (CI run 41403282165, commit 4d3f227)

* **CI.** The run is green. `check-ggml-opt.sh` passed for the plugin build and for the exe
  build: `CMAKE_C/CXX_FLAGS_RELEASE:STRING=-MD -O2 -Ob2 -DNDEBUG`, and `MaxSpeed` for `ggml-cpu`,
  `ggml-base` and `whisper`.
* **Method.**
  * `murmur_whisper_vulkan.dll` was taken from the run's hybrid MSI with `msiexec /a` (an
    administrative extract, nothing installed) into a temp folder on the laptop.
  * It was timed with the 11-02 probe's `plugin_bench.py`: `--dll <temp>`,
    `MURMUR_WHISPER_DEVICE=cpu`, the 10.38 s clip, one warm-up.
  * The temp folder was removed afterwards. The install and prefs were not touched.
* **Results.** `tools/probes/prototypes/vulkan_he_check_2026_11_02/results/o2-*.jsonl`.

| model, 10.38 s clip | whisper-cli b5454 | plugin 1.0.0 | plugin fixed | fixed vs whisper-cli |
|---|---|---|---|---|
| ivrit turbo q5_0, CPU, 4 threads | 15.1 s | 267 s | **18.0 s** (warm-up 15.8 s), 3/22 word errors | 1.19× |
| ggml-base, CPU, 4 threads | 1.60 s | 10.6 s | **1.08 s** | 0.67× |
| ggml-base, CPU, 1 thread | 5.28 s | 33.3 s | **3.33 s** | 0.63× |
| ivrit turbo q5_0, Vulkan GPU | 1.18 s | 1.47 s | **1.26 s** | 1.07× |

* Acceptance (within ~1.5× of whisper-cli) is met.
* The 15× speed-up on ivrit and ~10× on base fit an unoptimised build.
* The GPU path is a little faster too. The host-side ggml code it runs (graph build, ops that
  stay on the CPU) is now optimised as well.
* whisper-cli's times are its "total − load". The plugin's are the wall time of one
  `murmur_whisper_transcribe`, so the two columns are close but not identical measures.
