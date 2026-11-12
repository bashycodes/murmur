# Coding standards

Read at review time by the reviewer (`/mattpocock-skills:code-review` on a
`feat/` branch against `origin/integration`), not by implementers. Only
judgement calls live here; anything a script can decide is a check in CI
(`tools/checks/`, `tools/check-*.sh`, clippy `-D warnings`), and the reviewer
skips it.

Each rule comes from a problem a review caught before merge.

## Privacy

- **No owner content.** Test fixtures, examples, commit messages and docs use
  made-up sentences, never the owner's own dictations, names or messages,
  even ones quoted in the ticket. Shape-preserving stand-ins are fine.
- **No audio or transcript at rest** unless a decision allows it, and then
  behind an off-by-default setting (see the server debug capture). Logging a
  transcript needs `log_text` or an equivalent opt-in.
- **Keys and tokens** never reach logs, `Debug` output, error strings or
  test snapshots.
- **Cloud paths keep the consent gate.** Any new route that sends audio or
  text off the device goes through the router's cloud gate and is rechecked
  on fallback hops. A provider is only ever reached through OpenRouter,
  except AI cleanup's custom server (D52), whose host the user typed and
  consented to, and (D85) audio to that same LiteLLM proxy for speech to
  text, only after a separate audio consent for that host.

## Shared code

- **Reuse before extracting.** When the diff pulls a helper out of shared
  code (server connection setup, model download, settings I/O), check
  whether integration already has one from a branch merged meanwhile. Two
  versions of the same seam is a hard finding.
- **Shared crates mean both apps.** A change under `crates/` that one app
  calls is checked for the other app's call sites too: CI runs both builds,
  but a behaviour change that compiles is still a change.

## Docs that travel with code

- **Claims are checked.** Findings, comparisons and README statements about
  what the app does or lacks must match the code and past tests. A wrong
  "never ran on Windows" or "feature missing" is a hard finding.
- **CHANGELOG under `[Unreleased]`**, one line per user-visible change, and a
  merge must not bring back a line integration deliberately removed.
- **Decisions** get the next free `D` number on the rebased branch; a
  renumber updates every reference to the old id.

## Tests

- **A fix carries a regression test** that fails on the old code, or the
  report says why one isn't possible (device-only behaviour).
- **Benchmarks measure what the change is about.** A punctuation or
  formatting change isn't validated by a word-error-rate run that ignores
  punctuation.
