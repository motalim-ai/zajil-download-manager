package io.github.motalim_ai.zajil.ui

import android.content.ActivityNotFoundException
import android.content.ClipboardManager
import android.content.Context
import android.content.Intent
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
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
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
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
import io.github.motalim_ai.zajil.downloads.Status
import io.github.motalim_ai.zajil.engine.Problem

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun DownloadsScreen(
    onAdd: (String) -> Unit,
    onPause: (Long) -> Unit,
    onResume: (Long) -> Unit,
    onRemove: (Long) -> Unit,
) {
    val context = LocalContext.current
    val list by context.downloads.list.collectAsStateWithLifecycle()
    var link by rememberSaveable { mutableStateOf("") }
    val canAdd = isLink(link.trim())
    val add = {
        if (canAdd) {
            onAdd(link.trim())
            link = ""
        }
    }

    Scaffold(topBar = { TopAppBar(title = { Text(stringResource(R.string.app_name)) }) }) { padding ->
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
            if (list.isEmpty()) {
                Text(
                    stringResource(R.string.empty),
                    style = MaterialTheme.typography.bodyMedium,
                    modifier = Modifier.padding(top = 24.dp),
                )
            }
            LazyColumn(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                items(list.sortedByDescending { it.id }, key = { it.id }) { download ->
                    DownloadCard(download, onPause, onResume, onRemove)
                }
            }
        }
    }
}

@Composable
private fun DownloadCard(
    download: Download,
    onPause: (Long) -> Unit,
    onResume: (Long) -> Unit,
    onRemove: (Long) -> Unit,
) {
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
            Text(
                "${Notifications.sizeText(context, download.done, total)} · ${statusText(download)}",
                style = MaterialTheme.typography.bodySmall,
            )
            Row(horizontalArrangement = Arrangement.spacedBy(4.dp)) {
                when (download.status) {
                    Status.Running -> {
                        TextButton(onClick = { onPause(download.id) }) { Text(stringResource(R.string.pause)) }
                        TextButton(onClick = { onRemove(download.id) }) { Text(stringResource(R.string.cancel)) }
                    }
                    Status.NeedsLocalNetwork -> {
                        TextButton(onClick = { onResume(download.id) }) { Text(stringResource(R.string.allow)) }
                        TextButton(onClick = { onRemove(download.id) }) { Text(stringResource(R.string.delete)) }
                    }
                    Status.Paused, Status.Interrupted, Status.Failed -> {
                        TextButton(onClick = { onResume(download.id) }) { Text(stringResource(R.string.resume)) }
                        TextButton(onClick = { onRemove(download.id) }) { Text(stringResource(R.string.delete)) }
                    }
                    Status.Publishing -> {}
                    Status.Done -> {
                        TextButton(onClick = { open(context, download) }) { Text(stringResource(R.string.open)) }
                        TextButton(onClick = { onRemove(download.id) }) { Text(stringResource(R.string.remove_from_list)) }
                    }
                }
            }
        }
    }
}

@Composable
private fun statusText(download: Download): String = when (download.status) {
    Status.Running -> stringResource(R.string.status_running)
    Status.Interrupted -> stringResource(R.string.status_interrupted)
    Status.Paused -> stringResource(R.string.status_paused)
    Status.NeedsLocalNetwork -> stringResource(R.string.status_needs_local_network)
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
