package io.github.motalim_ai.zajil.ui

import android.text.format.Formatter
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.FilterChip
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.style.TextDirection
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import io.github.motalim_ai.zajil.R
import io.github.motalim_ai.zajil.video.Picked
import io.github.motalim_ai.zajil.video.Sound
import io.github.motalim_ai.zajil.video.VideoChoice
import io.github.motalim_ai.zajil.video.VideoInfo
import java.util.Locale

/** The format sheet for a video link (A3): reading it, the choice, or why not. */
sealed interface VideoSheet {
    val url: String

    data class Reading(override val url: String) : VideoSheet

    data class Ready(override val url: String, val info: VideoInfo) : VideoSheet

    /** [message] is said to the user; [detail] is yt-dlp's own words. */
    data class Refused(override val url: String, val message: Int, val detail: String?) : VideoSheet
}

/**
 * The desktop's choice of quality and sound, sized for a phone: the
 * heights the page has (the highest up to 1080p chosen first, since a
 * phone's screen rarely needs more), «Sound only», and the video's own sound
 * or a dubbed one by language.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun VideoSheetView(
    sheet: VideoSheet,
    onDownload: (url: String, info: VideoInfo, picked: Picked, choice: String) -> Unit,
    onDismiss: () -> Unit,
) {
    ModalBottomSheet(onDismissRequest = onDismiss) {
        Column(
            Modifier
                .padding(horizontal = 20.dp)
                .padding(bottom = 24.dp)
                .verticalScroll(rememberScrollState()),
        ) {
            when (sheet) {
                is VideoSheet.Reading -> Row(verticalAlignment = Alignment.CenterVertically) {
                    CircularProgressIndicator(Modifier.padding(end = 16.dp))
                    Text(stringResource(R.string.video_reading))
                }
                is VideoSheet.Refused -> {
                    Text(stringResource(sheet.message), style = MaterialTheme.typography.titleMedium)
                    sheet.detail?.let {
                        Text(it, style = MaterialTheme.typography.bodySmall, modifier = Modifier.padding(top = 8.dp))
                    }
                    TextButton(onClick = onDismiss, modifier = Modifier.padding(top = 8.dp)) {
                        Text(stringResource(R.string.ok))
                    }
                }
                is VideoSheet.Ready -> Choice(sheet, onDownload, onDismiss)
            }
        }
    }
}

@OptIn(androidx.compose.foundation.layout.ExperimentalLayoutApi::class)
@Composable
private fun Choice(
    sheet: VideoSheet.Ready,
    onDownload: (url: String, info: VideoInfo, picked: Picked, choice: String) -> Unit,
    onDismiss: () -> Unit,
) {
    val context = LocalContext.current
    val info = sheet.info
    val heights = remember(info) { VideoChoice.qualities(info) }
    val sounds = remember(info) { VideoChoice.sounds(info) }
    var height by remember(info) { mutableStateOf(heights.firstOrNull { it <= 1080 } ?: heights.firstOrNull()) }
    var sound by remember(info) { mutableStateOf(sounds.firstOrNull()) }
    val picked = sound?.let { VideoChoice.pick(info, height, it) }

    Text(
        info.title,
        // An English title reads left to right, an Arabic one right to left.
        style = MaterialTheme.typography.titleMedium.copy(textDirection = TextDirection.Content),
        maxLines = 2,
        overflow = TextOverflow.Ellipsis,
    )
    Spacer(Modifier.height(16.dp))
    Text(stringResource(R.string.video_quality), style = MaterialTheme.typography.labelLarge)
    FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
        heights.forEach { h ->
            FilterChip(selected = height == h, onClick = { height = h }, label = { Text("${h}p") })
        }
        FilterChip(
            selected = height == null,
            onClick = { height = null },
            label = { Text(stringResource(R.string.video_sound_only)) },
        )
    }
    if (sounds.size > 1) {
        // A video may have dozens of dubbed sounds: a menu, not a list,
        // so that «Download» stays in reach.
        Spacer(Modifier.height(12.dp))
        Text(stringResource(R.string.video_sound), style = MaterialTheme.typography.labelLarge)
        var open by remember { mutableStateOf(false) }
        Box {
            OutlinedButton(onClick = { open = true }) { Text("${sound?.let { soundLabel(it) }.orEmpty()}  ▾") }
            DropdownMenu(expanded = open, onDismissRequest = { open = false }) {
                sounds.forEach { s ->
                    DropdownMenuItem(text = { Text(soundLabel(s)) }, onClick = {
                        sound = s
                        open = false
                    })
                }
            }
        }
    }
    Spacer(Modifier.height(12.dp))
    Text(
        when {
            picked == null -> stringResource(R.string.video_no_pair)
            picked.size == null -> stringResource(R.string.video_size_unknown)
            else -> stringResource(R.string.video_size, Formatter.formatShortFileSize(context, picked.size!!))
        },
        style = MaterialTheme.typography.bodySmall,
    )
    Row(Modifier.padding(top = 12.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
        val choice = listOfNotNull(
            height?.let { "${it}p" } ?: stringResource(R.string.video_sound_only),
            sound?.takeIf { sounds.size > 1 }?.let { soundLabel(it) },
        ).joinToString(" · ")
        Button(onClick = { picked?.let { onDownload(sheet.url, info, it, choice) } }, enabled = picked != null) {
            Text(stringResource(R.string.download))
        }
        TextButton(onClick = onDismiss) { Text(stringResource(R.string.not_now)) }
    }
}

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
