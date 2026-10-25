#!/usr/bin/env bash
# Checks what the built ffmpeg and ffprobe themselves load (ADR-0009,
# checklist items 1, 17 and 18: no external library). configure's own
# lists are checked by build.sh; this reads the real files: every library
# they need must be the system's own C library or a Windows system DLL.
# Prints what each file needs, for the record.
#
#   tools/ffmpeg/check-linked.sh <dir with ffmpeg and ffprobe>
set -euo pipefail

dir="${1:?dir}"
fail() { echo "::error::$*" >&2; exit 1; }

case "$(uname -s)" in
  Linux)
    # glibc's own parts only.
    allowed='^(libc|libm|libpthread|libdl|librt)\.so\.[0-9]+$|^ld-linux[-a-z0-9_.]*\.so\.[0-9]+$'
    for f in "$dir/ffmpeg" "$dir/ffprobe"; do
      # A failing readelf stops here (set -e); one with no NEEDED line read
      # a static program, which needs nothing.
      deps="$(readelf -d "$f")"
      if ! grep -q '(NEEDED)' <<< "$deps"; then
        echo "$(basename "$f") needs: nothing (statically linked)"
        continue
      fi
      needed="$(sed -n 's/.*(NEEDED).*\[\(.*\)\]/\1/p' <<< "$deps")"
      echo "$(basename "$f") needs: $(tr '\n' ' ' <<< "$needed")"
      while IFS= read -r lib; do
        grep -qE "$allowed" <<< "$lib" || fail "$f needs $lib, which is not the system's C library"
      done <<< "$needed"
    done
    ;;
  Darwin)
    # libSystem only, for every architecture of a universal file.
    for f in "$dir/ffmpeg" "$dir/ffprobe"; do
      deps="$(otool -arch all -L "$f")"
      grep -q '/usr/lib/libSystem.B.dylib' <<< "$deps" || fail "otool could not read the libraries of $f"
      needed="$(grep -E '^[[:space:]]+/' <<< "$deps" | awk '{ print $1 }' | sort -u)"
      echo "$(basename "$f") needs: $(tr '\n' ' ' <<< "$needed")"
      while IFS= read -r lib; do
        [ "$lib" = /usr/lib/libSystem.B.dylib ] || fail "$f needs $lib, which is not libSystem"
      done <<< "$needed"
    done
    ;;
  MINGW*|MSYS*|CYGWIN*)
    # Windows' own DLLs, from System32; never one from MinGW or MSYS2.
    allowed='^(kernel32|user32|advapi32|shell32|bcrypt|ole32|ws2_32|msvcrt|ucrtbase|api-ms-win-[a-z0-9-]+)\.dll$'
    dump="$(command -v objdump || command -v llvm-objdump)" || fail "no objdump"
    for f in "$dir/ffmpeg.exe" "$dir/ffprobe.exe"; do
      deps="$("$dump" -p "$f")"
      grep -qiE 'DLL Name: KERNEL32\.dll' <<< "$deps" || fail "$dump could not read the imports of $f"
      needed="$(sed -n 's/.*DLL Name: *//p' <<< "$deps" | tr -d '\r' | sort -u)"
      echo "$(basename "$f") needs: $(tr '\n' ' ' <<< "$needed")"
      while IFS= read -r lib; do
        grep -qiE "$allowed" <<< "$lib" || fail "$f needs $lib, which is not a Windows system DLL"
      done <<< "$needed"
    done
    ;;
  *) fail "unknown system $(uname -s)" ;;
esac
echo "linked libraries: only the system's own"
