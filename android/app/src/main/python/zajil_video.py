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

# The most entries of a playlist that are listed, as the desktop's
# (zajil-core video/output.rs, MAX_PLAYLIST_ENTRIES).
MAX_PLAYLIST_ENTRIES = 1000

INFO_FIELDS = (
    "id", "title", "duration", "webpage_url", "extractor_key", "is_live", "live_status",
    "thumbnail", "channel", "uploader",
)


def formats(url, qjs, updated_yt_dlp=None, cache_dir=None, cookies=None, referer=None):
    """The video at [url] and its formats, as JSON: {"error": ...} on failure.

    [updated_yt_dlp] is a newer yt-dlp (its zip, checked by the app against
    the signed tools list) to use instead of the one in the app.
    [cache_dir] is the app's own cache folder for yt-dlp: YouTube's player
    code and the answers to its challenges, which QuickJS-NG takes minutes
    to work out on a phone. Nothing of the user's is kept there.
    [cookies] is the in-app browser's `Cookie` header for [url] (the user
    signed in there): given to yt-dlp for [url]'s site only, in memory,
    never written. A format that needs cookies (some sites set their own
    while the page is read) says so in its `cookie_header`, for the app's
    engine to send to that format's address only, also in memory.
    [referer] is the page a playlist was found on, for its server.
    """
    if updated_yt_dlp and updated_yt_dlp not in sys.path and "yt_dlp" not in sys.modules:
        sys.path.insert(0, updated_yt_dlp)
    import yt_dlp

    options = {
        "quiet": True,
        "no_warnings": True,
        "skip_download": True,
        "noplaylist": True,
        # A playlist or a channel is listed, not opened: each video is read
        # when its turn comes, as on the desktop (--flat-playlist).
        "extract_flat": "in_playlist",
        "playlistend": MAX_PLAYLIST_ENTRIES,
        # No plugins, nothing fetched from elsewhere: only the scripts in the
        # app (yt-dlp-ejs), run by the app's QuickJS-NG.
        "cachedir": cache_dir or False,
        "plugin_dirs": [],
        "remote_components": [],
        "js_runtimes": {"quickjs": {"path": qjs}},
        "socket_timeout": 20,
        "extractor_retries": 2,
    }
    if referer:
        options["http_headers"] = {"Referer": referer}
    try:
        with yt_dlp.YoutubeDL(options) as ydl:
            _add_cookies(ydl, url, cookies)
            url = _snapchat_spotlight(ydl, url)
            info = ydl.sanitize_info(ydl.extract_info(url, download=False))
            if info.get("_type") == "playlist":
                return json.dumps(_playlist(info, yt_dlp.version.__version__))
            found = []
            # A page of one file (Snapchat's Spotlight) has its address on
            # the video itself, no list of formats.
            listed = info.get("formats") or ([dict(info, format_id=info.get("format_id") or "0")] if info.get("url") else [])
            for f in listed:
                if not f.get("url"):
                    continue
                if f.get("protocol") in ("https", "http"):
                    entry = {k: f.get(k) for k in FORMAT_FIELDS}
                    entry["cookie_header"] = ydl.cookiejar.get_cookie_header(f["url"]) or None
                elif f.get("protocol") in ("m3u8_native", "m3u8"):
                    # HLS: yt-dlp downloads it itself ([download_hls]).
                    entry = {k: f.get(k) for k in FORMAT_FIELDS}
                    entry["hls"] = True
                elif f.get("protocol") in ("http_dash_segments", "http_dash_segments_generator"):
                    # DASH: its pieces too ([download_hls]), one stream each, joined by the app.
                    entry = {k: f.get(k) for k in FORMAT_FIELDS}
                    entry["dash"] = True
                else:
                    continue
                found.append(entry)
    except Exception as e:  # yt-dlp's own errors say what went wrong
        return json.dumps({"error": str(e)[:500], "version": yt_dlp.version.__version__})
    result = {k: info.get(k) for k in INFO_FIELDS}
    result["formats"] = found
    result["version"] = yt_dlp.version.__version__
    return json.dumps(result)


def _playlist(info, version):
    """A playlist yt-dlp listed: its title, and for each entry its page,
    title, length and smallest https thumbnail of at least 120 pixels.
    An entry that is itself a playlist (a channel's tab) says so."""
    entries = []
    for e in (info.get("entries") or [])[:MAX_PLAYLIST_ENTRIES]:
        if not isinstance(e, dict):
            continue
        url = e.get("url") or e.get("webpage_url")
        if e.get("ie_key") == "Youtube" and e.get("id") and not str(url or "").startswith("http"):
            url = "https://www.youtube.com/watch?v=" + e["id"]
        thumbs = [t for t in (e.get("thumbnails") or []) if str(t.get("url", "")).startswith("https://")]
        thumbs.sort(key=lambda t: t.get("width") or 0)
        thumb = next((t["url"] for t in thumbs if (t.get("width") or 0) >= 120), thumbs[-1]["url"] if thumbs else None)
        entries.append({
            "url": url if str(url or "").startswith(("https://", "http://")) else None,
            "title": e.get("title"),
            "duration": e.get("duration"),
            "thumbnail": thumb,
            "playlist": e.get("_type") == "playlist" or e.get("ie_key") == "YoutubeTab",
            "available": e.get("availability") not in ("private", "needs_auth", "subscriber_only", "premium_only")
            and (e.get("title") or "") not in ("[Private video]", "[Deleted video]"),
        })
    return {
        "playlist": True,
        "title": info.get("title") or info.get("id") or "",
        "channel": info.get("channel") or info.get("uploader"),
        "entries": entries,
        "version": version,
    }


def version(updated_yt_dlp=None):
    """The version of the yt-dlp that [formats] would use."""
    if updated_yt_dlp and updated_yt_dlp not in sys.path and "yt_dlp" not in sys.modules:
        sys.path.insert(0, updated_yt_dlp)
    import yt_dlp
    return yt_dlp.version.__version__


def _snapchat_spotlight(ydl, url):
    """A Snapchat Spotlight link as yt-dlp knows it, snapchat.com/spotlight/ID:
    a shared short link (t.snapchat.com/X, snapchat.com/t/X) is followed
    first, by yt-dlp's own connection, and «/@user/spotlight/ID» is the same
    video. Any other link is returned as it is."""
    import re
    from urllib.parse import urlsplit
    parts = urlsplit(url)
    host = (parts.hostname or "").lower()
    if host not in ("snapchat.com", "www.snapchat.com", "t.snapchat.com"):
        return url
    if host == "t.snapchat.com" or parts.path.startswith("/t/"):
        try:
            from yt_dlp.networking import Request
            with ydl.urlopen(Request(url, method="HEAD")) as response:
                url = response.url
        except Exception:
            return url
    spotlight = re.search(r"^https://(?:www\.)?snapchat\.com/(?:@[\w.-]+/)?spotlight/([\w-]+)", url)
    return f"https://www.snapchat.com/spotlight/{spotlight.group(1)}" if spotlight else url


def _site(url):
    """[url]'s host without «www.» or «m.»: the cookies' domain."""
    from urllib.parse import urlsplit
    host = (urlsplit(url).hostname or "").lower()
    for prefix in ("www.", "m.", "mobile."):
        if host.startswith(prefix):
            return host[len(prefix):]
    return host


def _add_cookies(ydl, url, cookies):
    """The browser's [cookies] (a `Cookie` header) for [url]'s site only."""
    if not cookies:
        return
    import http.cookiejar
    site = _site(url)
    if not site:
        return
    for part in cookies.split(";"):
        name, sep, value = part.strip().partition("=")
        if not sep or not name:
            continue
        ydl.cookiejar.set_cookie(http.cookiejar.Cookie(
            0, name, value, None, False, "." + site, True, True, "/", True,
            True, None, True, None, None, {},
        ))


class _Stopped(Exception):
    pass


def download_hls(url, format_id, out_dir, listener, updated_yt_dlp=None, cookies=None, referer=None, name="stream"):
    """Downloads the HLS or DASH format [format_id] of [url] into [out_dir]
    as [name].<ext>, with yt-dlp's own fragment downloader (no FFmpeg: HLS
    fragments are joined as MPEG-TS, which the app turns into MP4 with
    Android's MediaMuxer; DASH's are fragmented MP4, joined by it too).
    Again it continues from the fragments already there. [listener] is the
    app's: `progress(done, total)` and `stopping()`. Returns JSON:
    {"file": path}, {"stopped": true} or {"error": ...}.
    """
    if updated_yt_dlp and updated_yt_dlp not in sys.path and "yt_dlp" not in sys.modules:
        sys.path.insert(0, updated_yt_dlp)
    import os
    import yt_dlp

    def hook(d):
        if listener.stopping():
            raise _Stopped()
        done = d.get("downloaded_bytes") or 0
        total = d.get("total_bytes") or d.get("total_bytes_estimate") or 0
        listener.progress(int(done), int(total))

    options = {
        "quiet": True,
        "no_warnings": True,
        "noprogress": True,
        "noplaylist": True,
        "cachedir": False,
        "plugin_dirs": [],
        "remote_components": [],
        "format": format_id,
        "outtmpl": {"default": os.path.join(out_dir, name + ".%(ext)s")},
        "hls_use_mpegts": True,
        "continuedl": True,
        "fixup": "never",
        "postprocessors": [],
        "progress_hooks": [hook],
        "socket_timeout": 20,
        "retries": 5,
        "fragment_retries": 5,
    }
    if referer:
        options["http_headers"] = {"Referer": referer}
    try:
        with yt_dlp.YoutubeDL(options) as ydl:
            _add_cookies(ydl, url, cookies)
            info = ydl.extract_info(url, download=True)
            files = [d.get("filepath") for d in info.get("requested_downloads") or [] if d.get("filepath")]
    except Exception as e:
        if listener.stopping() or isinstance(e, _Stopped) or "_Stopped" in repr(e):
            return json.dumps({"stopped": True})
        return json.dumps({"error": str(e)[:500]})
    if not files:
        return json.dumps({"error": "yt-dlp wrote no file"})
    return json.dumps({"file": files[0]})
