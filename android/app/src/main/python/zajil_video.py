"""Zajil's use of yt-dlp on Android (ADR-0012, section 6).

yt-dlp only finds a video's formats here; Zajil's own engine downloads them,
and Android's MediaMuxer joins the picture and the sound, as on the desktop.
Nothing is downloaded or written by this module, apart from what yt-dlp
keeps in memory, and it never runs a JavaScript runtime other than the
QuickJS-NG that the app ships ([qjs]).
"""

import json
import sys

# Fields of a format that the app needs, and no other.
FORMAT_FIELDS = (
    "format_id", "ext", "vcodec", "acodec", "height", "width", "fps", "tbr",
    "abr", "vbr", "filesize", "filesize_approx", "language", "language_preference",
    "format_note", "url", "http_headers", "protocol", "has_drm", "dynamic_range",
)

INFO_FIELDS = ("id", "title", "duration", "webpage_url", "extractor_key", "is_live", "live_status")


def formats(url, qjs, updated_yt_dlp=None, cache_dir=None):
    """The video at [url] and its formats, as JSON: {"error": ...} on failure.

    [updated_yt_dlp] is a newer yt-dlp (its zip, checked by the app against
    the signed tools list) to use instead of the one in the app.
    [cache_dir] is the app's own cache folder for yt-dlp: YouTube's player
    code and the answers to its challenges, which QuickJS-NG takes minutes
    to work out on a phone. Nothing of the user's is kept there.
    """
    if updated_yt_dlp and updated_yt_dlp not in sys.path and "yt_dlp" not in sys.modules:
        sys.path.insert(0, updated_yt_dlp)
    import yt_dlp

    options = {
        "quiet": True,
        "no_warnings": True,
        "skip_download": True,
        "noplaylist": True,
        # No plugins, nothing fetched from elsewhere: only the scripts in the
        # app (yt-dlp-ejs), run by the app's QuickJS-NG.
        "cachedir": cache_dir or False,
        "plugin_dirs": [],
        "remote_components": [],
        "js_runtimes": {"quickjs": {"path": qjs}},
        "socket_timeout": 20,
        "extractor_retries": 2,
    }
    try:
        with yt_dlp.YoutubeDL(options) as ydl:
            info = ydl.sanitize_info(ydl.extract_info(url, download=False))
    except Exception as e:  # yt-dlp's own errors say what went wrong
        return json.dumps({"error": str(e)[:500], "version": yt_dlp.version.__version__})
    found = []
    for f in info.get("formats") or []:
        if f.get("protocol") not in ("https", "http") or not f.get("url"):
            continue
        found.append({k: f.get(k) for k in FORMAT_FIELDS})
    result = {k: info.get(k) for k in INFO_FIELDS}
    result["formats"] = found
    result["version"] = yt_dlp.version.__version__
    return json.dumps(result)


def version(updated_yt_dlp=None):
    """The version of the yt-dlp that [formats] would use."""
    if updated_yt_dlp and updated_yt_dlp not in sys.path and "yt_dlp" not in sys.modules:
        sys.path.insert(0, updated_yt_dlp)
    import yt_dlp
    return yt_dlp.version.__version__
