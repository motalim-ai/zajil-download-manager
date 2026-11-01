package io.github.motalim_ai.zajil.ui

import android.app.TimePickerDialog
import android.text.format.DateFormat
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBars
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Icon
import androidx.compose.material3.Switch
import androidx.compose.material3.SwitchDefaults
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableLongStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.pluralStringResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.style.TextDirection
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import io.github.motalim_ai.zajil.R
import io.github.motalim_ai.zajil.downloads
import io.github.motalim_ai.zajil.downloads.Download
import io.github.motalim_ai.zajil.downloads.NetworkState
import io.github.motalim_ai.zajil.downloads.Queue
import io.github.motalim_ai.zajil.downloads.QueuePlan
import io.github.motalim_ai.zajil.downloads.QueueSettings
import io.github.motalim_ai.zajil.downloads.Status
import java.time.LocalTime

/**
 * The queues (A2; several from 0.3.0) as the approved design shows them:
 * «New queue» in the bar, and a card for each queue. The open one has its
 * state, «Run» or «Stop», how many at once, «Wi-Fi only» and a daily time;
 * the others show what they are in one line («Night queue»: starts 1:00 ·
 * Wi-Fi only · 5 downloads), and open with a tap.
 */
@Composable
fun QueueScreen(bottomPadding: PaddingValues) {
    val center = LocalContext.current.downloads
    val queues by center.settings.queues.collectAsStateWithLifecycle()
    val list by center.list.collectAsStateWithLifecycle()
    val network by center.networkState.collectAsStateWithLifecycle()
    var open by rememberSaveable { mutableLongStateOf(Queue.MAIN) }
    var naming by rememberSaveable { mutableStateOf<Long?>(null) }
    var creating by rememberSaveable { mutableStateOf(false) }
    val c = Z.colors

    Column(Modifier.fillMaxSize().background(c.bg)) {
        TitleBar(stringResource(R.string.queues)) {
            PillButton(stringResource(R.string.new_queue), LineIcon.Plus) { creating = true }
        }
        Column(
            Modifier
                .verticalScroll(rememberScrollState())
                .padding(horizontal = 12.dp, vertical = 14.dp)
                .padding(bottom = bottomPadding.calculateBottomPadding()),
            verticalArrangement = Arrangement.spacedBy(14.dp),
        ) {
            for (queue in queues) {
                val mine = list.filter { QueuePlan.queueOf(it, queues) == queue.id }
                QueueCard(
                    queue,
                    mine,
                    network,
                    expanded = queue.id == open || queues.none { it.id == open },
                    onOpen = { open = queue.id },
                    onRename = { naming = queue.id },
                    onDelete = {
                        center.deleteQueue(queue.id)
                        open = Queue.MAIN
                    },
                )
            }
            ZText(
                stringResource(R.string.queue_footer),
                Modifier.padding(horizontal = 6.dp).fillMaxWidth(),
                size = 12.5.sp,
                color = c.muted,
            )
        }
    }
    if (creating) {
        NameSheet(stringResource(R.string.new_queue), "", stringResource(R.string.create_queue), onDismiss = { creating = false }) { name ->
            open = center.createQueue(name)
        }
    }
    naming?.let { id ->
        val queue = queues.firstOrNull { it.id == id }
        if (queue != null) {
            NameSheet(stringResource(R.string.rename_queue), queue.name, stringResource(R.string.save_name), onDismiss = { naming = null }) { name ->
                center.settings.rename(id, name)
            }
        }
    }
}

@Composable
private fun QueueCard(
    queue: Queue,
    list: List<Download>,
    network: NetworkState,
    expanded: Boolean,
    onOpen: () -> Unit,
    onRename: () -> Unit,
    onDelete: () -> Unit,
) {
    val context = LocalContext.current
    val center = context.downloads
    val settings = center.settings
    val rules = queue.rules
    val c = Z.colors
    val running = list.count { it.status == Status.Running }
    val waiting = list.count { it.status == Status.Queued }
    val busy = running > 0 || waiting > 0
    ZCard(radius = 18.dp) {
        Row(
            Modifier.clickable(enabled = !expanded, onClick = onOpen).padding(14.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            ColorImage(if (rules.scheduled) ColorIcon.QueueTimed else ColorIcon.Queue, 40.dp)
            Column(Modifier.weight(1f)) {
                ZText(queueName(queue), size = 16.5.sp, weight = FontWeight.Bold, maxLines = 1)
                val parts = if (expanded) {
                    listOfNotNull(
                        queueState(list, rules, network),
                        running.takeIf { it > 0 }?.let { pluralStringResource(R.plurals.queue_downloading, it, it) },
                        waiting.takeIf { it > 0 }?.let { pluralStringResource(R.plurals.queue_waiting, it, it) },
                    )
                } else {
                    // The design's one line: when it starts, Wi-Fi, how many it has.
                    listOfNotNull(
                        stringResource(R.string.queue_starts_at, timeText(rules.startAt)).takeIf { rules.scheduled },
                        stringResource(R.string.wifi_only_short).takeIf { rules.wifiOnly },
                        list.count { it.status != Status.Done }.let { pluralStringResource(R.plurals.queue_count, it, it) },
                    )
                }
                ZText(parts.joinToString(" · "), size = 12.5.sp, color = c.muted)
            }
            if (expanded) {
                if (busy) {
                    RoundIconButton(ColorIcon.Stop, stringResource(R.string.stop_queue), { center.stopQueue(queue.id) })
                } else {
                    RoundIconButton(ColorIcon.Resume, stringResource(R.string.run_queue), { center.runQueue(queue.id) })
                }
            }
        }
        if (!expanded) return@ZCard
        RowLine()
        SettingRow(stringResource(R.string.max_running)) {
            Stepper(rules.maxRunning, 1, QueueSettings.MAX) { n -> settings.update(queue.id) { it.copy(maxRunning = n) } }
        }
        RowLine()
        SettingRow(
            stringResource(R.string.wifi_only),
            hint = stringResource(R.string.wifi_only_hint),
            onClick = { settings.update(queue.id) { it.copy(wifiOnly = !it.wifiOnly) } },
        ) {
            Switch(
                checked = rules.wifiOnly,
                onCheckedChange = { on -> settings.update(queue.id) { it.copy(wifiOnly = on) } },
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
                        settings.update(queue.id) { it.copy(scheduled = true, startAt = LocalTime.of(hour, minute), open = false) }
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
                onCheckedChange = { on -> settings.update(queue.id) { it.copy(scheduled = on, open = false) } },
                colors = switchColors(),
            )
        }
        if (!queue.main) {
            RowLine()
            SettingRow(stringResource(R.string.rename_queue), onClick = onRename) {}
            RowLine()
            SettingRow(
                stringResource(R.string.delete_queue),
                hint = stringResource(R.string.delete_queue_hint),
                onClick = onDelete,
                labelColor = c.red,
            ) {}
        }
    }
}

/** A queue's name: the main queue's is the app's, in the app's language. */
@Composable
fun queueName(queue: Queue): String = if (queue.main) stringResource(R.string.main_queue) else queue.name

/** A short name, typed in a sheet: a new queue's, or a queue's new one. */
@Composable
private fun NameSheet(title: String, initial: String, action: String, onDismiss: () -> Unit, onDone: (String) -> Unit) {
    var name by rememberSaveable { mutableStateOf(initial) }
    val ok = name.isNotBlank()
    ZSheet(onDismiss = onDismiss, label = title) {
        Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
            ZText(title, size = 19.sp, weight = FontWeight.Bold)
            SheetField(name, { name = it.take(QueueSettings.MAX_NAME) }, stringResource(R.string.queue_name), KeyboardType.Text, TextDirection.Content)
            Row(horizontalArrangement = Arrangement.spacedBy(10.dp)) {
                MainButton(
                    action,
                    onClick = {
                        if (ok) {
                            onDone(name.trim())
                            onDismiss()
                        }
                    },
                    modifier = Modifier.weight(2f),
                    enabled = ok,
                )
                QuietButton(stringResource(R.string.cancel_sheet), onClick = onDismiss, modifier = Modifier.weight(1f))
            }
        }
    }
}

/** The bar's action: an outlined pill with an icon («New queue»). */
@Composable
fun PillButton(text: String, icon: LineIcon, onClick: () -> Unit) {
    val c = Z.colors
    Row(
        Modifier.heightIn(min = 48.dp).clip(CircleShape).clickable(role = Role.Button, onClick = onClick),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Row(
            Modifier
                .height(36.dp)
                .clip(CircleShape)
                .background(c.sel)
                .border(1.dp, c.accent, CircleShape)
                .padding(horizontal = 12.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(6.dp),
        ) {
            Icon(lineIcon(icon), null, tint = c.accent, modifier = Modifier.size(16.dp))
            ZText(text, size = 13.sp, weight = FontWeight.SemiBold, color = c.accent)
        }
    }
}

/** A screen's title on the white bar: «Queues», «Settings», and its action if any. */
@Composable
fun TitleBar(title: String, action: (@Composable () -> Unit)? = null) {
    Column(Modifier.background(Z.colors.surface).windowInsetsPadding(WindowInsets.statusBars)) {
        Row(
            Modifier.padding(start = 16.dp, end = 12.dp).heightIn(min = 60.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            ZText(title, Modifier.weight(1f).padding(top = 4.dp), size = 21.sp, weight = FontWeight.Bold)
            action?.invoke()
        }
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
