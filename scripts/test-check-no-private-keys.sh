#!/usr/bin/env bash
# Shows that check-no-private-keys.sh refuses each way a private key can
# get in, and passes a clean repository. Every test key is made here, in a
# temporary folder, and never committed anywhere.
set -euo pipefail
check="$(cd "$(dirname "$0")" && pwd)/check-no-private-keys.sh"
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
cd "$work"
refused() { if "$check" . >/dev/null 2>&1; then echo "FAIL: $1 was not refused"; exit 1; fi; }
git init -q repo && cd repo && git config user.email t@example.invalid && git config user.name t
echo "hello" > README.md && git add README.md && git commit -q -m init
"$check" . >/dev/null || { echo "FAIL: a clean repository was refused"; exit 1; }
# The name alone: a .pem with no key text in it.
echo "not a key" > notes.pem && git add notes.pem && refused "a .pem file name"
git rm -q --cached notes.pem && rm notes.pem
# The text alone, in a file of another name: RSA (PKCS#8) and OpenSSH.
openssl genpkey -algorithm RSA -pkeyopt rsa_keygen_bits:2048 2>/dev/null > notes.txt
git add notes.txt && refused "PKCS#8 key text in notes.txt"
git rm -q --cached notes.txt && rm notes.txt
ssh-keygen -q -t ed25519 -N "" -f "$work/ssh" && cp "$work/ssh" deploy.txt
git add deploy.txt && refused "an OpenSSH key in deploy.txt"
git rm -q --cached deploy.txt && rm deploy.txt
# An update signing key (ADR-0011), as minisign writes it and as Tauri does
# (that text in base64).
printf 'untrusted comment: minisign encrypted secret %s\nRWRTY0IyAAAA\n' key > sign.txt
git add sign.txt && refused "a minisign secret key in sign.txt"
git rm -q --cached sign.txt && rm sign.txt
printf 'untrusted comment: rsign encrypted secret %s\nRWRTY0IyAAAA\n' key | base64 > zajil.txt
git add zajil.txt && refused "a Tauri update key in zajil.txt"
git rm -q --cached zajil.txt && rm zajil.txt
# A key committed, then removed: still in the history.
openssl genpkey -algorithm RSA -pkeyopt rsa_keygen_bits:2048 2>/dev/null > old.txt
git add old.txt && git commit -q -m "add" && git rm -q old.txt && git commit -q -m "remove"
refused "a key in the history"
# Not a git repository: it fails rather than say nothing was found.
cd "$work" && mkdir plain && cd plain
if "$check" . >/dev/null 2>&1; then echo "FAIL: a folder that is not a repository passed"; exit 1; fi
echo "check-no-private-keys: refuses a .pem name, PKCS#8, OpenSSH and update-key text, a key in the history, and a folder it cannot check; passes a clean repository"
