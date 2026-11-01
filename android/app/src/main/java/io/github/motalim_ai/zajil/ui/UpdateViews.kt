package io.github.motalim_ai.zajil.ui

import android.content.Context
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
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

/**
 * Zajil's own update in Settings: whether it looks daily, its version, the
 * check, and the steps after it.
 */
@Composable
fun AppUpdateRows() {
    val context = LocalContext.current
    var auto by remember { mutableStateOf(UpdatePrefs.auto(context)) }
    SettingRow(
        stringResource(R.string.update_auto),
        hint = stringResource(R.string.update_version, BuildConfig.VERSION_NAME),
        value = stringResource(if (auto) R.string.update_daily else R.string.update_off),
        valueColor = if (auto) Z.colors.accent else Z.colors.muted,
        onClick = {
            auto = !auto
            UpdatePrefs.setAuto(context, auto)
        },
    )
    RowLine()
    UpdateSteps(inSettings = true)
}

/** The card at the top of the list when an update was found at start. */
@Composable
fun UpdateCard() {
    val state by AppUpdate.state.collectAsStateWithLifecycle()
    if (state is AppUpdate.State.Idle || state is AppUpdate.State.Latest || state is AppUpdate.State.Checking) return
    val shape = RoundedCornerShape(16.dp)
    Column(
        Modifier
            .padding(horizontal = 12.dp)
            .padding(bottom = 10.dp)
            .fillMaxWidth()
            .clip(shape)
            .background(Z.colors.sel)
            .border(1.dp, Z.colors.line, shape),
    ) {
        UpdateSteps(inSettings = false)
    }
}

@Composable
private fun UpdateSteps(inSettings: Boolean) {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    val state by AppUpdate.state.collectAsStateWithLifecycle()
    val check = { scope.launch { AppUpdate.check() } }
    val muted = Z.colors.muted
    when (val s = state) {
        AppUpdate.State.Idle -> if (inSettings) {
            SettingRow(stringResource(R.string.update_check), value = stringResource(R.string.check_now), onClick = { check() })
        }
        AppUpdate.State.Checking -> SettingRow(stringResource(R.string.update_check), value = stringResource(R.string.ytdlp_checking), valueColor = muted)
        AppUpdate.State.Latest -> SettingRow(
            stringResource(R.string.update_check),
            hint = stringResource(R.string.update_latest),
            value = stringResource(R.string.check_now),
            onClick = { check() },
        )
        is AppUpdate.State.Failed -> SettingRow(
            stringResource(R.string.update_check),
            hint = stringResource(s.message),
            value = if (inSettings) stringResource(R.string.check_now) else null,
            onClick = if (inSettings) ({ check() }) else null,
        )
        is AppUpdate.State.Offered -> Column(Modifier.padding(vertical = 6.dp)) {
            SettingRow(
                stringResource(R.string.update_offered, s.update.version, Units.size(s.update.size.toLong(), sizeUnits())),
                hint = s.update.notes.takeIf { it.isNotBlank() },
                value = stringResource(R.string.update_download),
                onClick = { scope.launch { AppUpdate.download(context, s.update) } },
            )
        }
        is AppUpdate.State.Downloading -> SettingRow(
            stringResource(R.string.update_downloading, s.update.version, s.percent),
            valueColor = muted,
        )
        is AppUpdate.State.Ready -> Row(verticalAlignment = Alignment.CenterVertically) {
            SettingRow(
                stringResource(R.string.update_ready, s.update.version),
                value = stringResource(R.string.update_install),
                onClick = { AppUpdate.install(context, s.apk) },
            )
        }
    }
}

/** The green note at the end of Settings. */
@Composable
fun PromiseNote() {
    val c = Z.colors
    Row(
        Modifier
            .fillMaxWidth()
            .clip(RoundedCornerShape(16.dp))
            .background(c.greenBg)
            .padding(horizontal = 14.dp, vertical = 12.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(10.dp),
    ) {
        androidx.compose.material3.Icon(
            lineIcon(LineIcon.Shield),
            contentDescription = null,
            tint = c.green,
            modifier = Modifier.size(22.dp),
        )
        ZText(stringResource(R.string.promise), size = 13.sp, color = c.green, weight = FontWeight.Normal)
    }
}
