#!/usr/bin/env bash
# Runs the ffmpeg and ffprobe that Zajil distributes on what they are for
# (ADR-0009), with media made by the test-only build:
# - checks the build: LGPL, no GPL or non-free option, no encoder but
#   aac, no protocol but file and pipe;
# - merges a separate video and audio track without re-encoding;
# - remuxes fragmented-MP4 HLS and MPEG-TS audio, as yt-dlp's fixups do;
# - turns Opus into audio-only m4a with FFmpeg's own AAC encoder.
#
#   tools/ffmpeg/smoke-test.sh <dist dir> <testgen dir>
set -euo pipefail

dist="$(cd "${1:?dist dir}" && pwd)"
testgen="$(cd "${2:?testgen dir}" && pwd)"
exe=""
[ -f "$dist/ffmpeg.exe" ] && exe=".exe"
ff="$dist/ffmpeg$exe"
fp="$dist/ffprobe$exe"
gen="$testgen/ffmpeg-testgen$exe"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
cd "$work"

fail() { echo "::error::$*" >&2; exit 1; }
# The output of a tool, without Windows line ends.
run() { "$@" | tr -d '\r'; }

# The build itself.
conf="$(run "$ff" -hide_banner -buildconf)"
# The half that runs, when the caller says which (macOS under Rosetta).
if [ -n "${ZAJIL_EXPECT_ARCH:-}" ]; then
  grep -q -- "--arch=$ZAJIL_EXPECT_ARCH" <<< "$conf" || fail "the $ZAJIL_EXPECT_ARCH half did not run"
fi
for bad in --enable-gpl --enable-nonfree --enable-version3 --enable-lib; do
  case "$conf" in *"$bad"*) fail "the distributed ffmpeg was built with $bad" ;; esac
done
license="$(run "$ff" -hide_banner -L)"
grep -q 'GNU Lesser General Public' <<< "$license" || fail "ffmpeg -L does not say LGPL"
grep -q '^version 2.1 of the License, or (at your option) any later version.$' <<< "$license" ||
  fail "ffmpeg -L does not say LGPL version 2.1 or later"
# The list after its legend (" V..... = Video" and so on).
encoders="$(run "$ff" -hide_banner -encoders | awk '/^ -+$/ { body = 1; next } body { print $2 }' | sort | tr '\n' ' ')"
[ "$encoders" = "aac " ] || fail "the distributed ffmpeg has encoders other than aac: $encoders"
protocols="$(run "$ff" -hide_banner -protocols | awk 'NF == 1 && !/:/ { print $1 }' | sort -u | tr '\n' ' ')"
[ "$protocols" = "file pipe " ] || fail "the distributed ffmpeg has protocols other than file and pipe: $protocols"

duration() { run "$fp" -v error -show_entries format=duration -of default=nw=1:nk=1 "$1"; }
codecs() { run "$fp" -v error -show_entries stream=codec_name -of default=nw=1:nk=1 "$1" | tr '\n' ' '; }
near4() { awk -v d="$1" 'BEGIN { exit !(d > 3.9 && d < 4.2) }' || fail "$2 lasts $1 s, not about 4 s"; }

# Media, from the test-only build.
"$gen" -hide_banner -loglevel error -filter_complex \
  "testsrc=duration=4:size=320x240:rate=25,format=yuv420p[v];sine=duration=4:frequency=440[a]" \
  -map "[v]" -map "[a]" -c:v mpeg4 -c:a aac clip.mp4
"$gen" -hide_banner -loglevel error -i clip.mp4 -map 0:v -c copy video.mp4
"$gen" -hide_banner -loglevel error -i clip.mp4 -map 0:a -c copy audio.m4a
"$gen" -hide_banner -loglevel error -filter_complex "sine=duration=4" -c:a opus -strict experimental audio.ogg
mkdir hls audio-hls
"$gen" -hide_banner -loglevel error -i clip.mp4 -c copy -f hls -hls_time 1 -hls_playlist_type vod \
  -hls_segment_type fmp4 hls/index.m3u8
"$gen" -hide_banner -loglevel error -i audio.m4a -c copy -f hls -hls_time 1 -hls_playlist_type vod audio-hls/a.m3u8

# 1. A separate video and audio track, merged without re-encoding.
"$ff" -hide_banner -loglevel error -i video.mp4 -i audio.m4a -map 0:v -map 1:a -c copy -movflags +faststart merged.mp4
[ "$(codecs merged.mp4)" = "mpeg4 aac " ] || fail "merged.mp4 has streams $(codecs merged.mp4)"
near4 "$(duration merged.mp4)" merged.mp4

# 2. Fragmented-MP4 HLS segments joined, then remuxed.
cat hls/init.mp4 hls/index*.m4s > joined.mp4
"$ff" -hide_banner -loglevel error -i joined.mp4 -c copy fixed.mp4
[ "$(codecs fixed.mp4)" = "mpeg4 aac " ] || fail "fixed.mp4 has streams $(codecs fixed.mp4)"
near4 "$(duration fixed.mp4)" fixed.mp4

# 3. MPEG-TS audio segments joined, then remuxed to m4a.
cat audio-hls/a*.ts > joined.ts
"$ff" -hide_banner -loglevel error -i joined.ts -c copy -bsf:a aac_adtstoasc fixed.m4a
[ "$(codecs fixed.m4a)" = "aac " ] || fail "fixed.m4a has streams $(codecs fixed.m4a)"
near4 "$(duration fixed.m4a)" fixed.m4a

# 4. Opus to audio-only m4a, with FFmpeg's own AAC encoder.
"$ff" -hide_banner -loglevel error -i audio.ogg -vn -c:a aac -b:a 128k out.m4a
[ "$(codecs out.m4a)" = "aac " ] || fail "out.m4a has streams $(codecs out.m4a)"
near4 "$(duration out.m4a)" out.m4a

echo "smoke test passed: $("$ff" -hide_banner -version | head -n 1 | tr -d '\r')"
