# Windows CPU whisper 6–18× slower than whisper-cli, 2026-11-02

> **Status:** root cause found and fixed in CI (`feat/win-cpu-whisper-speed`). Laptop
> timing of the fixed DLL is pending a CI build. Follows bug 1 of
> `2026-11-02-vulkan-he-laptop-check.md`.

## Symptom

On the laptop (Core Ultra 9 288V), with the 1.0.0 `flow_whisper_vulkan.dll` forced to the CPU:

| model, 10.38 s Hebrew FLEURS clip | whisper-cli b5454 CPU | plugin, CPU |
|---|---|---|
| ivrit turbo q5_0, 4 threads | 15.1 s | 267 s (18×) |
| ggml-base f16, 4 threads | 1.60 s | 10.6 s (6.7×) |
| ggml-base f16, 1 thread | 5.28 s | 33.3 s (6.3×) |

`flow-windows.exe` is built the same way, so its built-in CPU whisper is very likely just as slow.

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

`objdump -d` of the 1.0.0 `flow_whisper_vulkan.dll` against whisper.cpp's own
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
  `set_n_threads` unchanged (`flow-core/src/whisper.rs`), and was 4 in the bench, the same as
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
* **Regression guard.** `apps/flow-windows/tools/ci/check-ggml-opt.sh` runs after the plugin build
  and after the exe build. It prints `CMAKE_*_FLAGS_RELEASE` from each whisper-rs-sys
  `CMakeCache.txt`, and fails unless the Release `<Optimization>` of `ggml-cpu`, `ggml-base` and
  `whisper` is `MaxSpeed`.

## Laptop check (to do once CI has built the branch)

1. Extract `flow_whisper_vulkan.dll` from the branch's hybrid NSIS artefact (`7z x`).
2. Copy it to a temp folder on the laptop, not over the install.
3. Run, from the 11-02 probe (`tools/probes/prototypes/vulkan_he_check_2026_11_02/`), with
   `FLOW_WHISPER_DEVICE=cpu`, `--dll <temp>\flow_whisper_vulkan.dll`, the 10.38 s clip:
   * `plugin_bench.py` with ivrit turbo q5_0 (target ≤ ~23 s, i.e. within 1.5× of whisper-cli's
     15.1 s);
   * the same with ggml-base, at 4 threads and at 1 thread (targets ≤ ~2.4 s and ≤ ~7.9 s).

Then remove the temp folder.

Expected: whisper-cli's speed or close to it, with no OpenMP and an older ggml than b5454.

| model, 10.38 s clip | whisper-cli | plugin 1.0.0 | plugin fixed |
|---|---|---|---|
| ivrit turbo q5_0, 4 threads | 15.1 s | 267 s | _pending CI_ |
| ggml-base, 4 threads | 1.60 s | 10.6 s | _pending CI_ |
| ggml-base, 1 thread | 5.28 s | 33.3 s | _pending CI_ |
