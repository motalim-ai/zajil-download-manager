#!/usr/bin/env bash
# Downloads the pinned FFmpeg source tarball and checks it twice: its
# OpenPGP signature by the FFmpeg release key (FFMPEG_SIGNING_KEY), and
# its SHA-256 (FFMPEG_SHA256). Writes the tarball and its .asc to <dir>.
#
#   tools/ffmpeg/get-source.sh <dir>
#
# While FFMPEG_SHA256 is empty, a tarball whose signature is good has its
# SHA-256 printed and the script fails, so the hash is pinned on purpose,
# never taken silently.
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=tools/ffmpeg/source.env
. "$here/source.env"
dir="${1:?dir}"
mkdir -p "$dir"

fetch() {
  curl --fail --silent --show-error --location --proto '=https' --tlsv1.2 \
    --retry 4 --retry-delay 5 --retry-all-errors -o "$2" "$1"
}
fetch "$FFMPEG_URL" "$dir/$FFMPEG_TARBALL"
fetch "$FFMPEG_URL.asc" "$dir/$FFMPEG_TARBALL.asc"
fetch "https://ffmpeg.org/ffmpeg-devel.asc" "$dir/ffmpeg-devel.asc"

gnupg="$(mktemp -d)"
trap 'rm -rf "$gnupg"' EXIT
export GNUPGHOME="$gnupg"
gpg --batch --quiet --import "$dir/ffmpeg-devel.asc"
# The key file must hold the pinned key; a different key in it signs nothing.
fingerprints="$(gpg --batch --with-colons --fingerprint)"
grep -q "^fpr:::::::::$FFMPEG_SIGNING_KEY:" <<< "$fingerprints" || {
  echo "::error::ffmpeg-devel.asc does not hold the key $FFMPEG_SIGNING_KEY" >&2
  exit 1
}
status="$(gpg --batch --status-fd 1 --verify "$dir/$FFMPEG_TARBALL.asc" "$dir/$FFMPEG_TARBALL" 2>"$gnupg/verify.err")" || {
  echo "::error::gpg could not verify $FFMPEG_TARBALL" >&2
  cat "$gnupg/verify.err" >&2
  exit 1
}
# A good signature (GOODSIG), valid (VALIDSIG, which ends with the primary
# key's fingerprint, whichever subkey signed), and by a key neither expired
# nor revoked.
if ! grep -q '^\[GNUPG:\] GOODSIG ' <<< "$status" ||
   ! grep -qE "^\[GNUPG:\] VALIDSIG .* $FFMPEG_SIGNING_KEY\$" <<< "$status" ||
   grep -qE '^\[GNUPG:\] (EXPKEYSIG|REVKEYSIG|EXPSIG) ' <<< "$status"; then
  echo "::error::$FFMPEG_TARBALL is not signed by $FFMPEG_SIGNING_KEY" >&2
  printf '%s\n' "$status" >&2
  exit 1
fi
rm -f "$dir/ffmpeg-devel.asc"

actual="$(sha256sum "$dir/$FFMPEG_TARBALL" | cut -d' ' -f1)"
echo "$FFMPEG_TARBALL: signature good (key $FFMPEG_SIGNING_KEY), SHA-256 $actual"
if [ -z "$FFMPEG_SHA256" ]; then
  echo "::error::FFMPEG_SHA256 is not pinned yet. The signed tarball's SHA-256 is $actual" >&2
  exit 1
fi
if [ "$actual" != "$FFMPEG_SHA256" ]; then
  echo "::error::$FFMPEG_TARBALL has SHA-256 $actual, not the pinned $FFMPEG_SHA256" >&2
  exit 1
fi
