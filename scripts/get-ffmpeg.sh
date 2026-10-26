#!/usr/bin/env bash
# Downloads the pinned ffmpeg and ffprobe for this machine into <dir>, from
# this repository's published tools-ffmpeg release, and checks each one's
# SHA-256 against tools/ffmpeg/pins-ffmpeg.txt and pins-ffprobe.txt, the
# files Zajil itself reads (ADR-0009). For the CI's video tests (instead of
# building FFmpeg on every run) and for developers; prints <dir>.
#
#   scripts/get-ffmpeg.sh <dir>
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
dir="${1:?dir}"
release="https://github.com/motalim-ai/zajil-download-manager/releases/download"

case "$(uname -s)" in
  Linux) os=linux ;;
  Darwin) os=macos ;;
  MINGW*|MSYS*|CYGWIN*) os=windows ;;
  *) echo "unknown system $(uname -s)" >&2; exit 1 ;;
esac
case "$(uname -m)" in
  x86_64|amd64) arch=x86_64 ;;
  arm64|aarch64) arch=aarch64 ;;
  *) echo "unknown processor $(uname -m)" >&2; exit 1 ;;
esac

mkdir -p "$dir"
for tool in ffmpeg ffprobe; do
  pins="$root/tools/ffmpeg/pins-$tool.txt"
  version="$(awk '$1 == "version" { print $2 }' "$pins")"
  read -r file sha256 < <(awk -v t="$os-$arch" '$1 == t { print $2, $3 }' "$pins")
  [ -n "${file:-}" ] || { echo "no $tool is pinned for $os-$arch" >&2; exit 1; }
  name="$tool"
  [ "$os" = windows ] && name="$tool.exe"
  curl --fail --silent --show-error --location --proto '=https' --tlsv1.2 \
    --retry 4 --retry-delay 5 --retry-all-errors \
    -o "$dir/$name.download" "$release/tools-ffmpeg-$version/$file"
  # From stdin: with a file name, Git Bash's sha256sum on Windows puts a
  # backslash before the hash of a path that holds one.
  actual="$( (sha256sum < "$dir/$name.download" 2>/dev/null || shasum -a 256 < "$dir/$name.download") | cut -d' ' -f1)"
  if [ "$actual" != "$sha256" ]; then
    rm -f "$dir/$name.download"
    echo "$tool $version ($file) has SHA-256 $actual, not the pinned $sha256" >&2
    exit 1
  fi
  mv "$dir/$name.download" "$dir/$name"
  chmod 700 "$dir/$name"
done
echo "$dir"
