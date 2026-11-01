package io.github.motalim_ai.zajil.ui

import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.StrokeJoin
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.graphics.vector.addPathNodes
import androidx.compose.ui.unit.dp

/**
 * The desktop's coloured icons (apps/desktop/src/lib/components
 * ColorIcon.svelte), drawn from the same path data on the same 32×32
 * grid: a filled shape with a soft top-to-bottom gradient and a white
 * glyph or a small badge. Drawn for Zajil, from no other program's icons.
 */
enum class ColorIcon {
    Add, Pause, Resume, Refresh, Stop, Schedule, Queue, QueueTimed,
    Video, Music, Images, Archives, Programs, Documents, File,
}

/** The desktop's tones: each shape's top and bottom colour. */
private enum class Tone(val top: Long, val bottom: Long) {
    Green(0xFF46D36C, 0xFF169A3D),
    Red(0xFFFF6B61, 0xFFD42F2C),
    Amber(0xFFFFC54D, 0xFFE08A06),
    Blue(0xFF5AA0FF, 0xFF1D63D8),
    Purple(0xFFAD85FF, 0xFF6C3CE0),
    Brown(0xFFF1A454, 0xFFB8641B),
    Dark(0xFF5D7584, 0xFF2A3A44),
    Grey(0xFFC3CAD4, 0xFF8A94A3),
    Yellow(0xFFFFD361, 0xFFE59C1C),
    ;

    val bottomColor get() = Color(bottom)

    fun brush(top: Float, bottom: Float) =
        Brush.verticalGradient(listOf(Color(this.top), Color(this.bottom)), startY = top, endY = bottom)
}

private val White = SolidColor(Color.White)

private fun circle(cx: Float, cy: Float, r: Float) =
    "M${cx - r},${cy}a$r,$r 0 1,0 ${2 * r},0a$r,$r 0 1,0 ${-2 * r},0z"

private fun rect(x: Float, y: Float, w: Float, h: Float, rx: Float) =
    "M${x + rx},${y}h${w - 2 * rx}a$rx,$rx 0 0 1 $rx,${rx}v${h - 2 * rx}a$rx,$rx 0 0 1 ${-rx},${rx}" +
        "h${-(w - 2 * rx)}a$rx,$rx 0 0 1 ${-rx},${-rx}v${-(h - 2 * rx)}a$rx,$rx 0 0 1 $rx,${-rx}z"

private class Drawing(size: Float = 32f) {
    val builder = ImageVector.Builder(
        defaultWidth = size.dp,
        defaultHeight = size.dp,
        viewportWidth = size,
        viewportHeight = size,
    )

    /** A shape filled with [tone]'s gradient from [top] to [bottom] (its height on the grid). */
    fun fill(d: String, tone: Tone, top: Float, bottom: Float, alpha: Float = 1f) {
        builder.addPath(addPathNodes(d), fill = tone.brush(top, bottom), fillAlpha = alpha)
    }

    fun fill(d: String, brush: Brush, alpha: Float = 1f) {
        builder.addPath(addPathNodes(d), fill = brush, fillAlpha = alpha)
    }

    fun stroke(d: String, brush: Brush, width: Float, alpha: Float = 1f, round: Boolean = true) {
        builder.addPath(
            addPathNodes(d),
            stroke = brush,
            strokeAlpha = alpha,
            strokeLineWidth = width,
            strokeLineCap = if (round) StrokeCap.Round else StrokeCap.Butt,
            strokeLineJoin = StrokeJoin.Round,
        )
    }

    /** A badge's ring in the colour behind the icon ("var(--surface)" on the desktop). */
    fun ring(cx: Float, cy: Float, r: Float, tone: Tone, surface: Color) {
        fill(circle(cx, cy, r + 0.75f), SolidColor(surface))
        fill(circle(cx, cy, r), tone, cy - r, cy + r)
    }

    fun build() = builder.build()
}

private val cache = HashMap<Pair<ColorIcon, Color>, ImageVector>()

/** [icon] drawn for a background of [surface] (its badges' rings). */
fun colorIcon(icon: ColorIcon, surface: Color): ImageVector = synchronized(cache) {
    cache.getOrPut(icon to surface) { draw(icon, surface) }
}

private fun draw(icon: ColorIcon, surface: Color): ImageVector = Drawing().apply {
    when (icon) {
        ColorIcon.Add -> {
            fill(circle(16f, 16f, 13.5f), Tone.Green, 2.5f, 29.5f)
            stroke("M16 9v14M9 16h14", White, 3.6f)
        }
        ColorIcon.Resume ->
            fill("M9 6.4c0-1.7 1.9-2.7 3.3-1.8l14.2 9.6c1.2.8 1.2 2.6 0 3.4L12.3 27.2C10.9 28.1 9 27.1 9 25.4z", Tone.Green, 4.3f, 27.5f)
        ColorIcon.Pause -> {
            fill(rect(7f, 5f, 7f, 22f, 2.2f), Tone.Amber, 5f, 27f)
            fill(rect(18f, 5f, 7f, 22f, 2.2f), Tone.Amber, 5f, 27f)
        }
        ColorIcon.Refresh -> {
            stroke("M25 12.5A9.5 9.5 0 1 0 25.6 20", Tone.Green.brush(6.5f, 25.5f), 3.6f)
            builder.addPath(
                addPathNodes("M27.5 5.5v8.2h-8.2z"),
                fill = SolidColor(Tone.Green.bottomColor),
                stroke = SolidColor(Tone.Green.bottomColor),
                strokeLineWidth = 1.6f,
                strokeLineJoin = StrokeJoin.Round,
            )
        }
        ColorIcon.Stop -> fill(rect(7f, 7f, 18f, 18f, 4f), Tone.Red, 7f, 25f)
        ColorIcon.Schedule -> {
            fill(circle(16f, 16.5f, 13f), Tone.Blue, 3.5f, 29.5f)
            fill(circle(16f, 16.5f, 9.6f), White)
            stroke("M16 10.5v6.3l4 2.6", SolidColor(Tone.Blue.bottomColor), 2.4f)
            fill(circle(16f, 16.8f, 1.4f), SolidColor(Tone.Blue.bottomColor))
        }
        ColorIcon.Queue, ColorIcon.QueueTimed -> {
            fill("M2 8.6A3 3 0 0 1 5 5.6h6.4l2.8 3H24a3 3 0 0 1 3 3V24a3 3 0 0 1-3 3H5a3 3 0 0 1-3-3z", SolidColor(Tone.Yellow.bottomColor))
            fill("M2 12.5h25V24a3 3 0 0 1-3 3H5a3 3 0 0 1-3-3z", Tone.Yellow, 12.5f, 27f)
            if (icon == ColorIcon.Queue) {
                ring(24f, 23f, 7f, Tone.Green, surface)
                builder.addPath(
                    addPathNodes("M22.3 20v6l4.9-3z"),
                    fill = White,
                    stroke = White,
                    strokeLineWidth = 0.8f,
                    strokeLineJoin = StrokeJoin.Round,
                )
            } else {
                ring(24f, 23f, 7f, Tone.Blue, surface)
                stroke("M24 19.6v3.6l2.3 1.4", White, 1.8f)
            }
        }
        ColorIcon.Video -> {
            fill(rect(3f, 6f, 26f, 20f, 3.5f), Tone.Blue, 6f, 26f)
            for (y in listOf(8.5f, 13.2f, 17.9f, 22.6f)) {
                fill(rect(5f, y - 1.2f, 2.4f, 2.4f, 0.6f), White, alpha = 0.8f)
                fill(rect(24.6f, y - 1.2f, 2.4f, 2.4f, 0.6f), White, alpha = 0.8f)
            }
            builder.addPath(
                addPathNodes("M13 11.2v9.6l7.6-4.8z"),
                fill = White,
                stroke = White,
                strokeLineWidth = 1f,
                strokeLineJoin = StrokeJoin.Round,
            )
        }
        ColorIcon.Music -> {
            fill(circle(16f, 16f, 13.5f), Tone.Purple, 2.5f, 29.5f)
            fill(circle(13f, 21.3f, 3.4f), White)
            fill(rect(15f, 8f, 2.3f, 13.5f, 1f), White)
            fill("M17.3 8c2.6.6 5 2.3 5 5.2-1.2-1.4-2.9-2-5-2z", White)
        }
        ColorIcon.Images -> {
            fill(rect(3f, 5f, 26f, 22f, 3.5f), Tone.Green, 5f, 27f)
            fill(circle(11f, 12f, 2.8f), SolidColor(Color(0xFFFFF6C4)))
            fill("M5.5 24.6l7.2-8 4.8 5.2 3.6-3.6 5.4 6.4z", White, alpha = 0.92f)
        }
        ColorIcon.Archives -> {
            fill(rect(5f, 11f, 22f, 16.5f, 2.2f), Tone.Brown, 11f, 27.5f)
            fill(rect(3.5f, 5.5f, 25f, 7f, 2f), SolidColor(Tone.Brown.bottomColor))
            fill(rect(13f, 12.8f, 6f, 5.4f, 1.2f), White, alpha = 0.9f)
        }
        ColorIcon.Programs -> {
            fill(rect(3f, 5f, 26f, 22f, 3f), Tone.Dark, 5f, 27f)
            fill(rect(5.4f, 11.5f, 21.2f, 13f, 1.3f), SolidColor(Color(0xFFEEF3F6)))
            fill(circle(7.4f, 8.3f, 1.1f), SolidColor(Color(0xFFFF6B61)))
            fill(circle(10.6f, 8.3f, 1.1f), SolidColor(Color(0xFFFFC54D)))
            fill(circle(13.8f, 8.3f, 1.1f), SolidColor(Color(0xFF46D36C)))
            stroke("M9 15.5l3.2 2.6L9 20.7M14.2 21h6", SolidColor(Color(0xFF2A3A44)), 2f)
        }
        ColorIcon.Documents, ColorIcon.File -> {
            // A page: blue with lines for a document, grey for any file.
            val tone = if (icon == ColorIcon.Documents) Tone.Blue else Tone.Grey
            fill("M8.5 3h10.8L26 9.7V27a2 2 0 0 1-2 2H8.5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2z", tone, 3f, 29f)
            fill("M19.3 3v4.7a2 2 0 0 0 2 2H26", White, alpha = 0.5f)
            if (icon == ColorIcon.Documents) stroke("M10.5 15h11M10.5 19h11M10.5 23h7", White, 1.9f)
        }
    }
}.build()

/**
 * The design's one-colour line icons, on a 24×24 grid: drawn in black and
 * tinted by `Icon`.
 */
enum class LineIcon {
    Search, More, Downloads, Queues, Settings, Shared, Folder, Check, ChevronDown, Shield, Play,
    Globe, Lock, LockOpen, Back, Reload, Private, Clock, Plus, Close, Star, StarFilled, History,
}

private val lineCache = HashMap<LineIcon, ImageVector>()

fun lineIcon(icon: LineIcon): ImageVector = synchronized(lineCache) {
    lineCache.getOrPut(icon) {
        val ink = SolidColor(Color.Black)
        Drawing(24f).apply {
            when (icon) {
                LineIcon.Search -> stroke("${circle(11f, 11f, 7f)}M20 20l-3.5-3.5", ink, 2f)
                LineIcon.More -> listOf(5f, 12f, 19f).forEach { fill(circle(12f, it, 2f), ink) }
                LineIcon.Downloads -> stroke("M12 4v11M7 10l5 5 5-5M5 19h14", ink, 2.2f)
                LineIcon.Queues -> stroke("M4 6h12M4 12h12M4 18h8${circle(18f, 18f, 3f)}", ink, 2f)
                LineIcon.Settings -> {
                    stroke("M4 7h16M4 12h16M4 17h16", ink, 2f)
                    fill(circle(15f, 7f, 2.2f) + circle(8f, 12f, 2.2f) + circle(14f, 17f, 2.2f), ink)
                }
                LineIcon.Shared -> stroke("M4 12v7a1 1 0 0 0 1 1h14a1 1 0 0 0 1-1v-7M12 3v12M8 7l4-4 4 4", ink, 2.2f)
                LineIcon.Folder -> stroke("M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z", ink, 2f)
                LineIcon.Check -> stroke("M5 12.5l4.5 4.5L19 7.5", ink, 2.6f)
                LineIcon.ChevronDown -> stroke("M6 9l6 6 6-6", ink, 2.4f)
                LineIcon.Shield -> stroke("M12 3l7 3v5c0 4.5-3 8.3-7 10-4-1.7-7-5.5-7-10V6zM9 12l2.2 2.2L15.5 10", ink, 2.2f)
                LineIcon.Play -> {
                    fill(circle(12f, 12f, 11f), SolidColor(Color.White), alpha = 0.25f)
                    fill("M10 8v8l6-4z", SolidColor(Color.White))
                }
                LineIcon.Globe -> stroke(circle(12f, 12f, 9f) + "M3 12h18M12 3c2.5 2.6 3.8 5.6 3.8 9s-1.3 6.4-3.8 9c-2.5-2.6-3.8-5.6-3.8-9S9.5 5.6 12 3z", ink, 2f)
                LineIcon.Lock -> stroke(rect(5f, 11f, 14f, 10f, 2f) + "M8 11V8a4 4 0 0 1 8 0v3", ink, 2.4f)
                // http: the shackle open.
                LineIcon.LockOpen -> stroke(rect(5f, 11f, 14f, 10f, 2f) + "M8 11V7a4 4 0 0 1 7.6-1.7", ink, 2.4f)
                // «Back» points to the start: right in Arabic; mirrored for English by the caller.
                LineIcon.Back -> stroke("M9 5l7 7-7 7", ink, 2.2f)
                LineIcon.Reload -> stroke("M19 8a8 8 0 1 0 1 5M20 3v5h-5", ink, 2.2f)
                LineIcon.Private -> stroke("M3 12s3.5-6 9-6 9 6 9 6-3.5 6-9 6-9-6-9-6zM4 4l16 16", ink, 2f)
                LineIcon.Clock -> stroke(circle(12f, 12f, 9f) + "M12 7v5l3 2", ink, 2f)
                LineIcon.Plus -> stroke("M12 5v14M5 12h14", ink, 2.4f)
                LineIcon.Close -> stroke("M6 6l12 12M18 6L6 18", ink, 2.2f)
                LineIcon.Star -> stroke("M12 3.5l2.6 5.3 5.9.9-4.25 4.15 1 5.85L12 16.95 6.75 19.7l1-5.85L3.5 9.7l5.9-.9z", ink, 2f)
                LineIcon.StarFilled -> fill("M12 3.5l2.6 5.3 5.9.9-4.25 4.15 1 5.85L12 16.95 6.75 19.7l1-5.85L3.5 9.7l5.9-.9z", ink)
                LineIcon.History -> stroke(circle(12f, 12f, 9f) + "M12 7v5l3 2M3 12H1M3 12l2-2", ink, 2f)
            }
        }.build()
    }
}
