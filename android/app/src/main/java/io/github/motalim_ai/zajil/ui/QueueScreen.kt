package io.github.motalim_ai.zajil.ui

import android.app.TimePickerDialog
import android.text.format.DateFormat
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.statusBars
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Switch
import androidx.compose.material3.SwitchDefaults
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.pluralStringResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import io.github.motalim_ai.zajil.R
import io.github.motalim_ai.zajil.downloads
import io.github.motalim_ai.zajil.downloads.QueueSettings
import io.github.motalim_ai.zajil.downloads.Status
import java.time.LocalTime

/**
 * The queue (A2) as the approved design shows it: one card, the main
 * queue, with its state, «Run» or «Stop», how many at once, «Wi-Fi only»
 * and a daily time. Zajil for Android has one queue; the design's second
 * («Night queue») and «New queue» wait for queues of their own.
 */
@Composable
fun QueueScreen(bottomPadding: PaddingValues) {
    val context = LocalContext.current
    val center = context.downloads
    val settings = center.settings
    val rules by settings.rules.collectAsStateWithLifecycle()
    val list by center.list.collectAsStateWithLifecycle()
    val network by center.networkState.collectAsStateWithLifecycle()
    val c = Z.colors
    val running = list.count { it.status == Status.Running }
    val waiting = list.count { it.status == Status.Queued }
    val busy = running > 0 || waiting > 0

    Column(Modifier.fillMaxSize().background(c.bg)) {
        TitleBar(stringResource(R.string.queues))
        Column(
            Modifier
                .verticalScroll(rememberScrollState())
                .padding(horizontal = 12.dp, vertical = 14.dp)
                .padding(bottom = bottomPadding.calculateBottomPadding()),
            verticalArrangement = Arrangement.spacedBy(14.dp),
        ) {
            ZCard(radius = 18.dp) {
                Row(
                    Modifier.padding(14.dp),
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(12.dp),
                ) {
                    ColorImage(if (rules.scheduled) ColorIcon.QueueTimed else ColorIcon.Queue, 40.dp)
                    Column(Modifier.weight(1f)) {
                        ZText(stringResource(R.string.main_queue), size = 16.5.sp, weight = FontWeight.Bold)
                        val parts = listOfNotNull(
                            queueState(list, rules, network),
                            running.takeIf { it > 0 }?.let { pluralStringResource(R.plurals.queue_downloading, it, it) },
                            waiting.takeIf { it > 0 }?.let { pluralStringResource(R.plurals.queue_waiting, it, it) },
                        )
                        ZText(parts.joinToString(" · "), size = 12.5.sp, color = c.muted)
                    }
                    if (busy) {
                        RoundIconButton(ColorIcon.Stop, stringResource(R.string.stop_queue), { center.stopQueue() })
                    } else {
                        RoundIconButton(ColorIcon.Resume, stringResource(R.string.run_queue), { center.runQueue() })
                    }
                }
                RowLine()
                SettingRow(stringResource(R.string.max_running)) {
                    Stepper(rules.maxRunning, 1, QueueSettings.MAX) { n -> settings.update { it.copy(maxRunning = n) } }
                }
                RowLine()
                SettingRow(
                    stringResource(R.string.wifi_only),
                    hint = stringResource(R.string.wifi_only_hint),
                    onClick = { settings.update { it.copy(wifiOnly = !it.wifiOnly) } },
                ) {
                    Switch(
                        checked = rules.wifiOnly,
                        onCheckedChange = { on -> settings.update { it.copy(wifiOnly = on) } },
                        colors = switchColors(),
                    )
                }
                RowLine()
                SettingRow(
                    stringResource(R.string.schedule),
                    hint = stringResource(if (rules.scheduled) R.string.schedule_hint else R.string.schedule_off_hint),
                    leading = { ColorImage(ColorIcon.Schedule, 26.dp) },
                    minHeight = 64.dp,
                    onClick = {
                        TimePickerDialog(
                            context,
                            { _, hour, minute ->
                                settings.update { it.copy(scheduled = true, startAt = LocalTime.of(hour, minute), open = false) }
                            },
                            rules.startAt.hour,
                            rules.startAt.minute,
                            DateFormat.is24HourFormat(context),
                        ).show()
                    },
                ) {
                    if (rules.scheduled) {
                        ZText(timeText(rules.startAt), size = 15.sp, weight = FontWeight.Bold, color = c.accent)
                    }
                    Switch(
                        checked = rules.scheduled,
                        // Turned on, the queue waits for the next start time.
                        onCheckedChange = { on -> settings.update { it.copy(scheduled = on, open = false) } },
                        colors = switchColors(),
                    )
                }
            }
            ZText(
                stringResource(R.string.queue_footer),
                Modifier.padding(horizontal = 6.dp).fillMaxWidth(),
                size = 12.5.sp,
                color = c.muted,
            )
        }
    }
}

/** A screen's title on the white bar: «Queues», «Settings». */
@Composable
fun TitleBar(title: String) {
    Column(Modifier.background(Z.colors.surface).windowInsetsPadding(WindowInsets.statusBars)) {
        ZText(
            title,
            Modifier.padding(start = 16.dp, end = 16.dp, top = 18.dp, bottom = 14.dp),
            size = 21.sp,
            weight = FontWeight.Bold,
        )
        RowLine()
    }
}

@Composable
fun switchColors() = SwitchDefaults.colors(
    checkedTrackColor = Z.colors.accent,
    checkedThumbColor = Z.colors.accentInk,
    uncheckedTrackColor = Z.colors.sunken,
    uncheckedBorderColor = Z.colors.outline,
    uncheckedThumbColor = Z.colors.muted,
)
