package io.github.motalim_ai.zajil.ui

import android.graphics.BitmapFactory
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.Icon
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextDirection
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.core.net.toUri
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import io.github.motalim_ai.zajil.R
import io.github.motalim_ai.zajil.downloads
import io.github.motalim_ai.zajil.update.Https
import io.github.motalim_ai.zajil.video.Picked
import io.github.motalim_ai.zajil.video.PlaylistEntry
import io.github.motalim_ai.zajil.video.PlaylistInfo
import io.github.motalim_ai.zajil.video.Sound
import io.github.motalim_ai.zajil.video.VideoChoice
import io.github.motalim_ai.zajil.video.VideoInfo
import java.util.Locale
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext

/** The format sheet for a video link (A3): reading it, the choice, or why not. */
sealed interface VideoSheet {
    val url: String

    data class Reading(override val url: String) : VideoSheet

    data class Ready(override val url: String, val info: VideoInfo) : VideoSheet

    /** A video in a list ([listUrl]): this one only, or the list (0.3.0). */
    data class AskList(override val url: String, val listUrl: String) : VideoSheet

    /** A playlist or a channel's videos, listed (0.3.0). */
    data class Playlist(override val url: String, val info: PlaylistInfo) : VideoSheet

    /**
     * [message] is said to the user (never yt-dlp's own words); [signIn]:
     * signing in on the site, in Zajil's browser, may let it read.
     */
    data class Refused(override val url: String, val message: Int, val signIn: Boolean = false) : VideoSheet
}

/**
 * The desktop's choice of quality and sound, sized for a phone (the
 * approved design): the heights the page has, each with its size (the
 * highest up to 1080p chosen first, since a phone's screen rarely needs
 * more), «Sound only», and the video's own sound or a dubbed one by
 * language.
 */
@Composable
fun VideoSheetView(
    sheet: VideoSheet,
    onDownload: (url: String, info: VideoInfo, picked: Picked, choice: String) -> Unit,
    onDismiss: () -> Unit,
    onBrowse: ((url: String) -> Unit)? = null,
    onRead: (url: String) -> Unit = {},
    onPlaylist: (url: String, info: PlaylistInfo, entries: List<PlaylistEntry>, height: Int?, dub: String?, queue: Long) -> Unit = { _, _, _, _, _, _ -> },
) {
    ZSheet(onDismiss = onDismiss, label = stringResource(R.string.video_quality)) {
        Column(
            Modifier.verticalScroll(rememberScrollState()),
            verticalArrangement = Arrangement.spacedBy(16.dp),
        ) {
            when (sheet) {
                is VideoSheet.Reading -> Row(
                    Modifier.padding(vertical = 12.dp),
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(16.dp),
                ) {
                    CircularProgressIndicator(Modifier.size(28.dp), color = Z.colors.accent, strokeWidth = 3.dp, trackColor = Z.colors.sunken)
                    Column {
                        ZText(stringResource(R.string.video_reading), size = 16.sp, weight = FontWeight.SemiBold)
                        ZText(sheet.url.toUri().host.orEmpty(), size = 12.5.sp, color = Z.colors.muted)
                    }
                }
                is VideoSheet.Refused -> {
                    ZText(stringResource(sheet.message), size = 17.sp, weight = FontWeight.SemiBold)
                    if (sheet.signIn && onBrowse != null) {
                        MainButton(stringResource(R.string.video_open_in_browser), onClick = { onBrowse(sheet.url) }, modifier = Modifier.fillMaxWidth())
                    }
                    QuietButton(stringResource(R.string.ok), onClick = onDismiss, modifier = Modifier.fillMaxWidth())
                }
                is VideoSheet.Ready -> Choice(sheet, onDownload, onDismiss)
                is VideoSheet.AskList -> AskListView(onVideo = { onRead(sheet.url) }, onList = { onRead(sheet.listUrl) })
                is VideoSheet.Playlist -> PlaylistView(
                    sheet.url,
                    sheet.info,
                    onDownload = { entries, height, dub, queue -> onPlaylist(sheet.url, sheet.info, entries, height, dub, queue) },
                    onOpen = onRead,
                    onDismiss = onDismiss,
                )
            }
        }
    }
}

@Composable
private fun Choice(
    sheet: VideoSheet.Ready,
    onDownload: (url: String, info: VideoInfo, picked: Picked, choice: String) -> Unit,
    @Suppress("UNUSED_PARAMETER") onDismiss: () -> Unit,
) {
    val c = Z.colors
    val info = sheet.info
    val units = sizeUnits()
    val prefs by LocalContext.current.downloads.prefs.prefs.collectAsStateWithLifecycle()
    val heights = remember(info) { VideoChoice.qualities(info) }
    val sounds = remember(info) { VideoChoice.sounds(info) }
    val soundOnly = remember(info) { VideoChoice.hasSoundOnly(info) }
    // The highest quality at or under the one chosen in Settings.
    var height by remember(info) { mutableStateOf(heights.firstOrNull { it <= prefs.videoHeight } ?: heights.lastOrNull()) }
    var sound by remember(info) { mutableStateOf(sounds.firstOrNull()) }
    val picked = sound?.let { VideoChoice.pick(info, height, it) }

    Header(sheet)

    // «فيديو / صوت فقط»: sound only is a choice of its own when the page has it.
    if (soundOnly && heights.isNotEmpty()) {
        Segmented(
            listOf(false to stringResource(R.string.kind_video), true to stringResource(R.string.video_sound_only)),
            height == null,
            onChoose = { audio -> height = if (audio) null else heights.firstOrNull { it <= prefs.videoHeight } ?: heights.lastOrNull() },
        )
    }
    Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
        val choices: List<Int?> = if (height == null || heights.isEmpty()) listOf(null) else heights
        choices.forEach { h ->
            val size = sound?.let { VideoChoice.pick(info, h, it) }?.size
            val sharpest = h != null && h == heights.firstOrNull() && heights.size > 1
            ChoiceRow(
                label = if (h == null) "M4A" else qualityLabel(h),
                selected = height == h,
                onClick = { height = h },
                note = when {
                    sharpest -> stringResource(R.string.quality_sharpest)
                    h != null && h == heights.lastOrNull() && heights.size > 2 -> stringResource(R.string.quality_saves_data)
                    else -> null
                },
                noteColor = if (sharpest) c.blue else c.green,
                end = size?.let { Units.size(it, units) } ?: "",
            )
        }
    }

    if (sounds.isNotEmpty()) SoundRow(sounds, sound) { sound = it }

    if (picked == null) ZText(stringResource(R.string.video_no_pair), size = 12.5.sp, color = c.red)

    val label = qualityLabel(height)
    val choice = listOfNotNull(label, sound?.takeIf { sounds.size > 1 }?.let { soundLabel(it) }).joinToString(" · ")
    MainButton(
        picked?.size?.let { stringResource(R.string.download_size, Units.size(it, units)) } ?: stringResource(R.string.download_choice, label),
        onClick = { picked?.let { onDownload(sheet.url, info, it, choice) } },
        modifier = Modifier.fillMaxWidth(),
        enabled = picked != null,
    )
}

/** The thumbnail with the length on it, the title, and where it is from. */
@Composable
private fun Header(sheet: VideoSheet.Ready) {
    val info = sheet.info
    val thumbnail by produceState<ImageBitmap?>(null, info.thumbnail) {
        value = info.thumbnail?.let { link ->
            withContext(Dispatchers.IO) {
                // A picture of at most 2 MB, from the page's own image server.
                runCatching { Https.bytes(link, 2 shl 20) }.getOrNull()
                    ?.let { BitmapFactory.decodeByteArray(it, 0, it.size) }
                    ?.asImageBitmap()
            }
        }
    }
    Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
        Box(
            Modifier
                .size(width = 116.dp, height = 66.dp)
                .clip(RoundedCornerShape(16.dp))
                .background(Brush.linearGradient(listOf(Color(0xFF1747C9), Color(0xFF3FA5FF)))),
            contentAlignment = Alignment.Center,
        ) {
            thumbnail?.let {
                Image(it, contentDescription = null, contentScale = ContentScale.Crop, modifier = Modifier.fillMaxSize())
            }
            ZIcon(LineIcon.PlayFill, Color.White, 24.dp)
            info.duration?.let { seconds ->
                Box(
                    Modifier
                        .align(Alignment.BottomEnd)
                        .padding(6.dp)
                        .height(20.dp)
                        .clip(RoundedCornerShape(6.dp))
                        .background(Color(0x8C000000))
                        .padding(horizontal = 6.dp),
                    contentAlignment = Alignment.Center,
                ) {
                    ZText(duration(seconds), size = 11.5.sp, color = Color.White)
                }
            }
        }
        Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(2.dp)) {
            ZText(
                info.title,
                size = 16.sp,
                weight = FontWeight.SemiBold,
                maxLines = 2,
                // An English title reads left to right, an Arabic one right to left.
                style = TextStyle(textDirection = TextDirection.Content, lineHeight = 22.sp),
            )
            ZText(
                listOfNotNull(sheet.url.toUri().host?.removePrefix("www.")?.removePrefix("m."), info.channel).joinToString(stringResource(R.string.list_sep)),
                size = 12.5.sp,
                color = Z.colors.faint,
                maxLines = 1,
            )
        }
    }
}

/** «الصوت»: the video's own sound, or a dubbed one from a menu (a video may have dozens). */
@Composable
private fun SoundRow(sounds: List<Sound>, chosen: Sound?, onChoose: (Sound) -> Unit) {
    val c = Z.colors
    var open by remember { mutableStateOf(false) }
    Box {
        Row(
            Modifier
                .fillMaxWidth()
                .heightIn(min = 52.dp)
                .clip(RoundedCornerShape(16.dp))
                .clickable(enabled = sounds.size > 1, role = Role.DropdownList) { open = true },
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            IconCircle(LineIcon.AudioBars, c.sunken, c.softInk)
            Column(Modifier.weight(1f)) {
                ZText(stringResource(R.string.video_sound), size = 12.5.sp, color = c.faint)
                ZText(chosen?.let { soundLabel(it) }.orEmpty(), size = 14.5.sp, weight = FontWeight.Medium, maxLines = 1)
            }
            if (sounds.size > 1) ZIcon(LineIcon.Chevron, c.chevron, 20.dp)
        }
        DropdownMenu(expanded = open, onDismissRequest = { open = false }, containerColor = c.surface) {
            sounds.forEach { s ->
                DropdownMenuItem(text = { ZText(soundLabel(s)) }, onClick = {
                    onChoose(s)
                    open = false
                })
            }
        }
    }
}

/** «1080p», the page's one file, or «Sound only». */
@Composable
private fun qualityLabel(height: Int?): String = when (height) {
    null -> stringResource(R.string.video_sound_only)
    VideoChoice.WHOLE -> stringResource(R.string.quality_whole)
    else -> "${height}p"
}

/** «25:14», «1:02:09». */
internal fun duration(seconds: Double): String {
    val s = seconds.toLong().coerceAtLeast(0)
    return if (s >= 3600) {
        "%d:%02d:%02d".format(Locale.ROOT, s / 3600, s % 3600 / 60, s % 60)
    } else {
        "%d:%02d".format(Locale.ROOT, s / 60, s % 60)
    }
}

/** The video's own sound first, then the dubbed ones, by language name in the user's language. */
@Composable
private fun soundLabel(sound: Sound): String {
    val language = sound.language?.let { tag ->
        Locale.forLanguageTag(tag).getDisplayLanguage(Locale.getDefault()).ifBlank { tag }
    }
    return when {
        sound.original && language != null -> stringResource(R.string.video_sound_original_in, language)
        sound.original -> stringResource(R.string.video_sound_original)
        else -> stringResource(R.string.video_sound_dubbed, language ?: "?")
    }
}
