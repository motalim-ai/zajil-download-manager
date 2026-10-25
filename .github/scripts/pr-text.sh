#!/usr/bin/env bash
# Prints the numbers of the lines of stdin that name a forbidden word (one
# per line in the file $1, matched without regard to case), and fails if
# there is one. Text in backticks is left out: inline `code` and fenced
# ``` blocks, where a description may quote what the check looks for.
set -euo pipefail
words="$1"
text="$(perl -0pe 's/(```.*?```)/ $1 =~ s{[^\n]}{}gr /gse; s/`[^`\n]*`//g')"
lines="$(printf '%s\n' "$text" | grep -inF -f "$words" | cut -d: -f1 | paste -sd, - || true)"
if [ -n "$lines" ]; then
  echo "$lines"
  exit 1
fi
