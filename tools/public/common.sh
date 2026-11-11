# shellcheck shell=bash disable=SC2034
# common.sh: sourced by filter.sh and scan.sh. The one place that names the
# scratch base and the uncommitted private files (never in the repo).
BASE=${MURMUR_FILTER_BASE:-$HOME/.cache/murmur-filter}
PRIVATE=$BASE/private/replacements.txt
HOSTS=$BASE/private/.hosts
