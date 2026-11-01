package io.github.motalim_ai.zajil.ui

import androidx.activity.compose.BackHandler
import androidx.compose.animation.animateColorAsState
import androidx.compose.animation.core.Animatable
import androidx.compose.animation.core.spring
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.asPaddingValues
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.navigationBars
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.Matrix
import androidx.compose.ui.graphics.Path
import androidx.compose.ui.graphics.vector.PathParser
import androidx.compose.ui.layout.layout
import androidx.compose.ui.platform.LocalLayoutDirection
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.selected
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.Constraints
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.unit.LayoutDirection
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import io.github.motalim_ai.zajil.R
import io.github.motalim_ai.zajil.browser.BrowserController
import io.github.motalim_ai.zajil.browser.Found
import kotlinx.coroutines.launch

/** The sections of the bottom bar. */
enum class Tab(val icon: LineIcon, val label: Int) {
    Downloads(LineIcon.Downloads, R.string.tab_downloads),
    Browse(LineIcon.Globe, R.string.browse),
    Queues(LineIcon.Queues, R.string.queues),
    Settings(LineIcon.Settings, R.string.settings),
}

/** What the screens below the bar leave free for it: the bar, its margin, and a little air. */
private val BAR_SPACE = 90.dp + 14.dp + 12.dp

/**
 * The app's screen (D5): the section the user is in, the floating bar
 * with «New link» in its notch, and the screens opened from a section
 * (all downloads, a download's details, the protection).
 */
@Composable
fun ZajilScreen(
    actions: DownloadActions,
    onNewLink: () -> Unit,
    onClipboard: () -> Unit,
    browser: BrowserController,
    onFound: (Found) -> Unit,
    browseRequests: Int = 0,
) {
    var tab by rememberSaveable { mutableStateOf(Tab.Downloads) }
    // A link opened in the browser from elsewhere (the video sheet's «Open it in Browse»).
    LaunchedEffect(browseRequests) { if (browseRequests > 0) tab = Tab.Browse }
    var signIns by rememberSaveable { mutableStateOf(false) }
    // «All downloads», with its filter, while it is open.
    var all by rememberSaveable { mutableStateOf<Filter?>(null) }
    var details by rememberSaveable { mutableStateOf<Long?>(null) }
    var protecting by rememberSaveable { mutableStateOf(false) }
    val page by browser.page.collectAsStateWithLifecycle()
    val nav = WindowInsets.navigationBars.asPaddingValues().calculateBottomPadding()
    val bottom = PaddingValues(bottom = BAR_SPACE + nav)

    // Back from another section: the downloads.
    BackHandler(enabled = tab != Tab.Downloads) { tab = Tab.Downloads }
    BackHandler(enabled = tab == Tab.Downloads && all != null) { all = null }

    ZBackground {
        val shown = details
        when {
            protecting -> ProtectionScreen(onBack = { protecting = false })
            shown != null -> DetailsScreen(shown, actions, onBack = { details = null })
            else -> when (tab) {
                Tab.Downloads -> {
                    val filter = all
                    if (filter != null) {
                        AllDownloads(filter, onFilter = { all = it }, onBack = { all = null }, onDetails = { details = it }, actions = actions, bottomPadding = bottom)
                    } else {
                        HomeScreen(
                            actions,
                            bottom,
                            onAll = { all = it },
                            onDetails = { details = it },
                            onProtection = { protecting = true },
                            onQueues = { tab = Tab.Queues },
                            onClipboard = onClipboard,
                            onNewLink = onNewLink,
                        )
                    }
                }
                Tab.Browse -> BrowseScreen(browser, onFound, onSignIns = { signIns = true }, bottom)
                Tab.Queues -> QueueScreen(bottom)
                Tab.Settings -> SettingsScreen(bottom, browser, onSignIns = { signIns = true }, onProtection = { protecting = true })
            }
        }
        // The browser's page has its own bar; the screens opened from a section have none.
        if (!protecting && shown == null && !(tab == Tab.Browse && page != null)) {
            BottomBar(
                tab,
                onTab = {
                    tab = it
                    all = null
                },
                onAdd = onNewLink,
                modifier = Modifier.align(Alignment.BottomCenter).padding(start = 14.dp, end = 14.dp, bottom = 14.dp + nav),
            )
        }
    }
    if (signIns) SignInsSheet(browser, onDismiss = { signIns = false })
}

/**
 * The bar's shape for a bar [width] wide (the design's is 362): a pill 66
 * high with a round notch cut around the centre button, as the hand-over's
 * path, its ends and notch kept where the design has them.
 */
internal fun barPath(width: Float): String {
    val cx = width / 2
    val end = width - 33
    return "M33 22H${cx - 53.85f}A14 14 0 0 1 ${cx - 39.89f} 34.96A40 40 0 0 0 ${cx + 39.89f} 34.96" +
        "A14 14 0 0 1 ${cx + 53.85f} 22H${end}A33 33 0 0 1 $end 88H33A33 33 0 0 1 33 22Z"
}

/**
 * The floating bar: settings and queues on the left, the add button in the
 * notch, the browser and the downloads on the right, in this order in both
 * languages (the owner's choice).
 */
@Composable
private fun BottomBar(tab: Tab, onTab: (Tab) -> Unit, onAdd: () -> Unit, modifier: Modifier) {
    val c = Z.colors
    BoxWithConstraints(modifier.fillMaxWidth().height(90.dp)) {
        val width = maxWidth
        val path = remember(width) { PathParser().parsePathString(barPath(width.value)).toPath() }
        CompositionLocalProvider(LocalLayoutDirection provides LayoutDirection.Ltr) {
            Box(
                Modifier
                    .size(width, 90.dp)
                    .drawBehind {
                        val scaled = Path().apply {
                            addPath(path)
                            transform(Matrix().apply { scale(density, density) })
                        }
                        drawPathShadow(scaled, c.barShadow, 18.dp, 10.dp)
                        drawPath(scaled, c.surface)
                    },
            ) {
                val places = listOf(Tab.Settings to 15.dp, Tab.Queues to 81.dp, Tab.Browse to width - 133.dp, Tab.Downloads to width - 67.dp)
                places.forEach { (_, x) -> TabWell(x) }
                ChosenBlob(places.first { it.first == tab }.second)
                places.take(2).forEach { (t, x) -> TabButton(t, tab, onTab, x) }
                val add = stringResource(R.string.new_link)
                Box(
                    Modifier
                        .offset(x = width / 2 - 32.dp)
                        .size(64.dp)
                        .clip(CircleShape)
                        .background(c.accent)
                        .clickable(onClickLabel = add, role = Role.Button, onClick = onAdd)
                        .semantics { contentDescription = add },
                    contentAlignment = Alignment.Center,
                ) {
                    ZIcon(LineIcon.Plus, Color.White, 26.dp)
                }
                places.drop(2).forEach { (t, x) -> TabButton(t, tab, onTab, x) }
            }
        }
    }
}

/** A tab's round well, under the chosen one's blue. */
@Composable
private fun TabWell(x: Dp) {
    Box(Modifier.offset(x = x, y = 29.dp).size(52.dp).clip(CircleShape).background(Z.colors.sunken))
}

/**
 * The chosen tab's blue, which flows to the next one chosen: its leading
 * edge goes first and its trailing edge follows, so it stretches on the way
 * and settles round. With animations off, it is just there.
 */
@Composable
private fun ChosenBlob(x: Dp) {
    val still = reducedMotion()
    val start = remember { Animatable(x.value) }
    val end = remember { Animatable(x.value + 52f) }
    LaunchedEffect(x) {
        if (still) {
            start.snapTo(x.value)
            end.snapTo(x.value + 52f)
            return@LaunchedEffect
        }
        val forward = x.value > start.value
        val lead = spring<Float>(dampingRatio = 0.8f, stiffness = 900f)
        val trail = spring<Float>(dampingRatio = 0.8f, stiffness = 380f)
        launch { start.animateTo(x.value, if (forward) trail else lead) }
        launch { end.animateTo(x.value + 52f, if (forward) lead else trail) }
    }
    Box(
        Modifier
            .offset { IntOffset(start.value.dp.roundToPx(), 29.dp.roundToPx()) }
            .layout { measurable, _ ->
                val w = (end.value - start.value).coerceAtLeast(0f).dp.roundToPx()
                val placeable = measurable.measure(Constraints.fixed(w, 52.dp.roundToPx()))
                layout(placeable.width, placeable.height) { placeable.place(0, 0) }
            }
            .clip(CircleShape)
            .background(Z.colors.accent),
    )
}

@Composable
private fun TabButton(t: Tab, chosen: Tab, onTab: (Tab) -> Unit, x: Dp) {
    val c = Z.colors
    val on = t == chosen
    val label = stringResource(t.label)
    val tint by animateColorAsState(if (on) Color.White else c.softInk, label = "tab tint")
    Box(
        Modifier
            .offset(x = x, y = 29.dp)
            .size(52.dp)
            .clip(CircleShape)
            .semantics {
                selected = on
                contentDescription = label
            }
            .clickable(role = Role.Tab, onClick = { onTab(t) }),
        contentAlignment = Alignment.Center,
    ) {
        ZIcon(t.icon, tint, 26.dp)
    }
}
