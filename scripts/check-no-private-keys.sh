#!/usr/bin/env bash
# Fails if the repository holds a private key (owner's decision on #7: the
# extension's private key never enters the repository; ADR-0011: nor the
# update's signing keys): a key or
# certificate-store file name among the tracked files, or private-key text
# in a tracked file or anywhere in the history (a key added, then removed,
# is still there). Fails too when it cannot look (not a git repository).
# Usage: scripts/check-no-private-keys.sh [repository folder]
set -euo pipefail
cd "${1:-.}"
git rev-parse --git-dir >/dev/null 2>&1 || { echo "not a git repository: cannot check for private keys" >&2; exit 2; }
armor='-----BEGIN ([A-Z0-9]+ )*PRIVATE KEY( BLOCK)?-----'
# Zajil's update keys (ADR-0011): a minisign or Tauri secret key, as text or
# in Tauri's base64 (of "untrusted comment: rsign/minisign encrypted
# secret key"). Written so that this file does not match itself.
armor="$armor|untrusted comment: (rsign|minisign) encrypted secret[ ]key|dW50cnVzdGVkIGNvbW1lbnQ6IHJzaWduIGVuY3J5cHRlZCBzZWNyZXQga2V[5]|dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIGVuY3J5cHRlZCBzZWNyZXQga2V[5]"
found=0
files=$(git ls-files -z | tr '\0' '\n') || { echo "cannot list the tracked files" >&2; exit 2; }
while IFS= read -r file; do
  [ -n "$file" ] || continue
  lower=$(printf '%s' "$file" | tr '[:upper:]' '[:lower:]')
  case "$lower" in
    *.pem|*.key|*.der|*.p8|*.pk8|*.p12|*.pfx|*.jks|*.keystore|*.ppk|*.crx|id_rsa|*/id_rsa|id_ecdsa|*/id_ecdsa|id_ed25519|*/id_ed25519)
      echo "a key file is tracked: $file"; found=1 ;;
  esac
done <<< "$files"
set +e
in_tree=$(git grep -I -l -E -e "$armor"); tree_status=$?
set -e
if [ "$tree_status" -eq 0 ]; then
  printf '%s\n' "$in_tree" | sed 's/^/private key text in: /'; found=1
elif [ "$tree_status" -ne 1 ]; then
  echo "cannot search the tracked files" >&2; exit 2
fi
# The whole history, every branch and tag.
# (Read whole first: with pipefail, `grep -q` stopping early would make the
# pipeline fail and hide what it found.)
history=$(git log --all -p --no-color --no-textconv) || { echo "cannot read the history" >&2; exit 2; }
if grep -E -q -e "^[+-]$armor" <<< "$history"; then
  echo "private key text in the history (a commit added it, even if a later one removed it)"; found=1
fi
if [ "$found" -ne 0 ]; then
  echo "Private keys must never be in the repository." >&2
  exit 1
fi
echo "no private key in the repository or its history"
