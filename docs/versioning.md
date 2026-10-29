# Versioning (D26)

One product version for the whole repo: the Windows app, the three Android
flavours, flow-server and every crate share it. A build also carries the
git commit it was made from, so a bug report, a debug-log line or a server
log pins the exact code.

## 1. Version scheme

* **SemVer `MAJOR.MINOR.PATCH`, pre-1.0.** While MAJOR is 0, a MINOR bump
  may break things (settings, wire protocol, file formats); PATCH is fixes
  only. `1.0.0` is a product decision, not a date.
* No pre-release suffixes (`-rc.1`): Android's versionCode cannot order them
  and the release flow does not need them. Commits between releases are told
  apart by their build metadata (§3).
* Ranges: MINOR and PATCH ≤ 99, MAJOR ≤ 209999 (the versionCode ceiling).
* **Starting version: 0.2.0.** `0.1.0` was never tagged but is baked into
  every build shipped so far (APKs with versionCode 1, `Flow-Clone_0.1.0_*`
  installers, `flow-server 0.1.0`). Tagging `v0.1.0` now would give one name
  to many different builds; `0.2.0` is the first version that means exactly
  one commit. Until it is cut, the tree says `0.1.0` (+ commit, see §3).

## 2. Single source of truth

The top-level **`VERSION`** file (one line, `0.2.0`). Everything else derives
from it or is a stamped copy that `tools/check-version.sh` verifies:

| Where | How it gets the version |
|---|---|
| every crate in `crates/` | `version.workspace = true` → root `Cargo.toml` `[workspace.package] version` (stamped copy) |
| root `Cargo.lock`, `apps/flow-windows/Cargo.lock` | the `flow-*` path packages' entries (stamped copies; CI builds `--locked`) |
| `apps/flow-windows/Cargo.toml` (own workspace) | `[package] version` (stamped copy) |
| Windows installers (Tauri) | `tauri.conf.json` has **no** `version`, so Tauri 2 takes Cargo.toml's → `Murmur_0.8.0_x64-setup_hybrid.exe` (`Flow-Clone_…` up to 0.7.1) |
| `apps/flow-windows/ui/package.json` + `package-lock.json` | stamped copies (kept so `npm ci` sees a consistent lock) |
| Android `versionName` / `versionCode` | read from `VERSION` by `app/build.gradle.kts` at build time; a literal fails the check |
| `flow_core::VERSION`, `flow_build_info::VERSION` | `CARGO_PKG_VERSION` |

Guards: `crates/flow-build-info/build.rs` and `apps/flow-windows/build.rs`
fail the Rust build when their package version ≠ `VERSION`;
`AppVersionTest` fails when BuildConfig ≠ `VERSION`; `tools/check-version.sh`
lists every copy and fails on any difference. Never edit a copy by hand:
`tools/release.sh` stamps all of them.

## 3. Build metadata

`build` = the 7-hex git short sha, `-dirty` appended when tracked files
differed from that commit at build time, `unknown` without git. `full` =
SemVer with build metadata: `0.2.0+1a2b3c4` / `0.2.0+1a2b3c4.dirty` /
`0.2.0+unknown`. `FLOW_GIT_SHA` / `FLOW_GIT_DIRTY=1` override git (e.g. a
build from a source tarball). Untracked files do not count as dirty.

* **Rust:** crate `flow-build-info` (no deps) bakes `VERSION`, `GIT_SHA`,
  `GIT_DIRTY`, `BUILD`, `FULL` at compile time. Its build script re-runs when
  HEAD, the index, `VERSION` or any source under `crates/` /
  `apps/flow-windows/{src,ui/src}` changes, so the dirty flag follows edits.
  Only leaf crates depend on it (flow-server, flow-ffi, flow-windows), so a
  new commit does not rebuild flow-core.
* **Android:** Gradle stamps `BuildConfig.GIT_SHA`, `GIT_DIRTY`, `BUILD`;
  `dev.flowclone.data.AppVersion.current` is the app-side view. The native
  `.so` reports its own build (`Native engine: flow-ffi 0.2.0+1a2b3c4`),
  which is how an APK packaged with a stale `.so` shows up.

Where it is shown:

| Surface | Text |
|---|---|
| Android main screen | `Version 0.2.0 (hybrid) · 1a2b3c4` + `Native engine: flow-ffi 0.2.0+1a2b3c4` |
| Android Developer → Build (hub and Status page) | `Version: 0.2.0 (200) · build 1a2b3c4` |
| Android Settings → About row (7 taps unlock Developer settings) | `Version 0.2.0 (hybrid) · 1a2b3c4` (`AppVersion.current.label()`, same as the main screen) |
| Windows Settings About row (`version_row`) | `Murmur 0.2.0 · 1a2b3c4` |
| Windows Developer build line | `Murmur 0.2.0+1a2b3c4 · client + server · ASR … · proto 1 · core 0.2.0` |
| flow-server | `ready` log line `version=0.2.0+1a2b3c4`; `hello_ok.server` / `/v1/health` `flow-server 0.2.0+1a2b3c4` (shown by every client's Test connection) |
| `hello.client` (both shells → server log `connected client=…`) | `flow-ffi 0.2.0+1a2b3c4` / `flow-windows 0.2.0+1a2b3c4` |
| D21 debug log, both platforms | every line (dictation and event): `"app_version":"0.2.0","build":"1a2b3c4"` right after `ts` (additive) |
| CI artifacts | `flow-clone-android-debug-apks-<label>`, `flow-clone-windows-<flavour>-<label>`; label = tag on tag builds, else `<version>+<sha>` |

## 4. Android versionCode

`versionCode = MAJOR × 10000 + MINOR × 100 + PATCH` (0.2.0 → 200, 1.2.3 →
10203). Monotonic in the version, deterministic, and every flavour of one
release shares it: the flavours share one applicationId and switching is an
in-place update (D17), so they must not be ordered against each other. The
first versioned build (200, or 100 for the untagged 0.1.0 tree) is above the
old literal 1, so it installs over existing APKs. If per-ABI APKs ever ship,
the scheme becomes `(…) × 10 + abi` (still larger than every earlier code).
The formula lives in `app/build.gradle.kts` and `AppVersion.code`; shared
vectors (`tools/versioning/fixtures/version_vectors.json`) pin both and the
Python rules.

## 5. What stays independent

* **Wire protocol** (`flow_protocol::PROTO_MIN/MAX`, path `/v1/`): negotiated
  between any client and server version (`docs/protocol.md` §5). A product
  release does not imply a protocol change and vice versa; tying them would
  either force protocol bumps on every release or make mixed-version
  client/server pairs look incompatible when they are not. The hello `core`
  field stays the plain product version and is informational.
* **`POSTPROCESS_REV`** (date-based, compared for the Developer warning):
  tracks post-processing behaviour, which changes far less often than the
  product.
* Model files, the D21 log schema and Wispr's own version numbers in
  `research/` are unrelated.

## 6. Tags and releases

* Annotated tags **`vX.Y.Z`** on `integration`, created only by
  `tools/release.sh`, never moved or deleted once pushed.
* `CHANGELOG.md` ([Keep a Changelog](https://keepachangelog.com/en/1.1.0/)):
  notes go under `## [Unreleased]` as work lands (Added / Changed / Fixed /
  Removed / Security); the release turns that section into
  `## [X.Y.Z] - YYYY-MM-DD`.

Cutting a release:

```sh
tools/release.sh 0.2.0            # dry run: refusals, the full diff, the notes; changes nothing
tools/release.sh 0.2.0 --apply    # stamp VERSION + copies, date CHANGELOG, check, commit "Release 0.2.0", tag v0.2.0
git push origin integration v0.2.0
```

`--apply` refuses a version not newer than `VERSION`, an existing tag, an
empty `[Unreleased]`, uncommitted changes to tracked files, a branch other
than `integration` (`FLOW_RELEASE_ANY_BRANCH=1` overrides) and lockfiles cargo
would rewrite. It never pushes. `FLOW_RELEASE_DATE` / `FLOW_REPO_URL` set the
date and add compare links.

On the pushed tag both workflows run `tools/check-version.sh --tag vX.Y.Z`
(tag = `v` + `VERSION`, CHANGELOG section present), refuse a dirty tree, build
as usual, and a `release` job (tags only, `contents: write`) attaches
`flow-clone-android-<flavour>-X.Y.Z-debug.apk`, the Windows MSI/NSIS
installers and `SHA256SUMS-*` to the GitHub release `vX.Y.Z`, creating it
from the CHANGELOG section if the other workflow has not yet. The APKs are
debug-signed and the installers unsigned, as in every CI build (no secrets
in CI).

## 7. Checks

* `tools/check-version.sh [--tag vX.Y.Z]`: runs the rules' unit tests
  (`tools/versioning/test_versioning.py`), then lists every copy and fails on
  a mismatch. Run by both CI workflows, by `tools/check-build-matrix.sh`
  (the local gate) and by `tools/release.sh`.
* `python3 -m unittest discover -s tools/versioning`: version parsing,
  versionCode, build metadata, copy discovery, stamping, CHANGELOG rotation.
* `cargo test -p flow-build-info` (shared `describe` vectors, baked constants
  = `VERSION`), `flow-server` `ws_protocol::hello_ok_describes_the_server`,
  Android `AppVersionTest` + `DebugLogTest.everyLineNamesTheBuild`, Windows
  `debug_log` schema tests.
