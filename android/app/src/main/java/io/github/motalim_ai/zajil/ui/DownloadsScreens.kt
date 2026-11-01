package io.github.motalim_ai.zajil.ui

import android.app.DownloadManager
import android.content.ActivityNotFoundException
import android.content.Context
import android.content.Intent
import android.text.format.DateUtils
import android.widget.Toast
import androidx.activity.compose.BackHandler
import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.background
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.RowScope
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.navigationBars
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.statusBars
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.layout.wrapContentHeight
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyRow
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
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
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.pluralStringResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.style.TextDirection
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.core.net.toUri
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import io.github.motalim_ai.zajil.R
import io.github.motalim_ai.zajil.downloads
import io.github.motalim_ai.zajil.downloads.Download
import io.github.motalim_ai.zajil.downloads.FileNames
import io.github.motalim_ai.zajil.downloads.NetworkState
import io.github.motalim_ai.zajil.downloads.Publisher
import io.github.motalim_ai.zajil.downloads.Queue
import io.github.motalim_ai.zajil.downloads.QueuePlan
import io.github.motalim_ai.zajil.downloads.Status
import io.github.motalim_ai.zajil.downloads.Wait
import io.github.motalim_ai.zajil.engine.Problem
import io.github.motalim_ai.zajil.engine.segmentParts
import io.github.motalim_ai.zajil.video.Container
import io.github.motalim_ai.zajil.video.VideoProblem
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.withContext

/** A stage's tint and colour: the row's icon circle and its pill. */
@Composable
fun stageColors(stage: Stage): Pair<Color, Color> {
    val c = Z.colors
    return when (stage) {
        Stage.Active -> c.sel to c.blue
        Stage.Done -> c.greenBg to c.green
        Stage.Waiting -> c.amberBg to c.amber
        Stage.Failed -> c.redBg to c.red
    }
}

/**
 * One download in a list (D5): its kind's icon in its status's colours,
 * its name and what it is doing, a thin bar while it runs, and its status
 * or time on the pill at the end.
 */
@OptIn(ExperimentalFoundationApi::class)
@Composable
fun DownloadRow(download: Download, speed: Long, withKind: Boolean, showBar: Boolean = false, onClick: () -> Unit) {
    val stage = Stage.of(download.status)
    val (tint, color) = stageColors(stage)
    val c = Z.colors
    Row(
        Modifier
            .fillMaxWidth()
            .combinedClickable(onClick = onClick, onLongClick = onClick)
            .padding(vertical = 10.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        IconCircle(Kind.of(download).icon, tint, color, size = 42.dp)
        Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(2.dp)) {
            // An Arabic name right to left, a Latin one left to right, as one unit.
            ZText(FileNames.display(download.name), size = 14.5.sp, weight = FontWeight.Medium, maxLines = 1)
            ZText(rowDetail(download, speed, withKind), size = 12.5.sp, color = c.faint, maxLines = 2)
            if (showBar && download.status == Status.Running) {
                val total = download.total?.takeIf { it > 0 }
                ThinBar(total?.let { download.done.toFloat() / it }, Modifier.padding(top = 4.dp))
            }
        }
        Pill(pillText(download), tint, color)
    }
}

/** The pill's word: the percentage while it runs, the time it finished, or its status. */
@Composable
private fun pillText(download: Download): String {
    val total = download.total?.takeIf { it > 0 }
    return when (download.status) {
        Status.Running -> total?.let { "${(download.done * 100 / it).coerceIn(0, 100)}%" } ?: stringResource(R.string.chip_running)
        Status.Merging -> stringResource(R.string.chip_merging)
        Status.Publishing -> stringResource(R.string.chip_publishing)
        Status.Queued -> stringResource(R.string.pill_waiting)
        Status.Paused, Status.Interrupted -> stringResource(R.string.chip_paused)
        Status.NeedsLocalNetwork -> stringResource(R.string.chip_needs_permission)
        Status.Failed -> stringResource(R.string.chip_failed)
        Status.Done -> download.finishedAt?.let { finishedText(LocalContext.current, it) } ?: stringResource(R.string.chip_done)
    }
}

/** The kind's word, with the video's quality: «فيديو 1080p». */
@Composable
fun kindChip(download: Download): String {
    val kind = stringResource(kindName(Kind.of(download)))
    val quality = download.video?.choice?.substringBefore(" · ")?.takeIf { download.video.container != Container.M4a }
    return listOfNotNull(kind, quality).joinToString(" ")
}

fun kindName(kind: Kind) = when (kind) {
    Kind.Video -> R.string.kind_video
    Kind.Music -> R.string.kind_music
    Kind.Images -> R.string.kind_image
    Kind.Documents -> R.string.kind_document
    Kind.Programs -> R.string.kind_program
    Kind.Archives -> R.string.kind_archive
    Kind.Other -> R.string.kind_file
}

/** The line under a download's name: its kind and sizes, its speed, or why it waits. */
@Composable
fun rowDetail(download: Download, speed: Long, withKind: Boolean): String {
    val center = LocalContext.current.downloads
    val list by center.list.collectAsStateWithLifecycle()
    val queues by center.settings.queues.collectAsStateWithLifecycle()
    val network by center.networkState.collectAsStateWithLifecycle()
    val places = rememberPlaces(list, queues)
    val queue = queues.firstOrNull { it.id == QueuePlan.queueOf(download, queues) } ?: queues.first()
    return detail(download, places[download.id] ?: -1, queue, network, speed, withKind)
}

@Composable
private fun detail(download: Download, place: Int, queue: Queue, network: NetworkState, speed: Long, withKind: Boolean): String {
    val rules = queue.rules
    val units = sizeUnits()
    val total = download.total?.takeIf { it > 0 }
    val progress = if (total != null) {
        Units.progress(download.done, total, units, stringResource(R.string.of_total))
    } else {
        Units.size(download.done, units)
    }
    val kind = if (withKind) stringResource(kindName(Kind.of(download))) else null
    val choice = download.video?.choice
    val parts: List<String?> = when (download.status) {
        // A playlist's video whose page is being read now.
        Status.Running if download.video == null && download.plan != null -> listOf(stringResource(R.string.video_reading))
        Status.Running -> listOf(
            kind,
            progress,
            speed.takeIf { it > 0 && !withKind }?.let { stringResource(R.string.per_second, Units.size(it, units)) },
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
        Status.Failed -> listOf(
            download.videoProblem?.let { name -> VideoProblem.entries.firstOrNull { it.name == name } }
                ?.let { stringResource(shortText(it)) }
                ?: stringResource(if (download.unsaved) R.string.problem_not_saved else problemText(download.problem)),
        )
        Status.Done -> listOf(kind, choice, Units.size(download.total ?: download.done, units))
    }
    return parts.filterNotNull().joinToString(stringResource(R.string.list_sep))
}

/** When it finished: its time today or yesterday, its date before; Western digits, as everywhere in Zajil. */
fun finishedText(context: Context, at: Long): String {
    val days = dayOf(System.currentTimeMillis()) - dayOf(at)
    val locale = context.resources.configuration.locales[0]
    val pattern = if (days in 0..1) "h:mm a" else "d MMM"
    val time = java.time.Instant.ofEpochMilli(at).atZone(java.time.ZoneId.systemDefault())
    return time.format(java.time.format.DateTimeFormatter.ofPattern(pattern, locale).withDecimalStyle(java.time.format.DecimalStyle.STANDARD))
}

/** Days since 1970 in the phone's time zone. */
private fun dayOf(millis: Long): Long {
    val zone = java.util.TimeZone.getDefault()
    return (millis + zone.getOffset(millis)) / DateUtils.DAY_IN_MILLIS
}

fun problemText(problem: Problem?): Int = when (problem) {
    Problem.NETWORK -> R.string.problem_network
    Problem.SERVER -> R.string.problem_server
    Problem.NOT_ENOUGH_SPACE -> R.string.problem_space
    Problem.CERTIFICATE -> R.string.problem_certificate
    Problem.BAD_LINK -> R.string.problem_link
    Problem.REFUSED -> R.string.problem_refused
    Problem.OTHER, null -> R.string.problem_other
}

/** A playlist video's problem, in a row's few words. */
private fun shortText(problem: VideoProblem): Int = when (problem) {
    VideoProblem.Private -> R.string.video_short_private
    VideoProblem.SignIn, VideoProblem.NotPublic -> R.string.video_short_sign_in
    VideoProblem.Gone -> R.string.video_short_gone
    VideoProblem.NoFormats -> R.string.video_short_no_format
    VideoProblem.Live -> R.string.video_short_live
    VideoProblem.Network -> R.string.problem_network
    VideoProblem.Drm -> R.string.video_short_drm
    VideoProblem.BotCheck, VideoProblem.Unsupported, VideoProblem.Unreadable -> R.string.video_short_unreadable
}

fun filterName(f: Filter) = when (f) {
    Filter.All -> R.string.filter_all
    Filter.Active -> R.string.filter_active
    Filter.Done -> R.string.filter_done
    Filter.Waiting -> R.string.filter_waiting
    Filter.Failed -> R.string.filter_failed
    Filter.Playlists -> R.string.tile_playlists
    Filter.Video -> R.string.filter_video
    Filter.Music -> R.string.filter_music
    Filter.Images -> R.string.filter_images
    Filter.Documents -> R.string.filter_documents
    Filter.Programs -> R.string.filter_programs
    Filter.Archives -> R.string.filter_archives
    Filter.Other -> R.string.filter_other
}

/**
 * Opens a finished download in the app the user picks. An app package
 * (APK) is not: handed to Android's installer by Zajil, it would install
 * under Zajil's «install unknown apps» permission, which is for Zajil's
 * own update only (ADR-0012, sections 8 and 11). The system's Downloads
 * opens instead, where the user installs it with that app's permission.
 */
fun openFile(context: Context, download: Download) {
    val uri = download.uri?.toUri() ?: return
    val type = context.contentResolver.getType(uri)
    if (type == APK_TYPE || download.name.endsWith(".apk", ignoreCase = true)) {
        Toast.makeText(context, R.string.apk_open_in_files, Toast.LENGTH_LONG).show()
        runCatching {
            context.startActivity(Intent(DownloadManager.ACTION_VIEW_DOWNLOADS).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
        }
        return
    }
    val intent = Intent(Intent.ACTION_VIEW)
        .setDataAndType(uri, type)
        .addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_ACTIVITY_NEW_TASK)
    try {
        context.startActivity(Intent.createChooser(intent, null).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
    } catch (_: ActivityNotFoundException) {
        // No app opens this kind of file.
    }
}

private const val APK_TYPE = "application/vnd.android.package-archive"

/** A day's group in «All downloads»: today, yesterday, a date, or before Zajil kept the day. */
private sealed interface Day {
    data class On(val day: Long) : Day
    data object Earlier : Day
}

/**
 * «كل التحميلات» (D5): the filters by status (and by kind, playlist, when
 * there are some), the downloads by day, a search, and «resume all», «pause
 * all», «remove finished».
 */
@Composable
fun AllDownloads(
    filter: Filter,
    onFilter: (Filter) -> Unit,
    onBack: () -> Unit,
    onDetails: (Long) -> Unit,
    actions: DownloadActions,
    bottomPadding: PaddingValues,
) {
    val context = LocalContext.current
    val center = context.downloads
    val c = Z.colors
    val list by center.list.collectAsStateWithLifecycle()
    val speeds by center.speeds.rates.collectAsStateWithLifecycle()
    var searching by rememberSaveable { mutableStateOf(false) }
    var query by rememberSaveable { mutableStateOf("") }
    var menu by remember { mutableStateOf(false) }
    BackHandler(enabled = searching) {
        searching = false
        query = ""
    }
    val shown = remember(list, filter, query, searching) {
        list.sortedByDescending { it.id }
            .filter { filter.matches(it) }
            .filter { !searching || query.isBlank() || it.name.contains(query.trim(), ignoreCase = true) }
    }
    val groups = remember(shown) {
        shown.groupBy { d -> (d.finishedAt ?: d.addedAt)?.let { Day.On(dayOf(it)) } ?: Day.Earlier }
            .entries.sortedByDescending { (day, _) -> (day as? Day.On)?.day ?: Long.MIN_VALUE }
    }
    val chips = buildList {
        addAll(Filter.STAGES)
        if (list.any { Filter.Playlists.matches(it) }) add(Filter.Playlists)
        addAll(Filter.KINDS.filter { k -> list.any { k.matches(it) } })
    }
    Column(Modifier.fillMaxSize()) {
        if (searching) {
            Row(
                Modifier.fillMaxWidth().windowInsetsPadding(WindowInsets.statusBars).padding(start = 18.dp, end = 18.dp, top = 18.dp),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(12.dp),
            ) {
                HeaderButton(stringResource(R.string.close), {
                    searching = false
                    query = ""
                }, LineIcon.Back)
                ZField(
                    query,
                    { query = it },
                    stringResource(R.string.search_hint),
                    Modifier.weight(1f).card(24.dp),
                    icon = LineIcon.Search,
                    imeAction = ImeAction.Search,
                    background = c.surface,
                )
            }
        } else {
            SubHeader(stringResource(R.string.all_downloads), onBack) {
                Row(horizontalArrangement = Arrangement.spacedBy(10.dp)) {
                    HeaderButton(stringResource(R.string.search), { searching = true }, LineIcon.Search)
                    Box {
                        HeaderButton(stringResource(R.string.more_actions), { menu = true }, LineIcon.More)
                        DropdownMenu(expanded = menu, onDismissRequest = { menu = false }, containerColor = c.surface) {
                            MenuItem(stringResource(R.string.resume_all)) { menu = false; center.resumeAll() }
                            MenuItem(stringResource(R.string.pause_all)) { menu = false; center.stopAllQueues() }
                            MenuItem(stringResource(R.string.remove_finished)) { menu = false; center.removeFinished() }
                        }
                    }
                }
            }
        }
        LazyRow(
            contentPadding = PaddingValues(horizontal = 18.dp),
            horizontalArrangement = Arrangement.spacedBy(6.dp),
            modifier = Modifier.padding(top = 8.dp),
        ) {
            items(chips) { f ->
                FilterChip(
                    if (f == Filter.All) null else list.count { f.matches(it) },
                    stringResource(filterName(f)),
                    on = f == filter,
                    onClick = { onFilter(f) },
                    raised = true,
                    height = 36.dp,
                )
            }
        }
        LazyColumn(
            Modifier.weight(1f),
            contentPadding = PaddingValues(start = 18.dp, end = 18.dp, bottom = bottomPadding.calculateBottomPadding()),
        ) {
            if (shown.isEmpty()) {
                item {
                    ZText(
                        stringResource(
                            when {
                                list.isEmpty() -> R.string.empty
                                filter == Filter.Playlists -> R.string.no_playlists
                                else -> R.string.nothing_matches
                            },
                        ),
                        Modifier.padding(vertical = 24.dp, horizontal = 4.dp),
                        color = c.muted,
                    )
                }
            }
            groups.forEach { (day, rows) ->
                item(key = "day-$day") {
                    ZText(
                        dayName(context, day),
                        Modifier.padding(start = 4.dp, end = 4.dp, top = 18.dp, bottom = 8.dp),
                        size = 13.5.sp,
                        weight = FontWeight.SemiBold,
                        color = c.muted,
                    )
                }
                itemsIndexed(rows, key = { _, d -> d.id }) { i, download ->
                    val top = if (i == 0) 24.dp else 0.dp
                    val bottom = if (i == rows.lastIndex) 24.dp else 0.dp
                    Column(
                        Modifier
                            .fillMaxWidth()
                            .clip(RoundedCornerShape(topStart = top, topEnd = top, bottomStart = bottom, bottomEnd = bottom))
                            .background(c.surface)
                            .padding(horizontal = 16.dp),
                    ) {
                        if (i > 0) RowLine()
                        DownloadRow(download, speeds[download.id] ?: 0, withKind = false, showBar = true) {
                            if (download.status == Status.Done) openFile(context, download) else onDetails(download.id)
                        }
                    }
                }
            }
        }
    }
}

@Composable
private fun dayName(context: Context, day: Day): String = when (day) {
    Day.Earlier -> stringResource(R.string.day_earlier)
    is Day.On -> when (dayOf(System.currentTimeMillis()) - day.day) {
        0L -> stringResource(R.string.day_today)
        1L -> stringResource(R.string.day_yesterday)
        else -> DateUtils.formatDateTime(context, day.day * DateUtils.DAY_IN_MILLIS + DateUtils.DAY_IN_MILLIS / 2, DateUtils.FORMAT_SHOW_DATE)
    }
}

@Composable
private fun MenuItem(text: String, onClick: () -> Unit) {
    DropdownMenuItem(text = { ZText(text) }, onClick = onClick)
}

/**
 * «تفاصيل التحميل» (D5): the download on the blue card with each
 * connection's part of the file, its speed and time left, where it comes
 * from and goes, and what can be done with it at the bottom.
 */
@Composable
fun DetailsScreen(id: Long, actions: DownloadActions, onBack: () -> Unit) {
    val context = LocalContext.current
    val center = context.downloads
    val c = Z.colors
    val list by center.list.collectAsStateWithLifecycle()
    val queues by center.settings.queues.collectAsStateWithLifecycle()
    val speeds by center.speeds.rates.collectAsStateWithLifecycle()
    val prefs by center.prefs.prefs.collectAsStateWithLifecycle()
    val download = list.firstOrNull { it.id == id }
    LaunchedEffect(download == null) { if (download == null) onBack() }
    BackHandler(onBack = onBack)
    if (download == null) return
    val units = sizeUnits()
    val speed = speeds[id] ?: 0
    val total = download.total?.takeIf { it > 0 }
    val queue = queues.firstOrNull { it.id == QueuePlan.queueOf(download, queues) } ?: queues.first()
    val places = rememberPlaces(list, queues)
    var menu by remember { mutableStateOf(false) }
    var moving by remember { mutableStateOf(false) }
    // Each connection's part, from the engine's state file, while it runs.
    var parts by remember { mutableStateOf<List<Float>>(emptyList()) }
    val stateFile = download.video?.streams?.firstOrNull { !it.finished }?.stateFile ?: download.stateFile
    LaunchedEffect(download.status, stateFile) {
        parts = emptyList()
        while (download.status == Status.Running && stateFile != null) {
            parts = withContext(Dispatchers.IO) { runCatching { segmentParts(stateFile) }.getOrDefault(emptyList()) }
            delay(1000)
        }
    }
    val movable = queues.size > 1 && download.status !in setOf(Status.Done, Status.Merging, Status.Publishing)
    val menuItems = buildList<Pair<Int, () -> Unit>> {
        if (download.status == Status.Queued && (places[id] ?: 0) > 0) add(R.string.move_up to { actions.moveUp(id) })
        if (movable) add(R.string.move_to_queue to { moving = true })
    }

    Column(Modifier.fillMaxSize()) {
        SubHeader(stringResource(R.string.download_details), onBack) {
            if (menuItems.isNotEmpty()) {
                Box {
                    HeaderButton(stringResource(R.string.more_actions), { menu = true }, LineIcon.More)
                    DropdownMenu(expanded = menu, onDismissRequest = { menu = false }, containerColor = c.surface) {
                        menuItems.forEach { (label, action) ->
                            MenuItem(stringResource(label)) {
                                menu = false
                                action()
                            }
                        }
                    }
                }
            } else {
                Spacer(Modifier.width(48.dp))
            }
        }
        Column(
            Modifier.weight(1f).verticalScroll(rememberScrollState()).padding(horizontal = 18.dp).padding(top = 18.dp, bottom = 16.dp),
            verticalArrangement = Arrangement.spacedBy(14.dp),
        ) {
            Column(
                Modifier.fillMaxWidth().clip(RoundedCornerShape(30.dp)).background(c.accent).padding(18.dp),
                verticalArrangement = Arrangement.spacedBy(12.dp),
            ) {
                Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    ZText(
                        kindChip(download),
                        Modifier.height(30.dp).clip(RoundedCornerShape(10.dp)).background(Color.White).padding(horizontal = 12.dp)
                            .wrapContentHeight(Alignment.CenterVertically),
                        size = 13.sp,
                        weight = FontWeight.SemiBold,
                        color = c.accent,
                        maxLines = 1,
                    )
                    ZText(hostOf(download.page ?: download.url), size = 13.sp, color = c.heroSub, maxLines = 1, style = TextStyle(textDirection = TextDirection.Ltr))
                }
                ZText(FileNames.display(download.name), size = 21.sp, weight = FontWeight.Bold, color = Color.White, maxLines = 3)
                Row(verticalAlignment = Alignment.Bottom, horizontalArrangement = Arrangement.spacedBy(10.dp)) {
                    if (total != null) {
                        ZText("${(download.done * 100 / total).coerceIn(0, 100)}%", size = 40.sp, weight = FontWeight.Bold, color = Color.White)
                    }
                    ZText(
                        if (total != null) Units.progress(download.done, total, units, stringResource(R.string.of_total)) else Units.size(download.done, units),
                        Modifier.padding(bottom = 6.dp),
                        size = 13.5.sp,
                        color = c.heroSub,
                    )
                }
                if (parts.size > 1) {
                    Row(horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                        parts.forEach { part ->
                            ThinBar(part, Modifier.weight(1f), color = Color.White, track = Color(0x3DFFFFFF), height = 8.dp)
                        }
                    }
                    val working = parts.count { it < 1f }.coerceAtLeast(1)
                    ZText(pluralStringResource(R.plurals.connections_working, working, "$working"), size = 13.sp, color = c.heroSub)
                } else {
                    val fraction = total?.let { download.done.toFloat() / it }
                    // Moving without an end while it runs with no known size.
                    ThinBar(
                        if (download.status == Status.Running && fraction == null) null else fraction ?: 0f,
                        color = Color.White,
                        track = Color(0x3DFFFFFF),
                        height = 8.dp,
                    )
                    ZText(rowDetail(download, speed, withKind = false), size = 13.sp, color = c.heroSub)
                }
            }
            Row(horizontalArrangement = Arrangement.spacedBy(10.dp)) {
                if (download.status == Status.Running) {
                    Stat(LineIcon.Bolt, c.amberBg, c.amber, if (speed > 0) stringResource(R.string.per_second, Units.size(speed, units)) else "—", stringResource(R.string.speed_label))
                    Stat(LineIcon.Clock, c.sel, c.blue, total?.let { Units.left(it - download.done, speed) } ?: "—", stringResource(R.string.time_left))
                } else {
                    val (tint, color) = stageColors(Stage.of(download.status))
                    Stat(LineIcon.File, c.sel, c.blue, Units.size(download.total ?: download.done, units), stringResource(R.string.size_label))
                    Stat(LineIcon.Clock, tint, color, pillOrStatus(download), stringResource(R.string.status_label))
                }
            }
            ZCard {
                InfoRow(LineIcon.Link, stringResource(R.string.source), hostOf(download.page ?: download.url))
                RowLine()
                val base = prefs.saveTree?.let(Publisher::shownTree) ?: "Download/Zajil"
                InfoRow(LineIcon.Folder, stringResource(R.string.save_place), listOfNotNull(base, download.folder).joinToString("/"))
                if (download.status != Status.Done) {
                    RowLine()
                    InfoRow(LineIcon.Queues, stringResource(R.string.queue_label), queueName(queue))
                }
            }
        }
        Bottom(download, actions, onBack)
    }
    if (moving) {
        ChoiceSheet(
            stringResource(R.string.move_to_queue),
            queues.map { it.id to queueName(it) },
            chosen = queue.id,
            onChoose = { to -> center.moveToQueue(id, to) },
            onDismiss = { moving = false },
        )
    }
}

@Composable
private fun pillOrStatus(download: Download): String = when (download.status) {
    Status.Done -> download.finishedAt?.let { finishedText(LocalContext.current, it) } ?: stringResource(R.string.chip_done)
    else -> pillText(download)
}

@Composable
private fun RowScope.Stat(icon: LineIcon, tint: Color, color: Color, value: String, label: String) {
    Column(
        Modifier.weight(1f).card(22.dp).padding(14.dp),
        verticalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        IconCircle(icon, tint, color, size = 34.dp, iconSize = 18.dp)
        Column {
            ZText(value, size = 19.sp, weight = FontWeight.Bold, maxLines = 1)
            ZText(label, size = 12.5.sp, color = Z.colors.muted, maxLines = 1)
        }
    }
}

@Composable
private fun InfoRow(icon: LineIcon, label: String, value: String) {
    Row(
        Modifier.fillMaxWidth().padding(vertical = 11.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        IconCircle(icon, Z.colors.sunken, Z.colors.softInk, size = 36.dp)
        ZText(label, Modifier.weight(1f), size = 15.sp, weight = FontWeight.Medium, maxLines = 1)
        ZText(value, Modifier.widthIn(max = 200.dp), size = 14.sp, color = Z.colors.muted, maxLines = 1, style = TextStyle(textDirection = TextDirection.Content))
    }
}

/** The buttons at the bottom: what can be done with the download as it is. */
@Composable
private fun Bottom(download: Download, actions: DownloadActions, onBack: () -> Unit) {
    val context = LocalContext.current
    val c = Z.colors
    val id = download.id
    val main: Triple<Int, LineIcon?, () -> Unit>? = when (download.status) {
        Status.Running -> Triple(R.string.pause, LineIcon.Pause) { actions.pause(id) }
        Status.Queued -> Triple(R.string.start_now, null) { actions.startNow(id) }
        Status.Paused, Status.Interrupted -> Triple(R.string.resume, LineIcon.PlayFill) { actions.resume(id) }
        Status.NeedsLocalNetwork -> Triple(R.string.allow, null) { actions.resume(id) }
        // A finished file not saved yet is only saved: no new link.
        Status.Failed -> Triple(if (download.video != null && !download.unsaved) R.string.refresh_link else R.string.try_again, LineIcon.Reload) { actions.resume(id) }
        Status.Done -> Triple(R.string.open, null) { openFile(context, download) }
        Status.Merging, Status.Publishing -> null
    }
    if (main == null) return
    val quiet = when (download.status) {
        Status.Running -> R.string.cancel
        Status.Done -> R.string.remove_from_list
        else -> R.string.delete
    }
    Row(
        Modifier
            .fillMaxWidth()
            .windowInsetsPadding(WindowInsets.navigationBars)
            .padding(start = 18.dp, end = 18.dp, bottom = 26.dp, top = 4.dp),
        horizontalArrangement = Arrangement.spacedBy(10.dp),
    ) {
        MainButton(stringResource(main.first), main.third, Modifier.weight(1f), icon = main.second)
        QuietButton(
            stringResource(quiet),
            onClick = {
                actions.remove(id)
                onBack()
            },
            modifier = Modifier.widthIn(min = 120.dp),
            color = if (download.status == Status.Done) c.ink else c.red,
            raised = true,
        )
    }
}

