package io.github.motalim_ai.zajil.ui

import android.content.ActivityNotFoundException
import android.content.Context
import android.content.Intent
import android.text.format.DateUtils
import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBars
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyRow
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.Icon
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextDirection
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.core.net.toUri
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import io.github.motalim_ai.zajil.R
import io.github.motalim_ai.zajil.downloads
import io.github.motalim_ai.zajil.downloads.BrowserLink
import io.github.motalim_ai.zajil.downloads.Download
import io.github.motalim_ai.zajil.downloads.FileNames
import io.github.motalim_ai.zajil.downloads.NetworkState
import io.github.motalim_ai.zajil.downloads.Queue
import io.github.motalim_ai.zajil.downloads.QueuePlan
import io.github.motalim_ai.zajil.downloads.QueueRules
import io.github.motalim_ai.zajil.downloads.Status
import io.github.motalim_ai.zajil.downloads.Wait
import io.github.motalim_ai.zajil.engine.Problem

/** When a new download starts: now, with the queue, or when the user says. */
enum class StartWhen { Now, Queue, Later }

/** What the screens ask of the app. */
interface DownloadActions {
    /** A link to download: a video page opens the format sheet instead. */
    fun add(url: String, name: String? = null, start: StartWhen = StartWhen.Queue, browser: BrowserLink? = null, queue: Long = Queue.MAIN)
    fun pause(id: Long)
    fun resume(id: Long)
    fun startNow(id: Long)
    fun moveUp(id: Long)
    fun remove(id: Long)
}

/** The downloads: what runs now, the filters, and the list (the approved design's home). */
@Composable
fun HomeScreen(actions: DownloadActions, bottomPadding: PaddingValues) {
    val context = LocalContext.current
    val center = context.downloads
    val list by center.list.collectAsStateWithLifecycle()
    val queues by center.settings.queues.collectAsStateWithLifecycle()
    val network by center.networkState.collectAsStateWithLifecycle()
    val speeds by center.speeds.rates.collectAsStateWithLifecycle()
    var filter by rememberSaveable { mutableStateOf(Filter.All) }
    var searching by rememberSaveable { mutableStateOf(false) }
    var query by rememberSaveable { mutableStateOf("") }
    // Each download's place in its own queue.
    val places = remember(list, queues) {
        QueuePlan.queued(list).groupBy { QueuePlan.queueOf(it, queues) }.values
            .flatMap { inQueue -> inQueue.mapIndexed { i, d -> d.id to i } }.toMap()
    }
    val shown = remember(list, filter, query, searching) {
        list.sortedByDescending { it.id }
            .filter { filter.matches(it) }
            .filter { !searching || query.isBlank() || it.name.contains(query.trim(), ignoreCase = true) }
    }
    val listState = rememberLazyListState()
    // A new download shows at the top: scroll there so that it is seen.
    val newest = list.maxOfOrNull { it.id }
    LaunchedEffect(newest) { if (newest != null) listState.animateScrollToItem(0) }

    Column(Modifier.fillMaxSize().background(Z.colors.bg)) {
        Column(Modifier.background(Z.colors.surface).windowInsetsPadding(WindowInsets.statusBars)) {
            if (searching) {
                SearchBar(query, onQuery = { query = it }, onClose = {
                    searching = false
                    query = ""
                })
            } else {
                AppBar(onSearch = { searching = true })
            }
            NowStrip(list.filter { QueuePlan.queueOf(it, queues) == Queue.MAIN }, speeds, queues.first().rules, network)
            Box(Modifier.fillMaxWidth().height(1.dp).background(Z.colors.line))
        }
        Filters(list, filter) { filter = it }
        UpdateCard()
        if (shown.isEmpty()) {
            ZText(
                stringResource(if (list.isEmpty()) R.string.empty else R.string.nothing_matches),
                Modifier.padding(24.dp),
                color = Z.colors.muted,
            )
        }
        LazyColumn(
            state = listState,
            verticalArrangement = Arrangement.spacedBy(10.dp),
            contentPadding = PaddingValues(
                start = 12.dp,
                end = 12.dp,
                bottom = bottomPadding.calculateBottomPadding() + 88.dp,
            ),
            modifier = Modifier.weight(1f),
        ) {
            items(shown, key = { it.id }) { download ->
                val queue = queues.first { it.id == QueuePlan.queueOf(download, queues) }
                DownloadCard(download, places[download.id] ?: -1, queue, network, speeds[download.id] ?: 0, actions)
            }
        }
    }
}

@Composable
private fun AppBar(onSearch: () -> Unit) {
    val center = LocalContext.current.downloads
    var menu by remember { mutableStateOf(false) }
    Row(
        Modifier.padding(start = 16.dp, end = 8.dp, top = 10.dp, bottom = 4.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(10.dp),
    ) {
        ZajilLogo()
        ZText(stringResource(R.string.app_name), Modifier.weight(1f), size = 21.sp, weight = FontWeight.Bold)
        BarIconButton(LineIcon.Search, stringResource(R.string.search)) { onSearch() }
        Box {
            BarIconButton(LineIcon.More, stringResource(R.string.more_actions)) { menu = true }
            DropdownMenu(expanded = menu, onDismissRequest = { menu = false }, containerColor = Z.colors.surface) {
                MenuItem(stringResource(R.string.resume_all)) { menu = false; center.resumeAll() }
                MenuItem(stringResource(R.string.pause_all)) { menu = false; center.stopAllQueues() }
                MenuItem(stringResource(R.string.remove_finished)) { menu = false; center.removeFinished() }
            }
        }
    }
}

@Composable
private fun MenuItem(text: String, onClick: () -> Unit) {
    DropdownMenuItem(text = { ZText(text) }, onClick = onClick)
}

@Composable
private fun SearchBar(query: String, onQuery: (String) -> Unit, onClose: () -> Unit) {
    val focus = remember { FocusRequester() }
    LaunchedEffect(Unit) { focus.requestFocus() }
    Row(
        Modifier.padding(start = 16.dp, end = 8.dp, top = 10.dp, bottom = 4.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(10.dp),
    ) {
        Icon(lineIcon(LineIcon.Search), contentDescription = null, tint = Z.colors.muted, modifier = Modifier.size(22.dp))
        Box(Modifier.weight(1f).height(48.dp), contentAlignment = Alignment.CenterStart) {
            if (query.isEmpty()) ZText(stringResource(R.string.search_hint), color = Z.colors.muted, size = 16.sp)
            BasicTextField(
                value = query,
                onValueChange = onQuery,
                singleLine = true,
                textStyle = TextStyle(fontFamily = Plex, fontSize = 16.sp, color = Z.colors.ink, textDirection = TextDirection.Content),
                cursorBrush = SolidColor(Z.colors.accent),
                modifier = Modifier.fillMaxWidth().focusRequester(focus),
            )
        }
        Box(
            Modifier
                .size(48.dp)
                .clip(CircleShape)
                .clickable(role = Role.Button, onClick = onClose),
            contentAlignment = Alignment.Center,
        ) {
            ZText(stringResource(R.string.close), size = 13.5.sp, weight = FontWeight.SemiBold, color = Z.colors.accent)
        }
    }
}

/** «Downloading now 2 · 4.8 MB/s · Main queue: running». */
@Composable
private fun NowStrip(list: List<Download>, speeds: Map<Long, Long>, rules: QueueRules, network: NetworkState) {
    val running = list.count { it.status == Status.Running }
    val units = sizeUnits()
    Row(
        Modifier.fillMaxWidth().padding(start = 16.dp, end = 16.dp, top = 4.dp, bottom = 12.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(10.dp),
    ) {
        if (running > 0) {
            Pill(
                stringResource(R.string.downloading_now, running),
                Z.colors.sel,
                Z.colors.accent,
                icon = lineIcon(LineIcon.Downloads),
                height = 30.dp,
                size = 13.sp,
            )
            val total = speeds.values.sum()
            if (total > 0) {
                ZText(stringResource(R.string.per_second, Units.size(total, units)), size = 13.sp, color = Z.colors.muted)
            }
        } else {
            ZText(stringResource(R.string.nothing_running), size = 13.sp, color = Z.colors.muted)
        }
        Spacer(Modifier.weight(1f))
        ZText(
            stringResource(R.string.main_queue_is, queueState(list, rules, network)),
            size = 12.5.sp,
            color = Z.colors.muted,
            maxLines = 1,
        )
    }
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
private fun Filters(list: List<Download>, chosen: Filter, onChoose: (Filter) -> Unit) {
    // The kinds only when there is one of them; «all», «active» and «done» always.
    val counts = Filter.entries.associateWith { f -> list.count { f.matches(it) } }
    val shown = Filter.entries.filter { it in setOf(Filter.All, Filter.Active, Filter.Done) || (counts[it] ?: 0) > 0 }
    LazyRow(
        contentPadding = PaddingValues(horizontal = 16.dp, vertical = 12.dp),
        horizontalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        items(shown) { f ->
            val on = f == chosen
            val shape = CircleShape
            Row(
                Modifier
                    .height(48.dp)
                    .clickable(role = Role.Tab, onClick = { onChoose(f) }),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Row(
                    Modifier
                        .height(34.dp)
                        .clip(shape)
                        .background(if (on) Z.colors.accent else Z.colors.surface)
                        .border(1.dp, if (on) Z.colors.accent else Z.colors.line, shape)
                        .padding(horizontal = 12.dp),
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(6.dp),
                ) {
                    val color = if (on) Z.colors.accentInk else Z.colors.ink
                    ZText(stringResource(filterName(f)), size = 13.5.sp, weight = FontWeight.SemiBold, color = color)
                    ZText("${counts[f] ?: 0}", size = 12.sp, color = color.copy(alpha = 0.8f))
                }
            }
        }
    }
}

private fun filterName(f: Filter) = when (f) {
    Filter.All -> R.string.filter_all
    Filter.Active -> R.string.filter_active
    Filter.Done -> R.string.filter_done
    Filter.Video -> R.string.filter_video
    Filter.Music -> R.string.filter_music
    Filter.Images -> R.string.filter_images
    Filter.Documents -> R.string.filter_documents
    Filter.Programs -> R.string.filter_programs
    Filter.Archives -> R.string.filter_archives
    Filter.Other -> R.string.filter_other
}

@OptIn(ExperimentalFoundationApi::class)
@Composable
private fun DownloadCard(
    download: Download,
    place: Int,
    queue: Queue,
    network: NetworkState,
    speed: Long,
    actions: DownloadActions,
) {
    val context = LocalContext.current
    val c = Z.colors
    var menu by remember { mutableStateOf(false) }
    var moving by remember { mutableStateOf(false) }
    val queues by context.downloads.settings.queues.collectAsStateWithLifecycle()
    // Another queue to move it to: only while it is not done or being made.
    val movable = queues.size > 1 && download.status !in setOf(Status.Done, Status.Merging, Status.Publishing)
    val shape = RoundedCornerShape(16.dp)
    val id = download.id
    val look = statusLook(download.status)
    val openLabel = stringResource(R.string.open)
    val menuLabel = stringResource(R.string.more_actions)
    // Nothing to do while the file is being made.
    val hasMenu = download.status != Status.Merging && download.status != Status.Publishing
    Box {
        Row(
            Modifier
                .fillMaxWidth()
                .clip(shape)
                .background(c.surface)
                .border(1.dp, c.line, shape)
                .combinedClickable(
                    onClickLabel = if (download.status == Status.Done) openLabel else menuLabel,
                    onLongClickLabel = menuLabel,
                    onClick = { if (download.status == Status.Done) open(context, download) else menu = hasMenu },
                    onLongClick = { menu = hasMenu },
                )
                .padding(12.dp),
            horizontalArrangement = Arrangement.spacedBy(12.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            ColorImage(Kind.of(download).icon, 40.dp, Modifier.align(Alignment.Top))
            Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(6.dp)) {
                ZText(
                    // An Arabic name right to left, a Latin one left to right, as one unit.
                    FileNames.display(download.name),
                    size = 15.sp,
                    weight = FontWeight.SemiBold,
                    maxLines = 1,
                )
                Bar(download)
                Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    Pill(stringResource(look.label), look.background(c), look.color(c))
                    ZText(detail(download, place, queue, network, speed), Modifier.weight(1f), size = 12.5.sp, color = c.muted, maxLines = 2)
                }
            }
            when (download.status) {
                Status.Running -> RoundIconButton(ColorIcon.Pause, stringResource(R.string.pause), { actions.pause(id) })
                Status.Paused, Status.Interrupted ->
                    RoundIconButton(ColorIcon.Resume, stringResource(R.string.resume), { actions.resume(id) })
                Status.NeedsLocalNetwork ->
                    RoundIconButton(ColorIcon.Resume, stringResource(R.string.allow), { actions.resume(id) })
                Status.Failed -> RoundIconButton(
                    ColorIcon.Refresh,
                    stringResource(if (download.video != null) R.string.refresh_link else R.string.try_again),
                    { actions.resume(id) },
                )
                else -> {}
            }
        }
        DropdownMenu(expanded = menu, onDismissRequest = { menu = false }, containerColor = c.surface) {
            val item = @Composable { label: Int, action: () -> Unit ->
                MenuItem(stringResource(label)) {
                    menu = false
                    action()
                }
            }
            when (download.status) {
                Status.Running -> {
                    item(R.string.pause) { actions.pause(id) }
                    item(R.string.cancel) { actions.remove(id) }
                }
                Status.Queued -> {
                    item(R.string.start_now) { actions.startNow(id) }
                    if (place > 0) item(R.string.move_up) { actions.moveUp(id) }
                    item(R.string.pause) { actions.pause(id) }
                    item(R.string.delete) { actions.remove(id) }
                }
                Status.NeedsLocalNetwork -> {
                    item(R.string.allow) { actions.resume(id) }
                    item(R.string.delete) { actions.remove(id) }
                }
                Status.Paused, Status.Interrupted, Status.Failed -> {
                    item(R.string.resume) { actions.resume(id) }
                    item(R.string.delete) { actions.remove(id) }
                }
                Status.Merging, Status.Publishing -> {}
                Status.Done -> {
                    item(R.string.open) { open(context, download) }
                    item(R.string.remove_from_list) { actions.remove(id) }
                }
            }
            if (movable) item(R.string.move_to_queue) { moving = true }
        }
    }
    if (moving) {
        ChoiceSheet(
            stringResource(R.string.move_to_queue),
            queues.map { it.id to queueName(it) },
            chosen = queue.id,
            onChoose = { to -> context.downloads.moveToQueue(id, to) },
            onDismiss = { moving = false },
        )
    }
}

/** The progress bar: only while it runs or waits half done, as the design has it. */
@Composable
private fun Bar(download: Download) {
    val c = Z.colors
    val total = download.total
    val color = when (download.status) {
        Status.Running, Status.Merging -> c.blue
        Status.Paused, Status.Interrupted -> c.amberFill
        else -> return
    }
    val modifier = Modifier.fillMaxWidth().height(6.dp).clip(RoundedCornerShape(3.dp))
    if (download.status == Status.Merging || (download.status == Status.Running && (total == null || total <= 0))) {
        LinearProgressIndicator(modifier, color = color, trackColor = c.track, strokeCap = StrokeCap.Round, gapSize = 0.dp)
    } else {
        val fraction = if (total != null && total > 0) download.done.toFloat() / total else 0f
        LinearProgressIndicator(
            progress = { fraction.coerceIn(0f, 1f) },
            modifier = modifier,
            color = color,
            trackColor = c.track,
            strokeCap = StrokeCap.Round,
            gapSize = 0.dp,
            drawStopIndicator = {},
        )
    }
}

/** A status's word and colours. */
private class StatusLook(
    val label: Int,
    val background: (ZajilColors) -> androidx.compose.ui.graphics.Color,
    val color: (ZajilColors) -> androidx.compose.ui.graphics.Color,
)

private fun statusLook(status: Status): StatusLook = when (status) {
    Status.Running -> StatusLook(R.string.chip_running, { it.sel }, { it.accent })
    Status.Merging -> StatusLook(R.string.chip_merging, { it.sel }, { it.accent })
    Status.Publishing -> StatusLook(R.string.chip_publishing, { it.sel }, { it.accent })
    Status.Queued -> StatusLook(R.string.chip_queued, { it.greyBg }, { it.grey })
    Status.Paused, Status.Interrupted -> StatusLook(R.string.chip_paused, { it.amberBg }, { it.amber })
    Status.NeedsLocalNetwork -> StatusLook(R.string.chip_needs_permission, { it.amberBg }, { it.amber })
    Status.Failed -> StatusLook(R.string.chip_failed, { it.redBg }, { it.red })
    Status.Done -> StatusLook(R.string.chip_done, { it.greenBg }, { it.green })
}

/** The line beside the status: sizes, speed and time left, or why it waits. */
@Composable
private fun detail(download: Download, place: Int, queue: Queue, network: NetworkState, speed: Long): String {
    val rules = queue.rules
    val units = sizeUnits()
    val done = Units.size(download.done, units)
    val total = download.total?.takeIf { it > 0 }
    val ofTotal = stringResource(R.string.of_total)
    val progress = if (total != null) Units.progress(download.done, total, units, ofTotal) else done
    val choice = download.video?.choice
    val parts: List<String?> = when (download.status) {
        Status.Running -> listOf(
            progress,
            speed.takeIf { it > 0 }?.let { stringResource(R.string.per_second, Units.size(it, units)) },
            total?.let { Units.left(it - download.done, speed) }?.let { stringResource(R.string.left, it) },
        )
        Status.Paused -> listOf(
            progress,
            stringResource(if (download.done > 0) R.string.resumes_where_stopped else R.string.status_paused),
        )
        Status.Interrupted -> listOf(stringResource(R.string.status_interrupted))
        Status.NeedsLocalNetwork -> listOf(stringResource(R.string.status_needs_local_network_short))
        Status.Merging -> listOf(stringResource(R.string.status_merging))
        Status.Publishing -> listOf(stringResource(R.string.status_publishing))
        Status.Queued -> listOf(
            // Waiting for something else than its turn: which queue, if not the main one.
            queue.name.takeIf { !queue.main && QueuePlan.waitOf(rules, network) != Wait.Turn },
            choice,
            when (QueuePlan.waitOf(rules, network)) {
                Wait.Turn -> if (place <= 0) {
                    stringResource(R.string.next_in_queue, queueName(queue))
                } else {
                    stringResource(R.string.in_queue_before, queueName(queue), place)
                }
                Wait.Schedule -> stringResource(R.string.wait_schedule, timeText(rules.startAt))
                Wait.Network -> stringResource(R.string.wait_network)
                Wait.Wifi -> stringResource(R.string.wait_wifi)
            },
        )
        Status.Failed -> listOf(stringResource(problemText(download.problem)))
        Status.Done -> listOf(
            choice,
            Units.size(download.total ?: download.done, units),
            download.finishedAt?.let { finishedText(LocalContext.current, it) },
        )
    }
    return parts.filterNotNull().joinToString(" · ")
}

private fun finishedText(context: Context, at: Long): String =
    if (DateUtils.isToday(at)) {
        context.getString(R.string.today_at, DateUtils.formatDateTime(context, at, DateUtils.FORMAT_SHOW_TIME))
    } else {
        DateUtils.formatDateTime(context, at, DateUtils.FORMAT_SHOW_DATE or DateUtils.FORMAT_ABBREV_MONTH)
    }

fun problemText(problem: Problem?): Int = when (problem) {
    Problem.NETWORK -> R.string.problem_network
    Problem.SERVER -> R.string.problem_server
    Problem.NOT_ENOUGH_SPACE -> R.string.problem_space
    Problem.CERTIFICATE -> R.string.problem_certificate
    Problem.BAD_LINK -> R.string.problem_link
    Problem.OTHER, null -> R.string.problem_other
}

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
