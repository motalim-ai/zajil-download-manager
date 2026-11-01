package io.github.motalim_ai.zajil.ui

import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.StrokeJoin
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.graphics.vector.addPathNodes
import androidx.compose.ui.unit.dp

/**
 * The D5 design's one-colour icons, on a 24×24 grid: strokes in the
 * Lucide style. The gear, languages and bolt are Lucide's own paths, from
 * the design's files (ISC; its licence in assets/licenses,
 * docs/DEPENDENCIES.md); the others are drawn for Zajil. Black, tinted by `Icon`.
 * Arrows and chevrons are drawn pointing as in English and mirrored for
 * Arabic: «back» points right there, a row's chevron left.
 */
enum class LineIcon {
    // The bar and the header.
    Downloads, Queues, Settings, Globe, Plus, Languages, Moon, Sun,

    // Arrows (mirrored right to left).
    Back, Chevron, ArrowBack, ArrowForward, SwipeHint,

    // Actions.
    Search, More, MoreVertical, Close, Check, ChevronDown, Minus, Reload, Home, Info, Pause, PlayFill, Download, Share,

    // Kinds of file.
    File, Video, Music, Disc, Archive, Image, App,

    // Rows and tiles.
    Clipboard, Playlist, Queue, Shield, ShieldAlert, Link, Folder, Plug, Bolt, BoltFill, Wifi, AudioBars, Subtitles,
    Ad, Malware, Phish, Clock, Lock, LockOpen, Private, Star, StarFilled, History, Tabs, Update,
}

/** One drawing: stroked unless [fill]; [width] its stroke. */
private class Look(val d: String, val width: Float = 1.9f, val fill: Boolean = false, val mirror: Boolean = false)

private fun circle(cx: Float, cy: Float, r: Float) =
    "M${cx - r},${cy}a$r,$r 0 1,0 ${2 * r},0a$r,$r 0 1,0 ${-2 * r},0z"

private fun rect(x: Float, y: Float, w: Float, h: Float, rx: Float) =
    "M${x + rx},${y}h${w - 2 * rx}a$rx,$rx 0 0 1 $rx,${rx}v${h - 2 * rx}a$rx,$rx 0 0 1 ${-rx},${rx}" +
        "h${-(w - 2 * rx)}a$rx,$rx 0 0 1 ${-rx},${-rx}v${-(h - 2 * rx)}a$rx,$rx 0 0 1 $rx,${-rx}z"

private const val SHIELD = "M12 3l7 3v5c0 4.5-3 8.3-7 10-4-1.7-7-5.5-7-10V6z"

private fun look(icon: LineIcon): List<Look> = when (icon) {
    LineIcon.Downloads, LineIcon.Download -> listOf(Look("M12 4v11M7 10l5 5 5-5M5 19h14", 2f))
    LineIcon.Queues -> listOf(Look("M4 6h12M4 12h12M4 18h8" + circle(18f, 18f, 3f), 1.8f))
    LineIcon.Settings -> listOf(
        Look(
            "M12.22 2h-.44a2 2 0 0 0-2 2v.18a2 2 0 0 1-1 1.73l-.43.25a2 2 0 0 1-2 0l-.15-.08a2 2 0 0 0-2.73.73l-.22.38a2 2 0 0 0 .73 2.73l.15.1a2 2 0 0 1 1 1.72v.51a2 2 0 0 1-1 1.74l-.15.09a2 2 0 0 0-.73 2.73l.22.38a2 2 0 0 0 2.73.73l.15-.08a2 2 0 0 1 2 0l.43.25a2 2 0 0 1 1 1.73V20a2 2 0 0 0 2 2h.44a2 2 0 0 0 2-2v-.18a2 2 0 0 1 1-1.73l.43-.25a2 2 0 0 1 2 0l.15.08a2 2 0 0 0 2.73-.73l.22-.39a2 2 0 0 0-.73-2.73l-.15-.08a2 2 0 0 1-1-1.74v-.5a2 2 0 0 1 1-1.74l.15-.09a2 2 0 0 0 .73-2.73l-.22-.38a2 2 0 0 0-2.73-.73l-.15.08a2 2 0 0 1-2 0l-.43-.25a2 2 0 0 1-1-1.73V4a2 2 0 0 0-2-2z" +
                circle(12f, 12f, 3f),
            1.8f,
        ),
    )
    LineIcon.Globe -> listOf(Look(circle(12f, 12f, 9f) + "M3 12h18M12 3c2.5 2.6 3.8 5.6 3.8 9s-1.3 6.4-3.8 9c-2.5-2.6-3.8-5.6-3.8-9S9.5 5.6 12 3z", 1.8f))
    LineIcon.Plus -> listOf(Look("M12 5v14M5 12h14", 2.3f))
    LineIcon.Minus -> listOf(Look("M5 12h14", 2.3f))
    LineIcon.Languages -> listOf(Look("M5 8l6 6M4 14l6-6 2-3M2 5h12M7 2h1M22 22l-5-10-5 10M14 18h6"))
    LineIcon.Moon -> listOf(Look("M12 3a6 6 0 0 0 9 9 9 9 0 1 1-9-9z", fill = true))
    LineIcon.Sun -> listOf(
        Look(circle(12f, 12f, 4.5f), fill = true),
        Look("M12 1.5v2.5M12 20v2.5M1.5 12h2.5M20 12h2.5M4.6 4.6l1.8 1.8M17.6 17.6l1.8 1.8M4.6 19.4l1.8-1.8M17.6 6.4l1.8-1.8"),
    )
    LineIcon.Back -> listOf(Look("M15 5l-7 7 7 7", mirror = true))
    LineIcon.Chevron -> listOf(Look("M9 5l7 7-7 7", mirror = true))
    LineIcon.ArrowBack -> listOf(Look("M19 12H5M11 6l-6 6 6 6", mirror = true))
    LineIcon.ArrowForward -> listOf(Look("M5 12h14M13 6l6 6-6 6", mirror = true))
    LineIcon.SwipeHint -> listOf(Look("M4 8l4 4-4 4M10 8l4 4-4 4M16 8l4 4-4 4", 1.7f, mirror = true))
    LineIcon.Search -> listOf(Look(circle(11f, 11f, 7f) + "M20 20l-3.5-3.5"))
    LineIcon.More -> listOf(Look(circle(5f, 12f, 1.6f) + circle(12f, 12f, 1.6f) + circle(19f, 12f, 1.6f), fill = true))
    LineIcon.MoreVertical -> listOf(Look(circle(12f, 5f, 1.6f) + circle(12f, 12f, 1.6f) + circle(12f, 19f, 1.6f), fill = true))
    LineIcon.Close -> listOf(Look("M6 6l12 12M18 6L6 18"))
    LineIcon.Check -> listOf(Look("M5 12.5l4.5 4.5L19 7.5", 2.4f))
    LineIcon.ChevronDown -> listOf(Look("M6 9l6 6 6-6", 2.2f))
    LineIcon.Reload -> listOf(Look("M20 11a8 8 0 1 0-2.3 5.7M20 5v6h-6"))
    LineIcon.Home -> listOf(Look("M4 11l8-7 8 7v9H4z"))
    LineIcon.Info -> listOf(Look(circle(12f, 12f, 9f) + "M12 11v6M12 7.5v.5"))
    LineIcon.Pause -> listOf(Look(rect(6f, 5f, 4f, 14f, 1.2f) + rect(14f, 5f, 4f, 14f, 1.2f), fill = true))
    LineIcon.PlayFill -> listOf(Look("M8 5v14l11-7z", fill = true))
    LineIcon.Share -> listOf(Look("M4 12v7a1 1 0 0 0 1 1h14a1 1 0 0 0 1-1v-7M12 3v12M8 7l4-4 4 4"))
    LineIcon.File -> listOf(Look("M7 3h7l4 4v14H7zM14 3v4h4"))
    LineIcon.Video -> listOf(Look("M4 6h12v12H4zM16 10l4-2v8l-4-2"))
    LineIcon.Music -> listOf(Look("M9 18V5l11-2v13M9 18a3 3 0 1 1-6 0 3 3 0 0 1 6 0zM20 16a3 3 0 1 1-6 0 3 3 0 0 1 6 0z"))
    LineIcon.Disc -> listOf(Look(circle(12f, 12f, 9f) + circle(12f, 12f, 2f)))
    LineIcon.Archive -> listOf(Look("M6 3h12v18H6zM12 3v3M12 8v2M12 12v2"))
    LineIcon.Image -> listOf(Look(rect(3f, 4f, 18f, 16f, 2f) + circle(9f, 10f, 1.8f) + "M21 16l-5-5-9 9"))
    LineIcon.App -> listOf(Look(rect(6f, 2f, 12f, 20f, 2.5f) + "M11 18h2"))
    LineIcon.Clipboard -> listOf(Look("M9 3h6v4H9zM16 5h2a2 2 0 0 1 2 2v12a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2V7a2 2 0 0 1 2-2h2", 1.7f))
    LineIcon.Playlist -> listOf(Look("M4 6h11M4 11h11M4 16h7M17 14v6l4-3z", 1.7f))
    LineIcon.Queue -> listOf(Look("M4 6h12M4 12h12M4 18h8", 1.7f))
    LineIcon.Shield -> listOf(Look(SHIELD, 1.7f))
    // A harmful site: the shield with «!».
    LineIcon.ShieldAlert -> listOf(Look("${SHIELD}M12 8.5v4.5", 2.2f), Look(circle(12f, 16.2f, 1.25f), fill = true))
    LineIcon.Link -> listOf(Look("M10 14a4 4 0 0 0 5.7 0l3-3a4 4 0 0 0-5.7-5.7l-1 1M14 10a4 4 0 0 0-5.7 0l-3 3a4 4 0 0 0 5.7 5.7l1-1"))
    LineIcon.Folder -> listOf(Look("M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z"))
    LineIcon.Plug -> listOf(Look("M8 3v5M16 3v5M6 8h12v3a6 6 0 0 1-12 0zM12 17v4"))
    LineIcon.Bolt -> listOf(Look("M13 3L5 14h6l-1 7 8-11h-6z"))
    LineIcon.BoltFill -> listOf(
        Look(
            "M4 14a1 1 0 0 1-.78-1.63l9.9-10.2a.5.5 0 0 1 .86.46l-1.92 6.02A1 1 0 0 0 13 10h7a1 1 0 0 1 .78 1.63l-9.9 10.2a.5.5 0 0 1-.86-.46l1.92-6.02A1 1 0 0 0 11 14z",
            fill = true,
        ),
    )
    LineIcon.Wifi -> listOf(Look("M5 12.5a10 10 0 0 1 14 0M8.5 16a5 5 0 0 1 7 0M12 19.5h.01"))
    LineIcon.AudioBars -> listOf(Look("M4 10v4M8 7v10M12 4v16M16 7v10M20 10v4"))
    LineIcon.Subtitles -> listOf(Look("M4 6h16v12H4zM7 13h4M13 13h4M7 16h7"))
    LineIcon.Ad -> listOf(Look("M4 5h16v12H4zM8 21h8M9 9l2 4 2-4M15 9v4"))
    LineIcon.Malware -> listOf(Look("M12 3a8 8 0 0 0-5 14.2V21h10v-3.8A8 8 0 0 0 12 3zM9 11h.01M15 11h.01M10 21v-3M14 21v-3"))
    LineIcon.Phish -> listOf(Look("M12 3v9a4 4 0 1 1-4-4M12 3l4 3"))
    LineIcon.Clock -> listOf(Look(circle(12f, 12f, 9f) + "M12 7v5l3 2", 2f))
    LineIcon.Lock -> listOf(Look("M5 11h14v10H5zM8 11V8a4 4 0 0 1 8 0v3", 2f))
    // http: the shackle open.
    LineIcon.LockOpen -> listOf(Look("M5 11h14v10H5zM8 11V7a4 4 0 0 1 7.6-1.7", 2f))
    LineIcon.Private -> listOf(
        Look("M3 3l18 18M10.6 6.1A10 10 0 0 1 12 6c5 0 9 4.5 10 6-.4.7-1.3 1.9-2.6 3.1M6.6 6.6C4.4 8 2.9 10.4 2 12c1 1.5 5 6 10 6 1.8 0 3.4-.6 4.8-1.4M9.9 9.9a3 3 0 0 0 4.2 4.2"),
    )
    LineIcon.Star -> listOf(Look("M12 3.5l2.6 5.3 5.9.9-4.25 4.15 1 5.85L12 16.95 6.75 19.7l1-5.85L3.5 9.7l5.9-.9z"))
    LineIcon.StarFilled -> listOf(Look("M12 3.5l2.6 5.3 5.9.9-4.25 4.15 1 5.85L12 16.95 6.75 19.7l1-5.85L3.5 9.7l5.9-.9z", fill = true))
    LineIcon.History -> listOf(Look(circle(12f, 12f, 9f) + "M12 7v5l3 2M3 12H1M3 12l2-2"))
    LineIcon.Tabs -> listOf(Look(rect(5f, 5f, 14f, 14f, 3.5f), 2f))
    LineIcon.Update -> listOf(Look("M12 4v10M7 9l5 5 5-5M5 20h14"))
}

private val cache = HashMap<LineIcon, ImageVector>()

fun lineIcon(icon: LineIcon): ImageVector = synchronized(cache) {
    cache.getOrPut(icon) {
        val parts = look(icon)
        val ink = SolidColor(Color.Black)
        ImageVector.Builder(
            defaultWidth = 24.dp,
            defaultHeight = 24.dp,
            viewportWidth = 24f,
            viewportHeight = 24f,
            autoMirror = parts.any { it.mirror },
        ).apply {
            for (p in parts) {
                if (p.fill) {
                    addPath(addPathNodes(p.d), fill = ink)
                } else {
                    addPath(
                        addPathNodes(p.d),
                        stroke = ink,
                        strokeLineWidth = p.width,
                        strokeLineCap = StrokeCap.Round,
                        strokeLineJoin = StrokeJoin.Round,
                    )
                }
            }
        }.build()
    }
}
