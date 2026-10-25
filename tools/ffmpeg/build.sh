#!/usr/bin/env bash
# Builds ffmpeg and ffprobe from the pinned FFmpeg source (ADR-0009).
#
#   tools/ffmpeg/build.sh <dist|testgen> <source tarball> <out dir>
#
# dist:    the build Zajil distributes (configure-dist.txt), named ffmpeg
#          and ffprobe as FFmpeg names them (LGPL checklist, item 16).
# testgen: the test-only build (configure-testgen.txt), named
#          ffmpeg-testgen and ffprobe-testgen so it can never be taken for
#          the distributed one.
#
# Extra configure options for the platform (cross-compiling, static
# linking) come in ZAJIL_FFMPEG_EXTRA, one per line. The tarball must
# already match FFMPEG_SHA256: get-source.sh checks it and its signature.
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=tools/ffmpeg/source.env
. "$here/source.env"

variant="${1:?dist or testgen}"
tarball="${2:?source tarball}"
out="${3:?out dir}"
case "$variant" in
  dist) suffix="" ;;
  testgen) suffix="-testgen" ;;
  *) echo "unknown variant: $variant" >&2; exit 2 ;;
esac

sha256() {
  if command -v sha256sum >/dev/null 2>&1; then sha256sum "$1" | cut -d' ' -f1
  else shasum -a 256 "$1" | cut -d' ' -f1; fi
}

actual="$(sha256 "$tarball")"
if [ -z "$FFMPEG_SHA256" ] || [ "$actual" != "$FFMPEG_SHA256" ]; then
  echo "::error::the source tarball's SHA-256 is $actual, not the pinned '$FFMPEG_SHA256'" >&2
  exit 1
fi

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
tar -xJf "$tarball" -C "$work"
src="$work/ffmpeg-$FFMPEG_VERSION"

# The options, one per line, comments and blank lines left out.
opts=()
while IFS= read -r line; do
  case "$line" in ''|'#'*) continue ;; esac
  opts+=("$line")
done < "$here/configure-$variant.txt"
while IFS= read -r line; do
  [ -n "$line" ] && opts+=("$line")
done <<< "${ZAJIL_FFMPEG_EXTRA:-}"

for opt in "${opts[@]}"; do
  case "$opt" in
    --enable-gpl|--enable-version3|--enable-nonfree|--enable-lib*)
      echo "::error::$opt is not allowed in a Zajil FFmpeg build (ADR-0009)" >&2
      exit 1 ;;
  esac
done

(cd "$src" && ./configure "${opts[@]}" > "$work/configure.log") || {
  tail -n 40 "$src/ffbuild/config.log" >&2 || true
  exit 1
}

# What configure decided, read back from its own output: LGPL, nothing
# GPL, nothing non-free (checklist items 1, 17 and 18).
grep -qx '#define CONFIG_GPL 0' "$src/config.h"
grep -qx '#define CONFIG_NONFREE 0' "$src/config.h"
grep -qx '#define CONFIG_VERSION3 0' "$src/config.h"
grep -q '^License: LGPL version 2.1 or later$' "$work/configure.log"

jobs="$(getconf _NPROCESSORS_ONLN 2>/dev/null || sysctl -n hw.ncpu 2>/dev/null || echo 2)"
make -C "$src" -j"$jobs" > "$work/make.log" 2>&1 || { tail -n 60 "$work/make.log" >&2; exit 1; }

mkdir -p "$out"
exe=""
[ -f "$src/ffmpeg.exe" ] && exe=".exe"
cp "$src/ffmpeg$exe" "$out/ffmpeg$suffix$exe"
cp "$src/ffprobe$exe" "$out/ffprobe$suffix$exe"

# The build file of checklist item 6: how this exact binary was made.
{
  echo "FFmpeg $FFMPEG_VERSION, built for Zajil ($variant)"
  echo "source: $FFMPEG_URL"
  echo "source SHA-256: $FFMPEG_SHA256"
  echo "build script: tools/ffmpeg/build.sh in https://github.com/motalim-ai/zajil-download-manager"
  echo "commit: ${GITHUB_SHA:-local}"
  echo "system: $(uname -srm)"
  echo "compiler: $(${CC:-cc} --version 2>/dev/null | head -n 1)"
  echo "changes to the FFmpeg source: none"
  echo
  echo "configure line:"
  printf './configure'
  printf ' %s' "${opts[@]}"
  echo
} > "$out/ffmpeg-build$suffix.txt"

ls -l "$out"
