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
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.selection.toggleable
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Icon
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableLongStateOf
import androidx.compose.runtime.mutableStateListOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.pluralStringResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextDirection
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import io.github.motalim_ai.zajil.R
import io.github.motalim_ai.zajil.downloads
import io.github.motalim_ai.zajil.downloads.Queue
import io.github.motalim_ai.zajil.update.Https
import io.github.motalim_ai.zajil.video.PlaylistEntry
import io.github.motalim_ai.zajil.video.PlaylistInfo
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext

/** One quality for every video of a playlist: the best up to a height, or the sound alone (null). */
private val PLAYLIST_QUALITIES: List<Int?> = listOf(1080, 720, 480, null)

/** A dubbed sound offered for a whole playlist: Arabic, when a video has it. */
private const val ARABIC = "ar"

/**
 * A video that is in a list (YouTube's `watch?v=…&list=…`): this video
 * only, or the whole list.
 */
@Composable
fun AskListView(onVideo: () -> Unit, onList: () -> Unit) {
    Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
        ZText(stringResource(R.string.in_a_playlist), size = 19.sp, weight = FontWeight.Bold)
        ZText(stringResource(R.string.in_a_playlist_hint), size = 13.5.sp, color = Z.colors.muted)
        ChoiceRow(stringResource(R.string.this_video_only), selected = false, onClick = onVideo)
        ChoiceRow(stringResource(R.string.whole_playlist), selected = false, onClick = onList)
    }
}

/**
 * A playlist or a channel's videos (0.3.0, as the desktop's since 1.1):
 * the title and how many, each video checked with its picture and length,
 * «Select all», one quality for all (the nearest each video has), the
 * sound, and the queue. A list inside the list (a channel's tab) opens.
 */
@Composable
fun PlaylistView(
    url: String,
    info: PlaylistInfo,
    onDownload: (entries: List<PlaylistEntry>, height: Int?, dub: String?, queue: Long) -> Unit,
    onOpen: (String) -> Unit,
    onDismiss: () -> Unit,
) {
    val c = Z.colors
    val center = LocalContext.current.downloads
    val queues by center.settings.queues.collectAsStateWithLifecycle()
    val chosen = remember(info) { mutableStateListOf(*info.entries.map { it.downloadable }.toTypedArray()) }
    var height by rememberSaveable { mutableStateOf<Int?>(1080) }
    var dub by rememberSaveable { mutableStateOf<String?>(null) }
    var queue by rememberSaveable { mutableLongStateOf(Queue.MAIN) }
    var asking by rememberSaveable { mutableStateOf<String?>(null) }
    val videos = info.entries.count { it.downloadable }
    val count = info.entries.indices.count { chosen[it] && info.entries[it].downloadable }
    val unavailable = info.entries.count { !it.playlist && !it.available }

    Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
        Column(verticalArrangement = Arrangement.spacedBy(2.dp)) {
            ZText(
                info.title,
                size = 17.sp,
                weight = FontWeight.Bold,
                maxLines = 2,
                style = TextStyle(textDirection = TextDirection.Content, lineHeight = 23.sp),
            )
            ZText(
                listOfNotNull(pluralStringResource(R.plurals.playlist_videos, videos, videos), info.channel).joinToString(" · "),
                size = 12.5.sp,
                color = c.muted,
                maxLines = 1,
            )
        }
        if (unavailable > 0) {
            ZText(pluralStringResource(R.plurals.playlist_unavailable, unavailable, unavailable), size = 12.5.sp, color = c.amber)
        }
        if (videos > 0) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                ZText(pluralStringResource(R.plurals.playlist_chosen, count, count), Modifier.weight(1f), size = 13.5.sp, color = c.muted)
                val all = count == videos
                ZText(
                    stringResource(if (all) R.string.select_none else R.string.select_all),
                    Modifier
                        .clip(RoundedCornerShape(8.dp))
                        .clickable(role = Role.Button) { info.entries.indices.forEach { chosen[it] = !all && info.entries[it].downloadable } }
                        .padding(horizontal = 8.dp, vertical = 6.dp),
                    size = 13.5.sp,
                    weight = FontWeight.SemiBold,
                    color = c.accent,
                )
            }
        }
        LazyColumn(
            Modifier
                .fillMaxWidth()
                .heightIn(max = 330.dp)
                .clip(RoundedCornerShape(14.dp))
                .border(1.dp, c.line, RoundedCornerShape(14.dp)),
        ) {
            itemsIndexed(info.entries) { i, entry ->
                if (i > 0) RowLine()
                EntryRow(entry, chosen[i], onToggle = { chosen[i] = it }, onOpen = { entry.url?.let(onOpen) })
            }
        }
        if (videos == 0 && info.entries.any { it.playlist }) {
            ZText(stringResource(R.string.playlists_only), size = 12.5.sp, color = c.muted)
        }
        ZCard {
            SettingRow(stringResource(R.string.video_quality), value = playlistQuality(height), onClick = { asking = "quality" })
            RowLine()
            SettingRow(
                stringResource(R.string.video_sound),
                value = stringResource(if (dub == null) R.string.video_sound_original else R.string.dub_arabic_if_any),
                onClick = { asking = "sound" },
            )
            if (queues.size > 1) {
                RowLine()
                SettingRow(
                    stringResource(R.string.choose_queue),
                    value = queueName(queues.firstOrNull { it.id == queue } ?: queues.first()),
                    onClick = { asking = "queue" },
                )
            }
        }
        Row(horizontalArrangement = Arrangement.spacedBy(10.dp)) {
            MainButton(
                pluralStringResource(R.plurals.download_chosen, count, count),
                onClick = { onDownload(info.entries.filterIndexed { i, e -> chosen[i] && e.downloadable }, height, dub, queue) },
                modifier = Modifier.weight(2f),
                enabled = count > 0,
                icon = lineIcon(LineIcon.Downloads),
            )
            QuietButton(stringResource(R.string.cancel_sheet), onClick = onDismiss, modifier = Modifier.weight(1f))
        }
    }
    when (asking) {
        "quality" -> ChoiceSheet(
            stringResource(R.string.video_quality),
            PLAYLIST_QUALITIES.map { it to playlistQuality(it) },
            chosen = height,
            onChoose = { height = it },
            onDismiss = { asking = null },
        )
        "sound" -> ChoiceSheet(
            stringResource(R.string.video_sound),
            listOf(null to stringResource(R.string.video_sound_original), ARABIC to stringResource(R.string.dub_arabic_if_any)),
            chosen = dub,
            onChoose = { dub = it },
            onDismiss = { asking = null },
        )
        "queue" -> ChoiceSheet(
            stringResource(R.string.choose_queue),
            queues.map { it.id to queueName(it) },
            chosen = queue,
            onChoose = { queue = it },
            onDismiss = { asking = null },
        )
    }
}

@Composable
private fun playlistQuality(height: Int?): String =
    if (height == null) stringResource(R.string.video_sound_only) else stringResource(R.string.best_up_to, "${height}p")

/** One video of the list: its box, picture, title and length; a list inside opens. */
@Composable
private fun EntryRow(entry: PlaylistEntry, checked: Boolean, onToggle: (Boolean) -> Unit, onOpen: () -> Unit) {
    val c = Z.colors
    val enabled = entry.downloadable
    Row(
        Modifier
            .fillMaxWidth()
            .then(
                if (entry.playlist) {
                    Modifier.clickable(role = Role.Button, onClick = onOpen)
                } else {
                    Modifier.toggleable(value = checked, enabled = enabled, role = Role.Checkbox, onValueChange = onToggle)
                },
            )
            .padding(horizontal = 10.dp, vertical = 8.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(10.dp),
    ) {
        if (entry.playlist) {
            Icon(lineIcon(LineIcon.Queues), null, tint = c.accent, modifier = Modifier.size(22.dp))
        } else {
            CheckBox(checked && enabled, enabled)
        }
        Thumb(entry)
        Column(Modifier.weight(1f)) {
            ZText(
                entry.title ?: entry.url.orEmpty(),
                size = 13.5.sp,
                maxLines = 2,
                color = if (enabled || entry.playlist) c.ink else c.muted,
                style = TextStyle(textDirection = TextDirection.Content, lineHeight = 18.sp),
            )
            val note = when {
                entry.playlist -> stringResource(R.string.open_playlist)
                !entry.available -> stringResource(R.string.video_short_unavailable)
                else -> null
            }
            if (note != null) ZText(note, size = 11.5.sp, color = if (entry.playlist) c.accent else c.amber)
        }
    }
}

/** A small picture with the length on it; loaded only while its row is on screen. */
@Composable
private fun Thumb(entry: PlaylistEntry) {
    val picture by produceState<ImageBitmap?>(null, entry.thumbnail) {
        value = entry.thumbnail?.let { link ->
            withContext(Dispatchers.IO) {
                // A small picture, at most 512 KB, from the site's own image server.
                runCatching { Https.bytes(link, 512 shl 10) }.getOrNull()
                    ?.let { bytes ->
                        val options = BitmapFactory.Options().apply { inSampleSize = 2 }
                        BitmapFactory.decodeByteArray(bytes, 0, bytes.size, options)
                    }
                    ?.asImageBitmap()
            }
        }
    }
    Box(
        Modifier.size(width = 72.dp, height = 41.dp).clip(RoundedCornerShape(8.dp)).background(Z.colors.sunken),
        contentAlignment = Alignment.Center,
    ) {
        picture?.let { Image(it, null, contentScale = ContentScale.Crop, modifier = Modifier.fillMaxSize()) }
        entry.duration?.let { seconds ->
            ZText(
                duration(seconds),
                Modifier
                    .align(Alignment.BottomStart)
                    .padding(start = 3.dp, bottom = 3.dp)
                    .clip(RoundedCornerShape(4.dp))
                    .background(Color(0x8C000000))
                    .padding(horizontal = 3.dp),
                size = 9.5.sp,
                color = Color.White,
            )
        }
    }
}

@Composable
private fun CheckBox(checked: Boolean, enabled: Boolean) {
    val c = Z.colors
    val shape = RoundedCornerShape(6.dp)
    Box(
        Modifier
            .size(22.dp)
            .clip(shape)
            .background(if (checked) c.accent else Color.Transparent)
            .border(2.dp, if (checked) c.accent else if (enabled) c.muted else c.line, shape),
        contentAlignment = Alignment.Center,
    ) {
        if (checked) Icon(lineIcon(LineIcon.Check), null, tint = c.accentInk, modifier = Modifier.size(16.dp))
    }
}
