package io.github.motalim_ai.zajil.ui

import android.app.TimePickerDialog
import android.text.format.DateFormat
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.selection.toggleable
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.pluralStringResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextDecoration
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import io.github.motalim_ai.zajil.R
import io.github.motalim_ai.zajil.downloads
import io.github.motalim_ai.zajil.downloads.Download
import io.github.motalim_ai.zajil.downloads.NetworkState
import io.github.motalim_ai.zajil.downloads.Queue
import io.github.motalim_ai.zajil.downloads.QueuePlan
import io.github.motalim_ai.zajil.downloads.QueueRules
import io.github.motalim_ai.zajil.downloads.QueueSettings
import io.github.motalim_ai.zajil.downloads.Status
import java.time.DayOfWeek
import java.time.LocalTime

/** The week as the design shows it: Saturday first. */
private val WEEK = listOf(
    DayOfWeek.SATURDAY, DayOfWeek.SUNDAY, DayOfWeek.MONDAY, DayOfWeek.TUESDAY,
    DayOfWeek.WEDNESDAY, DayOfWeek.THURSDAY, DayOfWeek.FRIDAY,
)

/**
 * «الطوابير» (D5): the main queue on the blue card, with how many at
 * once and «Wi-Fi only» at hand, then the other queues, each with its time
 * and days if it is scheduled.
 */
@Composable
fun QueueScreen(bottomPadding: PaddingValues) {
    val center = LocalContext.current.downloads
    val queues by center.settings.queues.collectAsStateWithLifecycle()
    val list by center.list.collectAsStateWithLifecycle()
    val network by center.networkState.collectAsStateWithLifecycle()
    var open by rememberSaveable { mutableStateOf<Long?>(null) }
    var naming by rememberSaveable { mutableStateOf<Long?>(null) }
    var creating by rememberSaveable { mutableStateOf(false) }
    val c = Z.colors

    Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(bottom = bottomPadding.calculateBottomPadding())) {
        TabHeader(stringResource(R.string.queues)) {
            HeaderButton(stringResource(R.string.new_queue), { creating = true }, LineIcon.Plus)
        }
        val main = queues.first()
        MainQueue(main, list.filter { QueuePlan.queueOf(it, queues) == main.id }, network)
        for (queue in queues.drop(1)) {
            OtherQueue(
                queue,
                list.filter { QueuePlan.queueOf(it, queues) == queue.id },
                network,
                expanded = open == queue.id,
                onToggle = { open = if (open == queue.id) null else queue.id },
                onRename = { naming = queue.id },
                onDelete = {
                    center.deleteQueue(queue.id)
                    open = null
                },
            )
        }
        ZText(
            stringResource(R.string.queue_footer),
            Modifier.padding(horizontal = 22.dp, vertical = 16.dp).fillMaxWidth(),
            size = 12.5.sp,
            color = c.muted,
        )
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

/** «2 يحمّلان الآن، 3 في الانتظار», or the queue's state in a word. */
@Composable
private fun queueLine(list: List<Download>, rules: QueueRules, network: NetworkState): String {
    val running = list.count { it.status == Status.Running }
    val waiting = list.count { it.status == Status.Queued }
    val counts = listOfNotNull(
        running.takeIf { it > 0 }?.let { pluralStringResource(R.plurals.queue_downloading, it, it) },
        waiting.takeIf { it > 0 }?.let { pluralStringResource(R.plurals.queue_waiting, it, it) },
    )
    return counts.ifEmpty { listOf(queueState(list, rules, network)) }.joinToString(stringResource(R.string.list_sep))
}

/** The main queue's state, in a word or two. */
@Composable
fun queueState(list: List<Download>, rules: QueueRules, network: NetworkState): String {
    val waiting = list.any { it.status == Status.Queued }
    val running = list.any { it.status == Status.Running }
    return when {
        rules.scheduled && !rules.open && !running -> stringResource(R.string.queue_starts_at, timeText(rules.startAt))
        waiting && !network.connected -> stringResource(R.string.wait_network)
        waiting && rules.wifiOnly && !network.unmetered -> stringResource(R.string.wait_wifi)
        running || waiting -> stringResource(R.string.queue_running)
        else -> stringResource(R.string.queue_empty)
    }
}

@Composable
private fun MainQueue(queue: Queue, list: List<Download>, network: NetworkState) {
    val settings = LocalContext.current.downloads.settings
    val rules = queue.rules
    val c = Z.colors
    val running = list.any { it.status == Status.Running }
    LayeredCard(Modifier.padding(start = 18.dp, end = 18.dp, top = 38.dp), gap = 14.dp) {
        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            Column(Modifier.weight(1f)) {
                ZText(queueName(queue), size = 21.sp, weight = FontWeight.Bold, color = Color.White, maxLines = 1)
                ZText(queueLine(list, rules, network), size = 13.5.sp, color = c.heroSub)
            }
            Row(
                Modifier.height(30.dp).clip(CircleShape).background(Color(0x38FFFFFF)).padding(horizontal = 12.dp),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(6.dp),
            ) {
                Box(Modifier.size(8.dp).clip(CircleShape).background(if (running) Color(0xFF5FDCA0) else Color(0x99FFFFFF)))
                ZText(stringResource(if (running) R.string.queue_working else R.string.queue_resting), size = 13.sp, color = Color.White)
            }
        }
        Column(Modifier.fillMaxWidth().clip(RoundedCornerShape(20.dp)).background(Color(0x24FFFFFF)).padding(horizontal = 14.dp, vertical = 4.dp)) {
            QueueRows(queue, list, onBlue = true)
        }
    }
}

/** A queue's settings, as rows: on the blue card or in a white one. */
@Composable
private fun ColumnScope.QueueRows(queue: Queue, list: List<Download>, onBlue: Boolean, withRun: Boolean = true) {
    val context = LocalContext.current
    val center = context.downloads
    val settings = center.settings
    val rules = queue.rules
    val c = Z.colors
    val ink = if (onBlue) Color.White else c.ink
    val sub = if (onBlue) c.heroSub else c.faint
    val line = if (onBlue) Color(0x2EFFFFFF) else c.lineSoft
    PanelRow(stringResource(R.string.max_running), null, ink, sub) {
        Stepper(rules.maxRunning, 1, QueueSettings.MAX, { n -> settings.update(queue.id) { it.copy(maxRunning = n) } }, onBlue = onBlue)
    }
    RowLine(line)
    PanelRow(stringResource(R.string.wifi_only), stringResource(R.string.wifi_only_hint), ink, sub, onClick = { settings.update(queue.id) { it.copy(wifiOnly = !it.wifiOnly) } }) {
        ZSwitch(rules.wifiOnly, { on -> settings.update(queue.id) { it.copy(wifiOnly = on) } }, onBlue = onBlue)
    }
    RowLine(line)
    val pickTime = {
        TimePickerDialog(
            context,
            { _, hour, minute -> settings.update(queue.id) { it.copy(scheduled = true, startAt = LocalTime.of(hour, minute), open = false) } },
            rules.startAt.hour,
            rules.startAt.minute,
            DateFormat.is24HourFormat(context),
        ).show()
    }
    PanelRow(
        stringResource(R.string.schedule),
        stringResource(if (rules.scheduled) R.string.schedule_hint else R.string.schedule_off_hint),
        ink,
        sub,
        onClick = pickTime,
    ) {
        if (rules.scheduled) ZText(timeText(rules.startAt), size = 15.sp, weight = FontWeight.Bold, color = ink)
        // Turned on, the queue waits for the next start time.
        ZSwitch(rules.scheduled, { on -> settings.update(queue.id) { it.copy(scheduled = on, open = false) } }, onBlue = onBlue)
    }
    if (rules.scheduled) {
        Days(rules.days, onBlue, Modifier.padding(bottom = 12.dp)) { days -> settings.update(queue.id) { it.copy(days = days) } }
    }
    if (withRun) {
        RowLine(line)
        val busy = list.any { it.status == Status.Running || it.status == Status.Queued }
        PanelRow(
            stringResource(if (busy) R.string.stop_queue else R.string.run_queue),
            null,
            ink,
            sub,
        ) {
            SoftButton(
                stringResource(if (busy) R.string.stop_word else R.string.run_word),
                { if (busy) center.stopQueue(queue.id) else center.runQueue(queue.id) },
                color = if (onBlue) c.accent else if (busy) c.red else c.blue,
                background = if (onBlue) Color.White else if (busy) c.redBg else c.sel,
            )
        }
    }
}

@Composable
private fun PanelRow(
    label: String,
    hint: String?,
    ink: Color,
    sub: Color,
    onClick: (() -> Unit)? = null,
    end: @Composable () -> Unit,
) {
    Row(
        Modifier
            .fillMaxWidth()
            .heightIn(min = 56.dp)
            .then(if (onClick != null) Modifier.clickable(onClick = onClick) else Modifier)
            .padding(vertical = 6.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(10.dp),
    ) {
        Column(Modifier.weight(1f)) {
            ZText(label, size = 14.5.sp, color = ink)
            if (hint != null) ZText(hint, size = 12.5.sp, color = sub)
        }
        end()
    }
}

/** The days a scheduled queue starts on: Saturday first; at least one stays on. */
@Composable
private fun Days(days: Set<DayOfWeek>, onBlue: Boolean, modifier: Modifier = Modifier, onChange: (Set<DayOfWeek>) -> Unit) {
    val c = Z.colors
    val locale = LocalConfiguration.current.locales[0]
    Row(modifier.fillMaxWidth(), horizontalArrangement = Arrangement.SpaceBetween) {
        WEEK.forEach { day ->
            val on = day in days
            val name = day.getDisplayName(java.time.format.TextStyle.FULL, locale)
            Box(
                Modifier
                    .size(44.dp)
                    .clip(CircleShape)
                    .toggleable(on, role = Role.Checkbox) { now -> if (now) onChange(days + day) else if (days.size > 1) onChange(days - day) }
                    .semantics { contentDescription = name },
                contentAlignment = Alignment.Center,
            ) {
                Box(
                    Modifier
                        .size(38.dp)
                        .clip(CircleShape)
                        .background(
                            when {
                                on && onBlue -> Color.White
                                on -> c.accent
                                onBlue -> Color(0x24FFFFFF)
                                else -> c.sunken
                            },
                        ),
                    contentAlignment = Alignment.Center,
                ) {
                    ZText(
                        day.getDisplayName(java.time.format.TextStyle.NARROW, locale),
                        size = 14.sp,
                        weight = FontWeight.SemiBold,
                        color = when {
                            on && onBlue -> c.accent
                            on -> Color.White
                            onBlue -> c.heroSub
                            else -> c.chevron
                        },
                        style = TextStyle(textDecoration = if (on) null else TextDecoration.LineThrough),
                    )
                }
            }
        }
    }
}

/** Another queue: its name and what it holds, its time or «run»; its settings when opened. */
@Composable
private fun OtherQueue(
    queue: Queue,
    list: List<Download>,
    network: NetworkState,
    expanded: Boolean,
    onToggle: () -> Unit,
    onRename: () -> Unit,
    onDelete: () -> Unit,
) {
    val center = LocalContext.current.downloads
    val rules = queue.rules
    val c = Z.colors
    val busy = list.any { it.status == Status.Running || it.status == Status.Queued }
    val running = list.any { it.status == Status.Running }
    ZCard(Modifier.padding(start = 18.dp, end = 18.dp, top = 14.dp), padding = PaddingValues(16.dp)) {
        Row(
            Modifier.fillMaxWidth().clip(RoundedCornerShape(16.dp)).clickable(onClick = onToggle),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            if (!rules.scheduled) {
                IconCircle(if (running) LineIcon.Downloads else LineIcon.Pause, if (running) c.sel else c.sunken, if (running) c.blue else c.softInk, size = 42.dp)
            }
            Column(Modifier.weight(1f)) {
                ZText(queueName(queue), size = 17.sp, weight = FontWeight.SemiBold, maxLines = 1)
                val count = list.count { it.status != Status.Done }
                ZText(
                    listOfNotNull(
                        if (!rules.scheduled && !busy) stringResource(R.string.queue_resting) else null,
                        pluralStringResource(R.plurals.queue_count, count, count),
                        stringResource(R.string.wifi_only_short).takeIf { rules.wifiOnly },
                    ).joinToString(stringResource(R.string.list_sep)),
                    size = 13.sp,
                    color = c.faint,
                )
            }
            if (rules.scheduled) {
                Pill(stringResource(R.string.queue_starts_at, timeText(rules.startAt)), c.amberBg, c.amber, icon = LineIcon.Clock)
            } else {
                SoftButton(
                    stringResource(if (busy) R.string.stop_word else R.string.run_word),
                    { if (busy) center.stopQueue(queue.id) else center.runQueue(queue.id) },
                    color = if (busy) c.red else c.blue,
                    background = if (busy) c.redBg else c.sel,
                    height = 38.dp,
                )
            }
        }
        if (rules.scheduled && !expanded) {
            Days(rules.days, onBlue = false, Modifier.padding(top = 12.dp)) { days -> center.settings.update(queue.id) { it.copy(days = days) } }
            // What happens at the time; while it runs, how it is doing.
            ZText(
                if (busy) queueLine(list, rules, network) else stringResource(R.string.schedule_hint),
                Modifier.padding(top = 8.dp),
                size = 12.5.sp,
                color = c.faint,
            )
        }
        if (!expanded) return@ZCard
        RowLine()
        QueueRows(queue, list, onBlue = false, withRun = rules.scheduled)
        RowLine()
        SettingRow(stringResource(R.string.rename_queue), onClick = onRename)
        RowLine()
        SettingRow(
            stringResource(R.string.delete_queue),
            hint = stringResource(R.string.delete_queue_hint),
            onClick = onDelete,
            labelColor = c.red,
            chevron = false,
        )
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
        Column(verticalArrangement = Arrangement.spacedBy(16.dp)) {
            SheetTitle(title)
            ZField(name, { name = it.take(QueueSettings.MAX_NAME) }, stringResource(R.string.queue_name))
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
