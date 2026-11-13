#!/usr/bin/env bash
# Unpack a user-space espeak-ng (no root) into the cache dir, for the
# Hebrew / mixed TTS clips. Uses the distro's own packages (apt-get
# download, no install) and a stub libpcaudio (pcaudio-stub.c) so no
# PulseAudio is needed: the harness only writes WAV files.
# Prints the path of the wrapper script on success.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
cache="${MURMUR_E2E_CACHE:-${XDG_CACHE_HOME:-$HOME/.cache}/murmur-e2e}/espeak"
wrap="$cache/espeak-ng"
if [[ -x "$wrap" ]]; then echo "$wrap"; exit 0; fi
if command -v espeak-ng >/dev/null; then command -v espeak-ng; exit 0; fi

mkdir -p "$cache/debs" "$cache/root"
(cd "$cache/debs" && apt-get download -q espeak-ng espeak-ng-data libespeak-ng1 libsonic0 >&2)
for d in "$cache"/debs/*.deb; do dpkg -x "$d" "$cache/root"; done
lib="$cache/root/usr/lib/x86_64-linux-gnu"
gcc -shared -fPIC -O2 -o "$cache/root/libpcaudio.so.0" "$here/pcaudio-stub.c"
cat >"$wrap" <<EOF
#!/usr/bin/env bash
export LD_LIBRARY_PATH="$cache/root:$lib\${LD_LIBRARY_PATH:+:\$LD_LIBRARY_PATH}"
exec "$cache/root/usr/bin/espeak-ng" --path="$lib" "\$@"
EOF
chmod +x "$wrap"
echo "$wrap"
