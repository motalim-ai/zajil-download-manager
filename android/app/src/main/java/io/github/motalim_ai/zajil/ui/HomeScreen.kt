package io.github.motalim_ai.zajil.ui

import android.os.Build
import androidx.compose.animation.core.Animatable
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.gestures.Orientation
import androidx.compose.foundation.gestures.draggable
import androidx.compose.foundation.gestures.rememberDraggableState
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.RowScope
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBars
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.layout.wrapContentHeight
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalLayoutDirection
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.onClick
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.unit.LayoutDirection
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import io.github.motalim_ai.zajil.R
import io.github.motalim_ai.zajil.downloads
import io.github.motalim_ai.zajil.downloads.BrowserLink
import io.github.motalim_ai.zajil.downloads.Download
import io.github.motalim_ai.zajil.downloads.FileNames
import io.github.motalim_ai.zajil.downloads.Look
import io.github.motalim_ai.zajil.downloads.Queue
import io.github.motalim_ai.zajil.downloads.QueuePlan
import io.github.motalim_ai.zajil.downloads.Status
import kotlin.math.roundToInt
import kotlinx.coroutines.launch

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

/**
 * The downloads' home (D5): the app's name with the total speed, the
 * greeting, the download in progress on the blue card, four shortcuts, and
 * the latest downloads by status.
 */
@Composable
fun HomeScreen(
    actions: DownloadActions,
    bottomPadding: PaddingValues,
    onAll: (Filter) -> Unit,
    onDetails: (Long) -> Unit,
    onProtection: () -> Unit,
    onQueues: () -> Unit,
    onClipboard: () -> Unit,
    onNewLink: () -> Unit,
) {
    val center = LocalContext.current.downloads
    val list by center.list.collectAsStateWithLifecycle()
    val speeds by center.speeds.rates.collectAsStateWithLifecycle()
    // The download on the card: the first of those running, in queue order.
    val current = list.filter { Stage.of(it.status) == Stage.Active }.minByOrNull { it.order }
    Column(
        Modifier
            .fillMaxSize()
            .verticalScroll(rememberScrollState())
            .padding(bottom = bottomPadding.calculateBottomPadding()),
    ) {
        Header(speeds.values.sum())
        Column(Modifier.padding(start = 18.dp, end = 18.dp, top = 18.dp), verticalArrangement = Arrangement.spacedBy(4.dp)) {
            ZText(stringResource(R.string.greeting), size = 22.sp, weight = FontWeight.Bold)
            ZText(stringResource(R.string.tagline), size = 15.sp, color = Z.colors.muted)
        }
        Hero(current, current?.let { speeds[it.id] } ?: 0, actions, onDetails, onNewLink)
        UpdateCard()
        Tiles(onClipboard = onClipboard, onPlaylists = { onAll(Filter.Playlists) }, onQueues = onQueues, onProtection = onProtection)
        Activity(list, speeds, onAll, onDetails)
    }
}

/** The logo, «زاجِل» with the total speed now, and the language and look buttons. */
@Composable
private fun Header(speed: Long) {
    val context = LocalContext.current
    val center = context.downloads
    val c = Z.colors
    var asking by rememberSaveable { mutableStateOf(false) }
    val units = sizeUnits()
    Row(
        Modifier
            .fillMaxWidth()
            .windowInsetsPadding(WindowInsets.statusBars)
            .padding(start = 18.dp, end = 18.dp, top = 18.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        ZajilLogo()
        Column(Modifier.weight(1f)) {
            ZText(stringResource(R.string.app_name), size = 17.sp, weight = FontWeight.SemiBold)
            ZText(
                if (speed > 0) {
                    stringResource(R.string.speed_now, stringResource(R.string.per_second, Units.size(speed, units)))
                } else {
                    stringResource(R.string.nothing_running)
                },
                size = 12.5.sp,
                color = if (speed > 0) c.speed else c.muted,
                maxLines = 1,
            )
        }
        // Android keeps a language for each app from Android 13.
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            HeaderButton(stringResource(R.string.language), { asking = true }, LineIcon.Languages)
        }
        HeaderButton(
            stringResource(if (c.dark) R.string.light_mode else R.string.dark_mode),
            { center.prefs.update { it.copy(look = if (c.dark) Look.Light else Look.Dark) } },
            if (c.dark) LineIcon.Sun else LineIcon.Moon,
        )
    }
    if (asking) LanguageSheet(onDismiss = { asking = false })
}

/** The blue card of the download in progress, with its two layers behind it. */
@Composable
private fun Hero(download: Download?, speed: Long, actions: DownloadActions, onDetails: (Long) -> Unit, onNewLink: () -> Unit) {
    val c = Z.colors
    val units = sizeUnits()
    LayeredCard(Modifier.padding(start = 18.dp, end = 18.dp, top = 34.dp)) {
        if (download == null) {
            ZText(stringResource(R.string.hero_idle_title), size = 24.sp, weight = FontWeight.Bold, color = Color.White)
            ZText(stringResource(R.string.hero_idle_hint), size = 14.5.sp, color = c.heroSub)
            Row(
                Modifier
                    .height(56.dp)
                    .clip(CircleShape)
                    .background(Color.White)
                    .clickable(role = Role.Button, onClick = onNewLink)
                    .padding(horizontal = 20.dp),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                ZIcon(LineIcon.Plus, c.accent, 22.dp)
                ZText(stringResource(R.string.new_download), size = 15.sp, weight = FontWeight.SemiBold, color = c.accent)
            }
            return@LayeredCard
        }
        val total = download.total?.takeIf { it > 0 }
        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            ZText(
                kindChip(download),
                Modifier.height(30.dp).clip(RoundedCornerShape(10.dp)).background(Color.White).padding(horizontal = 12.dp).wrapContentHeight(Alignment.CenterVertically),
                size = 13.sp,
                weight = FontWeight.SemiBold,
                color = c.accent,
                maxLines = 1,
            )
            Spacer(Modifier.weight(1f))
            if (speed > 0) HeroPill(LineIcon.BoltFill, Color(0xFFFFD84D), stringResource(R.string.per_second, Units.size(speed, units)))
            total?.let { Units.left(it - download.done, speed) }?.let { HeroPill(LineIcon.Clock, Color.White, stringResource(R.string.left_short, it)) }
        }
        ZText(FileNames.display(download.name), size = 24.sp, weight = FontWeight.Bold, color = Color.White, maxLines = 2)
        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(10.dp)) {
            val fraction = total?.let { download.done.toFloat() / it }
            if (fraction != null) ZText("${(fraction * 100).toInt().coerceIn(0, 100)}%", size = 20.sp, weight = FontWeight.Bold, color = Color.White)
            ThinBar(
                if (download.status == Status.Running) fraction else null,
                Modifier.weight(1f),
                color = Color.White,
                track = Color(0x47FFFFFF),
                height = 6.dp,
            )
            ZText(
                if (total != null) Units.progress(download.done, total, units, stringResource(R.string.of_total)) else Units.size(download.done, units),
                size = 13.sp,
                color = c.heroSub,
                maxLines = 1,
            )
        }
        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(10.dp)) {
            if (download.status == Status.Running) {
                SwipeToPause(Modifier.weight(1f)) { actions.pause(download.id) }
            } else {
                // Being joined or saved: nothing to pause.
                ZText(
                    stringResource(if (download.status == Status.Merging) R.string.status_merging else R.string.status_publishing),
                    Modifier.weight(1f).height(56.dp).clip(CircleShape).background(Color(0x38FFFFFF)).padding(horizontal = 20.dp).wrapContentHeight(Alignment.CenterVertically),
                    size = 14.5.sp,
                    color = Color.White,
                    maxLines = 1,
                )
            }
            val info = stringResource(R.string.download_details)
            Box(
                Modifier
                    .size(56.dp)
                    .clip(CircleShape)
                    .background(Color.White)
                    .clickable(onClickLabel = info, role = Role.Button) { onDetails(download.id) }
                    .semantics { contentDescription = info },
                contentAlignment = Alignment.Center,
            ) {
                ZIcon(LineIcon.Info, Color(0xFF121722), 22.dp)
            }
        }
    }
}

/**
 * The design's blue card with the two layers behind it, peeking out above
 * (13 and 26 dp up, 20 and 40 dp narrower each side).
 */
@Composable
fun LayeredCard(modifier: Modifier = Modifier, gap: Dp = 12.dp, content: @Composable ColumnScope.() -> Unit) {
    val c = Z.colors
    Box(modifier.fillMaxWidth()) {
        Box(Modifier.padding(horizontal = 40.dp).offset(y = (-26).dp).fillMaxWidth().height(60.dp).clip(RoundedCornerShape(28.dp)).background(c.heroOuter))
        Box(Modifier.padding(horizontal = 20.dp).offset(y = (-13).dp).fillMaxWidth().height(60.dp).clip(RoundedCornerShape(30.dp)).background(c.heroInner))
        Column(
            Modifier.fillMaxWidth().clip(RoundedCornerShape(30.dp)).background(c.accent).padding(18.dp),
            verticalArrangement = Arrangement.spacedBy(gap),
            content = content,
        )
    }
}

@Composable
private fun HeroPill(icon: LineIcon, tint: Color, text: String) {
    Row(
        Modifier.height(36.dp).clip(CircleShape).background(Color(0x3DFFFFFF)).padding(horizontal = 11.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(5.dp),
    ) {
        ZIcon(icon, tint, 15.dp)
        ZText(text, size = 13.5.sp, color = Color.White, maxLines = 1)
    }
}

/**
 * «اسحب للإيقاف»: the knob dragged to the line's end pauses the download;
 * let go before, it slides back. TalkBack offers the pause as the slider's action.
 */
@Composable
private fun SwipeToPause(modifier: Modifier, onPause: () -> Unit) {
    val c = Z.colors
    val scope = rememberCoroutineScope()
    val rtl = LocalLayoutDirection.current == LayoutDirection.Rtl
    val density = LocalDensity.current
    val at = remember { Animatable(0f) }
    val label = stringResource(R.string.swipe_to_pause)
    val pause = stringResource(R.string.pause)
    BoxWithConstraints(
        modifier
            .height(56.dp)
            .clip(CircleShape)
            .background(Color(0x38FFFFFF))
            .semantics {
                contentDescription = label
                onClick(pause) {
                    onPause()
                    true
                }
            },
    ) {
        val travel = with(density) { (maxWidth - 56.dp).toPx() }.coerceAtLeast(1f)
        Row(
            Modifier.fillMaxSize().padding(start = 62.dp, end = 10.dp).alpha(1f - at.value / travel),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            ZText(label, Modifier.weight(1f), size = 14.5.sp, color = Color.White, maxLines = 1)
            ZIcon(LineIcon.SwipeHint, Color(0xCCFFFFFF), 18.dp)
        }
        Box(
            Modifier
                .padding(6.dp)
                .offset { IntOffset(at.value.roundToInt(), 0) }
                .size(44.dp)
                .clip(CircleShape)
                .background(c.heroKnob)
                .draggable(
                    orientation = Orientation.Horizontal,
                    state = rememberDraggableState { delta ->
                        scope.launch { at.snapTo((at.value + if (rtl) -delta else delta).coerceIn(0f, travel)) }
                    },
                    onDragStopped = {
                        if (at.value >= travel * 0.8f) {
                            at.animateTo(travel)
                            onPause()
                        }
                        at.animateTo(0f)
                    },
                ),
            contentAlignment = Alignment.Center,
        ) {
            ZIcon(LineIcon.Pause, Color.White, 18.dp)
        }
    }
}

/** The four shortcuts under the card. */
@Composable
private fun Tiles(onClipboard: () -> Unit, onPlaylists: () -> Unit, onQueues: () -> Unit, onProtection: () -> Unit) {
    val c = Z.colors
    Row(Modifier.padding(start = 18.dp, end = 18.dp, top = 16.dp), horizontalArrangement = Arrangement.spacedBy(10.dp)) {
        Tile(stringResource(R.string.tile_clipboard), LineIcon.Clipboard, Color(0xFF7B4BD6), onClipboard)
        Tile(stringResource(R.string.tile_playlists), LineIcon.Playlist, Color(0xFFD4392F), onPlaylists)
        Tile(stringResource(R.string.queues), LineIcon.Queue, c.green, onQueues)
        Tile(stringResource(R.string.group_protection), LineIcon.Shield, Color(0xFF0B8FD1), onProtection)
    }
}

@Composable
private fun RowScope.Tile(label: String, icon: LineIcon, tint: Color, onClick: () -> Unit) {
    Column(
        Modifier.weight(1f).clip(RoundedCornerShape(18.dp)).clickable(role = Role.Button, onClick = onClick),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.spacedBy(6.dp),
    ) {
        Box(Modifier.fillMaxWidth().height(56.dp).card(18.dp), contentAlignment = Alignment.Center) {
            ZIcon(icon, tint, 26.dp)
        }
        ZText(label, size = 13.sp, maxLines = 1)
    }
}

/** «النشاط»: the latest downloads, by status, and «عرض الكل». */
@Composable
private fun Activity(list: List<Download>, speeds: Map<Long, Long>, onAll: (Filter) -> Unit, onDetails: (Long) -> Unit) {
    val c = Z.colors
    var filter by rememberSaveable { mutableStateOf(Filter.All) }
    val shown = remember(list, filter) { list.sortedByDescending { it.id }.filter { filter.matches(it) }.take(3) }
    Column(
        Modifier.padding(start = 18.dp, end = 18.dp, top = 16.dp).fillMaxWidth().card().padding(start = 16.dp, end = 16.dp, top = 14.dp, bottom = 4.dp),
    ) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            ZText(stringResource(R.string.activity), Modifier.weight(1f), size = 16.5.sp, weight = FontWeight.SemiBold)
            ZText(
                stringResource(R.string.see_all),
                Modifier.clip(RoundedCornerShape(8.dp)).clickable(role = Role.Button) { onAll(filter) }.padding(horizontal = 6.dp, vertical = 12.dp),
                size = 13.5.sp,
                color = c.quietLink,
            )
        }
        Row(Modifier.horizontalScroll(rememberScrollState()), horizontalArrangement = Arrangement.spacedBy(6.dp)) {
            Filter.HOME.forEach { f ->
                FilterChip(
                    if (f == Filter.All) null else list.count { f.matches(it) },
                    stringResource(filterName(f)),
                    on = f == filter,
                    onClick = { filter = f },
                )
            }
        }
        if (shown.isEmpty()) {
            ZText(
                stringResource(if (list.isEmpty()) R.string.empty else R.string.nothing_matches),
                Modifier.padding(vertical = 14.dp),
                size = 13.5.sp,
                color = c.muted,
            )
        }
        val context = LocalContext.current
        shown.forEachIndexed { i, download ->
            if (i > 0) RowLine()
            DownloadRow(download, speeds[download.id] ?: 0, withKind = true) {
                if (download.status == Status.Done) openFile(context, download) else onDetails(download.id)
            }
        }
    }
}

/** The places of the queued downloads in their own queues: 0 is next. */
@Composable
fun rememberPlaces(list: List<Download>, queues: List<Queue>): Map<Long, Int> = remember(list, queues) {
    QueuePlan.queued(list).groupBy { QueuePlan.queueOf(it, queues) }.values
        .flatMap { inQueue -> inQueue.mapIndexed { i, d -> d.id to i } }.toMap()
}
