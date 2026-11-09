# Repo security

## Leak scan

Two layers keep secrets and owner details out of the repo:

- **Local pre-push hook**: the owner's global `leak-scan` (private project
  names and similar; its patterns live outside the repo).
- **Repo-local scan**, `tools/checks/leak-scan.py`, run by CI (`repo-checks`)
  on every push and pull request: the tree at HEAD, the lines added in the
  pushed range, its commit messages, and its commit identities. CI is the
  layer that sees a merge made on github.com, which no local hook does.

Patterns are in `tools/checks/leak-patterns.txt`: `sk-` API keys (OpenRouter,
OpenAI), GitHub tokens (`ghp_`, `gho_`, `ghu_`, `ghs_`, `ghr_`, `github_pat_`),
Hugging Face `hf_` tokens, AWS `AKIA`/`ASIA` key ids, PEM private keys,
JWT-shaped strings, tailnet addresses (100.64.0.0/10), and absolute home paths
(`/home/<user>`, `C:\Users\<name>`, also as `file://` URLs).

Commit author and committer emails must be GitHub noreply addresses
(`noreply@anthropic.com` is allowed for commits made by coding agents). A pull
request from an outside contributor whose commits carry a personal email fails
`repo-checks`; they need to set GitHub's "Keep my email addresses private" and
recommit. The scan cannot tell a real name from a handle, so the name a commit
displays is a manual check.

Fake values in tests carry a marker token as a delimited segment
(`sk-or-v1-LEAKCHECK-...`, `hf_FAKE...`; see `tools/checks/leak-allow.txt`);
a new fake uses one of them rather than a new allowlist entry. A hit prints
the pattern name, the location and at most 4 characters of the value
(tailnet addresses as `100.x.x.x`). Errors (bad range, bad pattern file)
exit 2.

Run it before pushing: `tools/checks/leak-scan.py origin/integration...HEAD`.
Tests: `tools/checks/leak-scan_test.sh`.

## GitHub settings for the public repo (`bashycodes/murmur`)

Repository, Settings → Code security:

- [ ] **Secret scanning** on.
- [ ] **Push protection** on, so GitHub rejects a push carrying a known
      token format.
- [ ] Dependabot alerts on (optional, cheap).

Repository, Settings → Rules (ruleset on `integration`):

- [ ] Block force pushes and deletion of `integration`, and of `v*` tags.
- [ ] Require a pull request before merging, with the `repo-checks` status
      check passing, or restrict who can push, so nothing lands on
      `integration` without the CI leak scan.
- [ ] If merges are ever done in the web UI: only after the account settings
      below, since a web merge is authored with the account's commit email.

Account, Settings → Emails (the owner's GitHub account):

- [ ] **Keep my email addresses private**, so web commits and merges use the
      `users.noreply.github.com` address.
- [ ] **Block command line pushes that expose my email**, so a local commit
      made with a personal address is refused at push time.
- [ ] Profile display name: the name every web commit carries; check it is
      the one meant to be public.
