package io.github.motalim_ai.zajil.ui

import android.content.ActivityNotFoundException
import android.content.ClipboardManager
import android.content.Context
import android.content.Intent
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.Button
import androidx.compose.material3.Card
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.style.TextDirection
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.core.net.toUri
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import io.github.motalim_ai.zajil.R
import io.github.motalim_ai.zajil.downloads
import io.github.motalim_ai.zajil.downloads.Download
import io.github.motalim_ai.zajil.downloads.Notifications
import io.github.motalim_ai.zajil.downloads.QueuePlan
import io.github.motalim_ai.zajil.downloads.QueueRules
import io.github.motalim_ai.zajil.downloads.Status
import io.github.motalim_ai.zajil.downloads.Wait
import io.github.motalim_ai.zajil.engine.Problem
import java.time.format.DateTimeFormatter

/** What the screen asks of the app. */
interface DownloadActions {
    fun add(url: String)
    fun pause(id: Long)
    fun resume(id: Long)
    fun startNow(id: Long)
    fun moveUp(id: Long)
    fun remove(id: Long)
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun DownloadsScreen(actions: DownloadActions, offer: Offer?, onOfferDone: () -> Unit) {
    val context = LocalContext.current
    val center = context.downloads
    val list by center.list.collectAsStateWithLifecycle()
    val rules by center.settings.rules.collectAsStateWithLifecycle()
    // Read so that the queued downloads say why they wait as it changes.
    center.networkState.collectAsStateWithLifecycle().value
    var link by rememberSaveable { mutableStateOf("") }
    var settingsOpen by rememberSaveable { mutableStateOf(false) }
    val canAdd = isLink(link.trim())
    val add = {
        if (canAdd) {
            actions.add(link.trim())
            link = ""
        }
    }
    val queue = remember(list) { QueuePlan.queued(list).map { it.id } }
    val listState = rememberLazyListState()
    // A new download shows at the top: scroll there so that it is seen.
    val newest = list.maxOfOrNull { it.id }
    LaunchedEffect(newest) { if (newest != null) listState.animateScrollToItem(0) }

    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text(stringResource(R.string.app_name)) },
                actions = {
                    TextButton(onClick = { settingsOpen = true }) { Text(stringResource(R.string.settings)) }
                },
            )
        },
    ) { padding ->
        Column(
            Modifier
                .padding(padding)
                .padding(horizontal = 16.dp)
                .fillMaxSize(),
        ) {
            OutlinedTextField(
                value = link,
                onValueChange = { link = it },
                label = { Text(stringResource(R.string.link)) },
                singleLine = true,
                // Links read left to right, in any language.
                textStyle = TextStyle(textDirection = TextDirection.Ltr),
                keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Uri, imeAction = ImeAction.Go),
                keyboardActions = KeyboardActions(onGo = { add() }),
                modifier = Modifier.fillMaxWidth(),
            )
            Row(Modifier.padding(vertical = 8.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                Button(onClick = add, enabled = canAdd) { Text(stringResource(R.string.download)) }
                OutlinedButton(onClick = { pasted(context)?.let { link = it } }) {
                    Text(stringResource(R.string.paste))
                }
            }
            if (rules.scheduled) {
                Text(
                    stringResource(
                        if (rules.open) R.string.schedule_open else R.string.schedule_waiting,
                        rules.startAt.format(DateTimeFormatter.ofPattern("HH:mm")),
                    ),
                    style = MaterialTheme.typography.bodySmall,
                    modifier = Modifier.padding(bottom = 8.dp),
                )
            }
            if (list.isEmpty()) {
                Text(
                    stringResource(R.string.empty),
                    style = MaterialTheme.typography.bodyMedium,
                    modifier = Modifier.padding(top = 24.dp),
                )
            }
            LazyColumn(state = listState, verticalArrangement = Arrangement.spacedBy(8.dp)) {
                items(list.sortedByDescending { it.id }, key = { it.id }) { download ->
                    DownloadCard(download, queue.indexOf(download.id), rules, actions)
                }
            }
        }
    }

    if (settingsOpen) QueueSettingsDialog(onDismiss = { settingsOpen = false })
    if (offer != null) {
        OfferDialog(
            offer,
            onAccept = { url ->
                actions.add(url)
                onOfferDone()
            },
            onDismiss = onOfferDone,
        )
    }
}

@Composable
private fun DownloadCard(download: Download, place: Int, rules: QueueRules, actions: DownloadActions) {
    val context = LocalContext.current
    Card(Modifier.fillMaxWidth()) {
        Column(Modifier.padding(12.dp)) {
            Text(
                download.name,
                // A link reads left to right, an Arabic name right to left.
                style = MaterialTheme.typography.titleSmall.copy(textDirection = TextDirection.Content),
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
            Spacer(Modifier.width(4.dp))
            val total = download.total
            if (download.status == Status.Running && (total == null || total <= 0)) {
                LinearProgressIndicator(Modifier.fillMaxWidth().padding(vertical = 8.dp))
            } else {
                val fraction = if (total != null && total > 0) download.done.toFloat() / total else 0f
                LinearProgressIndicator(
                    progress = { fraction.coerceIn(0f, 1f) },
                    modifier = Modifier.fillMaxWidth().padding(vertical = 8.dp),
                )
            }
            download.video?.let {
                Text(it.choice, style = MaterialTheme.typography.bodySmall)
            }
            Text(
                "${Notifications.sizeText(context, download.done, total)} · ${statusText(download, place, rules)}",
                style = MaterialTheme.typography.bodySmall,
            )
            FlowRow(horizontalArrangement = Arrangement.spacedBy(4.dp)) {
                val id = download.id
                when (download.status) {
                    Status.Running -> {
                        TextButton(onClick = { actions.pause(id) }) { Text(stringResource(R.string.pause)) }
                        TextButton(onClick = { actions.remove(id) }) { Text(stringResource(R.string.cancel)) }
                    }
                    Status.Queued -> {
                        TextButton(onClick = { actions.startNow(id) }) { Text(stringResource(R.string.start_now)) }
                        if (place > 0) {
                            TextButton(onClick = { actions.moveUp(id) }) { Text(stringResource(R.string.move_up)) }
                        }
                        TextButton(onClick = { actions.pause(id) }) { Text(stringResource(R.string.pause)) }
                        TextButton(onClick = { actions.remove(id) }) { Text(stringResource(R.string.delete)) }
                    }
                    Status.NeedsLocalNetwork -> {
                        TextButton(onClick = { actions.resume(id) }) { Text(stringResource(R.string.allow)) }
                        TextButton(onClick = { actions.remove(id) }) { Text(stringResource(R.string.delete)) }
                    }
                    Status.Paused, Status.Interrupted, Status.Failed -> {
                        TextButton(onClick = { actions.resume(id) }) { Text(stringResource(R.string.resume)) }
                        TextButton(onClick = { actions.remove(id) }) { Text(stringResource(R.string.delete)) }
                    }
                    Status.Merging, Status.Publishing -> {}
                    Status.Done -> {
                        TextButton(onClick = { open(context, download) }) { Text(stringResource(R.string.open)) }
                        TextButton(onClick = { actions.remove(id) }) { Text(stringResource(R.string.remove_from_list)) }
                    }
                }
            }
        }
    }
}

@Composable
private fun statusText(download: Download, place: Int, rules: QueueRules): String = when (download.status) {
    Status.Queued -> when (LocalContext.current.downloads.waitOf()) {
        Wait.Turn -> stringResource(R.string.wait_turn, place + 1)
        Wait.Schedule -> stringResource(R.string.wait_schedule, rules.startAt.format(DateTimeFormatter.ofPattern("HH:mm")))
        Wait.Network -> stringResource(R.string.wait_network)
        Wait.Wifi -> stringResource(R.string.wait_wifi)
    }
    Status.Running -> stringResource(R.string.status_running)
    Status.Interrupted -> stringResource(R.string.status_interrupted)
    Status.Paused -> stringResource(R.string.status_paused)
    Status.NeedsLocalNetwork -> stringResource(R.string.status_needs_local_network)
    Status.Merging -> stringResource(R.string.status_merging)
    Status.Publishing -> stringResource(R.string.status_publishing)
    Status.Done -> stringResource(R.string.status_done)
    Status.Failed -> stringResource(
        when (download.problem) {
            Problem.NETWORK -> R.string.problem_network
            Problem.SERVER -> R.string.problem_server
            Problem.NOT_ENOUGH_SPACE -> R.string.problem_space
            Problem.CERTIFICATE -> R.string.problem_certificate
            Problem.BAD_LINK -> R.string.problem_link
            Problem.OTHER, null -> R.string.problem_other
        },
    )
}

/** An http(s) link with a host: the only kind the engine downloads. */
private fun isLink(text: String): Boolean {
    val uri = text.toUri()
    return (uri.scheme == "http" || uri.scheme == "https") && !uri.host.isNullOrEmpty() && text.none(Char::isWhitespace)
}

/** The clipboard's text, read only when the user taps «Paste». */
private fun pasted(context: Context): String? =
    context.getSystemService(ClipboardManager::class.java)
        ?.primaryClip
        ?.takeIf { it.itemCount > 0 }
        ?.getItemAt(0)
        ?.coerceToText(context)
        ?.toString()
        ?.trim()

private fun open(context: Context, download: Download) {
    val uri = download.uri?.toUri() ?: return
    val intent = Intent(Intent.ACTION_VIEW)
        .setDataAndType(uri, context.contentResolver.getType(uri))
        .addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_ACTIVITY_NEW_TASK)
    try {
        context.startActivity(Intent.createChooser(intent, null).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
    } catch (_: ActivityNotFoundException) {
        // No app opens this kind of file.
    }
}
