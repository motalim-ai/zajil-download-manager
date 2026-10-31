package io.github.motalim_ai.zajil.ui

import android.app.TimePickerDialog
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.style.TextDirection
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import io.github.motalim_ai.zajil.R
import io.github.motalim_ai.zajil.downloads
import io.github.motalim_ai.zajil.downloads.QueueSettings
import java.time.LocalTime
import java.time.format.DateTimeFormatter

/**
 * A link that came from outside: shared by another app, or found on the
 * clipboard when Zajil opened. Nothing downloads until the user taps
 * «Download»; [url] is null when the shared text had no link.
 */
data class Offer(val url: String?, val fromClipboard: Boolean)

@Composable
fun OfferDialog(offer: Offer, onAccept: (String) -> Unit, onDismiss: () -> Unit) {
    val url = offer.url
    if (url == null) {
        AlertDialog(
            onDismissRequest = onDismiss,
            title = { Text(stringResource(R.string.no_link_title)) },
            text = { Text(stringResource(R.string.no_link_text)) },
            confirmButton = { TextButton(onClick = onDismiss) { Text(stringResource(R.string.ok)) } },
        )
        return
    }
    AlertDialog(
        onDismissRequest = onDismiss,
        title = {
            Text(stringResource(if (offer.fromClipboard) R.string.offer_clipboard_title else R.string.offer_shared_title))
        },
        text = {
            Text(
                url,
                // A link reads left to right.
                style = MaterialTheme.typography.bodyMedium.copy(textDirection = TextDirection.Ltr),
            )
        },
        confirmButton = { TextButton(onClick = { onAccept(url) }) { Text(stringResource(R.string.download)) } },
        dismissButton = { TextButton(onClick = onDismiss) { Text(stringResource(R.string.not_now)) } },
    )
}

/** The queue's rules: how many at once, «Wi-Fi only», and a start time. */
@Composable
fun QueueSettingsDialog(onDismiss: () -> Unit) {
    val context = LocalContext.current
    val settings = context.downloads.settings
    val rules by settings.rules.collectAsStateWithLifecycle()
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text(stringResource(R.string.settings)) },
        text = {
            Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Text(stringResource(R.string.max_running), Modifier.weight(1f))
                    TextButton(
                        onClick = { settings.update { it.copy(maxRunning = (it.maxRunning - 1).coerceAtLeast(1)) } },
                        enabled = rules.maxRunning > 1,
                    ) { Text("−") }
                    Text("${rules.maxRunning}", style = MaterialTheme.typography.titleMedium)
                    TextButton(
                        onClick = { settings.update { it.copy(maxRunning = (it.maxRunning + 1).coerceAtMost(QueueSettings.MAX)) } },
                        enabled = rules.maxRunning < QueueSettings.MAX,
                    ) { Text("+") }
                }
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Column(Modifier.weight(1f)) {
                        Text(stringResource(R.string.wifi_only))
                        Text(stringResource(R.string.wifi_only_hint), style = MaterialTheme.typography.bodySmall)
                    }
                    Switch(checked = rules.wifiOnly, onCheckedChange = { on -> settings.update { it.copy(wifiOnly = on) } })
                }
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Column(Modifier.weight(1f)) {
                        Text(stringResource(R.string.schedule))
                        Text(stringResource(R.string.schedule_hint), style = MaterialTheme.typography.bodySmall)
                    }
                    Switch(
                        checked = rules.scheduled,
                        // Turned on, the queue waits for the next start time.
                        onCheckedChange = { on -> settings.update { it.copy(scheduled = on, open = false) } },
                    )
                }
                if (rules.scheduled) {
                    OutlinedButton(
                        onClick = {
                            TimePickerDialog(
                                context,
                                { _, hour, minute -> settings.update { it.copy(startAt = LocalTime.of(hour, minute), open = false) } },
                                rules.startAt.hour,
                                rules.startAt.minute,
                                true,
                            ).show()
                        },
                        modifier = Modifier.fillMaxWidth().padding(top = 4.dp),
                    ) {
                        Text(stringResource(R.string.schedule_at, rules.startAt.format(DateTimeFormatter.ofPattern("HH:mm"))))
                    }
                }
            }
        },
        confirmButton = { TextButton(onClick = onDismiss) { Text(stringResource(R.string.ok)) } },
    )
}
