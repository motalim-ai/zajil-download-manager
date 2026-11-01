#!/bin/sh
# Rebuilds the protection lists the APK ships with
# (android/app/src/main/assets/blocklists/<list>.hosts, gzip-compressed) from sources.txt.
# Not «.gz»: the APK build takes that ending off and unpacks the file.
# Run by a maintainer before a release; the app updates them later only
# from the signed lists-android.json. Downloads go to a temporary folder;
# nothing in them runs.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
out="$here/../../android/app/src/main/assets/blocklists"
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
mkdir -p "$out"
for list in ads trackers malware; do
  n=0
  grep -v '^#' "$here/sources.txt" | while read -r name url _; do
    [ "$name" = "$list" ] || continue
    n=$((n + 1))
    curl -fsSL --max-time 120 -o "$work/$list-$n" "$url"
  done
  python3 -I "$here/compile.py" "$list" "$work/$list.txt" "$work/$list"-*
  gzip -9n < "$work/$list.txt" > "$out/$list.hosts"
done
date -u +%s > "$out/sequence.txt"
ls -l "$out"
