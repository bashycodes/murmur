# FROZEN: promoted to tools/public/ (filter.sh, sync.sh; docs/public-release/sync.md). This copy is the audit trail of the 2026-11-09 first publish; change tools/public/ instead.
# common.sh: sourced by run.sh and scan.sh. The one place that names the
# scratch base and the uncommitted private files (never in the repo).
BASE=$HOME/.cache/murmur-filter
PRIVATE=$BASE/private/replacements.txt
HOSTS=$BASE/private/.hosts
