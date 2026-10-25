#!/usr/bin/env bash
# Downloads the pinned QuickJS-NG (qjs) for this machine into <dir> and
# checks its SHA-256 against tools/quickjs/pins.txt, the file Zajil itself
# reads (ADR-0009). For the CI's zajil-jsbox tests and for developers;
# prints the program's path.
#
#   scripts/get-qjs.sh <dir>
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
pins="$root/tools/quickjs/pins.txt"
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
read -r file sha256 < <(awk -v t="$os-$arch" '$1 == t { print $2, $3 }' "$pins")
[ -n "${file:-}" ] || { echo "no QuickJS-NG is pinned for $os-$arch" >&2; exit 1; }

name="qjs"
[ "$os" = windows ] && name="qjs.exe"
mkdir -p "$dir"
curl --fail --silent --show-error --location --proto '=https' --tlsv1.2 \
  --retry 4 --retry-delay 5 --retry-all-errors \
  -o "$dir/$name.download" "https://github.com/quickjs-ng/quickjs/releases/download/$version/$file"
# From stdin: with a file name, Git Bash's sha256sum on Windows puts a
# backslash before the hash of a path that holds one.
actual="$( (sha256sum < "$dir/$name.download" 2>/dev/null || shasum -a 256 < "$dir/$name.download") | cut -d' ' -f1)"
if [ "$actual" != "$sha256" ]; then
  rm -f "$dir/$name.download"
  echo "QuickJS-NG $version ($file) has SHA-256 $actual, not the pinned $sha256" >&2
  exit 1
fi
mv "$dir/$name.download" "$dir/$name"
chmod 700 "$dir/$name"
echo "$dir/$name"
