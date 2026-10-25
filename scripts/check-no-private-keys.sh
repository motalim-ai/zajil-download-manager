#!/usr/bin/env bash
# Fails if the repository holds a private key (owner's decision on #7: the
# extension's private key never enters the repository). Looks at every file
# git tracks: key and certificate-store file names, and private-key text.
# Usage: scripts/check-no-private-keys.sh [repository folder]
set -euo pipefail
cd "${1:-.}"
found=0
while IFS= read -r -d '' file; do
  lower=$(printf '%s' "$file" | tr '[:upper:]' '[:lower:]')
  case "$lower" in
    *.pem|*.key|*.p8|*.p12|*.pfx|*.jks|*.keystore|*.ppk|id_rsa|*/id_rsa|id_ed25519|*/id_ed25519)
      echo "a key file is tracked: $file"; found=1 ;;
  esac
done < <(git ls-files -z)
if git grep -I -l -z -E -e '-----BEGIN ([A-Z0-9]+ )*PRIVATE KEY( BLOCK)?-----' >/dev/null 2>&1; then
  git grep -I -l -E -e '-----BEGIN ([A-Z0-9]+ )*PRIVATE KEY( BLOCK)?-----' | sed 's/^/private key text in: /'
  found=1
fi
if [ "$found" -ne 0 ]; then
  echo "Private keys must never be in the repository." >&2
  exit 1
fi
echo "no private key in the repository"
