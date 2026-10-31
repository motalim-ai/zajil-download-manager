package io.github.motalim_ai.zajil.ui

import android.content.Context
import android.text.format.Formatter
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import io.github.motalim_ai.zajil.BuildConfig
import io.github.motalim_ai.zajil.R
import io.github.motalim_ai.zajil.update.AppUpdate
import kotlinx.coroutines.launch

/** Whether Zajil looks for its own update when it opens (on, as on the desktop; at most daily). */
object UpdatePrefs {
    private fun prefs(context: Context) = context.getSharedPreferences("update", Context.MODE_PRIVATE)

    fun auto(context: Context) = prefs(context).getBoolean("auto", true)

    fun setAuto(context: Context, on: Boolean) = prefs(context).edit().putBoolean("auto", on).apply()

    /** True at most once a day, and only if [auto] is on. */
    fun due(context: Context): Boolean {
        if (!auto(context)) return false
        val now = System.currentTimeMillis()
        val last = prefs(context).getLong("checked_at", 0)
        if (now - last < 24 * 60 * 60 * 1000L) return false
        prefs(context).edit().putLong("checked_at", now).apply()
        return true
    }
}

/** Zajil's own update in Settings: its version, the check, and the steps after it. */
@Composable
fun AppUpdateSection() {
    val context = LocalContext.current
    var auto by remember { mutableStateOf(UpdatePrefs.auto(context)) }
    Column {
        Text(stringResource(R.string.update_version, BuildConfig.VERSION_NAME))
        Row(verticalAlignment = Alignment.CenterVertically) {
            Text(stringResource(R.string.update_auto), Modifier.weight(1f), style = MaterialTheme.typography.bodySmall)
            Switch(checked = auto, onCheckedChange = {
                auto = it
                UpdatePrefs.setAuto(context, it)
            })
        }
        UpdateSteps(showCheck = true)
    }
}

/** The card at the top of the list when an update was found at start. */
@Composable
fun UpdateCard() {
    val state by AppUpdate.state.collectAsStateWithLifecycle()
    if (state is AppUpdate.State.Idle || state is AppUpdate.State.Latest || state is AppUpdate.State.Checking) return
    Card(
        Modifier.fillMaxWidth().padding(bottom = 8.dp),
        colors = CardDefaults.cardColors(containerColor = MaterialTheme.colorScheme.primaryContainer),
    ) {
        Column(Modifier.padding(12.dp)) { UpdateSteps(showCheck = false) }
    }
}

@Composable
private fun UpdateSteps(showCheck: Boolean) {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    val state by AppUpdate.state.collectAsStateWithLifecycle()
    when (val s = state) {
        AppUpdate.State.Idle -> if (showCheck) {
            TextButton(onClick = { scope.launch { AppUpdate.check() } }) { Text(stringResource(R.string.update_check)) }
        }
        AppUpdate.State.Checking -> Text(stringResource(R.string.ytdlp_checking), style = MaterialTheme.typography.bodySmall)
        AppUpdate.State.Latest -> Text(stringResource(R.string.update_latest), style = MaterialTheme.typography.bodySmall)
        is AppUpdate.State.Failed -> Column {
            Text(stringResource(s.message), style = MaterialTheme.typography.bodySmall)
            if (showCheck) {
                TextButton(onClick = { scope.launch { AppUpdate.check() } }) { Text(stringResource(R.string.update_check)) }
            }
        }
        is AppUpdate.State.Offered -> Column {
            Text(
                stringResource(
                    R.string.update_offered,
                    s.update.version,
                    Formatter.formatShortFileSize(context, s.update.size.toLong()),
                ),
                style = MaterialTheme.typography.titleSmall,
            )
            if (s.update.notes.isNotBlank()) Text(s.update.notes, style = MaterialTheme.typography.bodySmall)
            TextButton(onClick = { scope.launch { AppUpdate.download(context, s.update) } }) {
                Text(stringResource(R.string.update_download))
            }
        }
        is AppUpdate.State.Downloading -> Text(
            stringResource(R.string.update_downloading, s.update.version, s.percent),
            style = MaterialTheme.typography.bodySmall,
        )
        is AppUpdate.State.Ready -> Row(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.CenterVertically) {
            Text(stringResource(R.string.update_ready, s.update.version), Modifier.weight(1f), style = MaterialTheme.typography.bodySmall)
            TextButton(onClick = { AppUpdate.install(context, s.apk) }) { Text(stringResource(R.string.update_install)) }
        }
    }
}
