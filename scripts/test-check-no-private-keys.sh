#!/usr/bin/env bash
# Shows that check-no-private-keys.sh fails on a private key, whether named
# .pem or hidden in another file, and passes on a clean repository. The test
# key is made here, in a temporary folder, and never committed anywhere.
set -euo pipefail
check="$(cd "$(dirname "$0")" && pwd)/check-no-private-keys.sh"
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
cd "$work"
git init -q . && git config user.email t@example.invalid && git config user.name t
echo "hello" > README.md && git add README.md
"$check" . >/dev/null || { echo "FAIL: a clean repository was refused"; exit 1; }
openssl genpkey -algorithm RSA -pkeyopt rsa_keygen_bits:2048 -out test.pem 2>/dev/null
git add test.pem
if "$check" . >/dev/null 2>&1; then echo "FAIL: a .pem was not refused"; exit 1; fi
git rm -q --cached test.pem
cp test.pem notes.txt && rm test.pem && git add notes.txt
if "$check" . >/dev/null 2>&1; then echo "FAIL: private key text was not refused"; exit 1; fi
git rm -q --cached notes.txt && rm notes.txt
openssl genpkey -algorithm ED25519 2>/dev/null > id_ed25519 && git add id_ed25519
if "$check" . >/dev/null 2>&1; then echo "FAIL: an ssh key was not refused"; exit 1; fi
echo "check-no-private-keys: refuses a .pem, key text in another file and an ssh key; passes a clean repository"
