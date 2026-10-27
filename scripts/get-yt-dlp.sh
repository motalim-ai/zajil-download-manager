#!/usr/bin/env bash
# Downloads the pinned yt-dlp for this machine into <dir> and checks its
# SHA-256 against tools/yt-dlp/pins.txt, the file Zajil itself reads
# (ADR-0009). For the CI's video tests and for developers; prints the
# program's path. A zip (macOS and Windows) is kept as <dir>/<file> and
# unpacked into <dir>/<its name>; Zajil itself checks what it holds
# (tools.rs), and so do the tests that install it.
#
#   scripts/get-yt-dlp.sh <dir>
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
pins="$root/tools/yt-dlp/pins.txt"
dir="${1:?dir}"

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

version="$(awk '$1 == "version" { print $2 }' "$pins")"
# `|| true`: without a line for this machine, `read` fails, and the
# message below must still be said.
read -r file sha256 program < <(awk -v t="$os-$arch" '$1 == t { print $2, $3, $4 }' "$pins") || true
[ -n "${file:-}" ] || { echo "no yt-dlp is pinned for $os-$arch" >&2; exit 1; }

name="yt-dlp"
[ "$os" = windows ] && name="yt-dlp.exe"
case "$file" in *.zip) name="$file" ;; esac
mkdir -p "$dir"
curl --fail --silent --show-error --location --proto '=https' --tlsv1.2 \
  --retry 4 --retry-delay 5 --retry-all-errors \
  -o "$dir/$name.download" "https://github.com/yt-dlp/yt-dlp/releases/download/$version/$file"
# From stdin: with a file name, Git Bash's sha256sum on Windows puts a
# backslash before the hash of a path that holds one.
actual="$( (sha256sum < "$dir/$name.download" 2>/dev/null || shasum -a 256 < "$dir/$name.download") | cut -d' ' -f1)"
if [ "$actual" != "$sha256" ]; then
  rm -f "$dir/$name.download"
  echo "yt-dlp $version ($file) has SHA-256 $actual, not the pinned $sha256" >&2
  exit 1
fi
mv "$dir/$name.download" "$dir/$name"
case "$file" in
  *.zip)
    folder="$dir/${file%.zip}"
    rm -rf "$folder"
    if command -v unzip >/dev/null; then
      unzip -q "$dir/$name" -d "$folder"
    else
      powershell -NoProfile -Command "Expand-Archive -LiteralPath '$dir/$name' -DestinationPath '$folder'"
    fi
    # The zip stays next to its folder: the real video tests install it
    # through Zajil's own code, against the tree SHA-256 of pins.txt.
    chmod 700 "$folder/$program"
    echo "$folder/$program"
    ;;
  *)
    chmod 700 "$dir/$name"
    echo "$dir/$name"
    ;;
esac
