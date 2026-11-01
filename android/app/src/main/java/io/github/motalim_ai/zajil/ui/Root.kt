package io.github.motalim_ai.zajil.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.navigationBars
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.FabPosition
import androidx.compose.material3.Icon
import androidx.compose.material3.Scaffold
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.shadow
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.selected
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import io.github.motalim_ai.zajil.R
import io.github.motalim_ai.zajil.browser.BrowserController
import io.github.motalim_ai.zajil.browser.Found

/** The sections of the bottom bar. */
enum class Tab(val icon: LineIcon, val label: Int) {
    Downloads(LineIcon.Downloads, R.string.tab_downloads),
    Browse(LineIcon.Globe, R.string.browse),
    Queues(LineIcon.Queues, R.string.queues),
    Settings(LineIcon.Settings, R.string.settings),
}

/** The app's screen: the section the user is in, the bottom bar, and «New link» on the downloads. */
@Composable
fun ZajilScreen(
    actions: DownloadActions,
    onNewLink: () -> Unit,
    browser: BrowserController,
    onFound: (Found) -> Unit,
    browseRequests: Int = 0,
) {
    var tab by rememberSaveable { mutableStateOf(Tab.Downloads) }
    // A link opened in the browser from elsewhere (the video sheet's «Open it in Browse»).
    LaunchedEffect(browseRequests) { if (browseRequests > 0) tab = Tab.Browse }
    var signIns by rememberSaveable { mutableStateOf(false) }
    Scaffold(
        containerColor = Z.colors.bg,
        contentWindowInsets = WindowInsets(0),
        bottomBar = { BottomBar(tab) { tab = it } },
        floatingActionButton = { if (tab == Tab.Downloads) NewLinkButton(onNewLink) },
        // The design's «inline start»: on the right in Arabic, on the left in English.
        floatingActionButtonPosition = FabPosition.Start,
    ) { padding ->
        when (tab) {
            Tab.Downloads -> HomeScreen(actions, padding)
            Tab.Browse -> BrowseScreen(browser, onFound, onSignIns = { signIns = true }, padding)
            Tab.Queues -> QueueScreen(padding)
            Tab.Settings -> SettingsScreen(padding, browser, onSignIns = { signIns = true })
        }
    }
    if (signIns) SignInsSheet(browser, onDismiss = { signIns = false })
}

@Composable
private fun NewLinkButton(onClick: () -> Unit) {
    val c = Z.colors
    val shape = RoundedCornerShape(18.dp)
    Row(
        Modifier
            .shadow(10.dp, shape, ambientColor = Color(0x7310263D), spotColor = Color(0x7310263D))
            .clip(shape)
            .background(c.fab)
            .clickable(role = Role.Button, onClick = onClick)
            .height(56.dp)
            .padding(start = 16.dp, end = 20.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(10.dp),
    ) {
        ColorImage(ColorIcon.Add, 26.dp)
        ZText(stringResource(R.string.new_link), size = 15.sp, weight = FontWeight.SemiBold, color = c.fabInk)
    }
}

@Composable
private fun BottomBar(tab: Tab, onTab: (Tab) -> Unit) {
    val c = Z.colors
    Column(Modifier.background(c.surface).windowInsetsPadding(WindowInsets.navigationBars)) {
        Box(Modifier.fillMaxWidth().height(1.dp).background(c.line))
        Row(Modifier.fillMaxWidth().height(76.dp)) {
            Tab.entries.forEach { t ->
                val on = t == tab
                Column(
                    Modifier
                        .weight(1f)
                        .height(76.dp)
                        .semantics { selected = on }
                        .clickable(role = Role.Tab, onClick = { onTab(t) }),
                    horizontalAlignment = Alignment.CenterHorizontally,
                    verticalArrangement = Arrangement.spacedBy(4.dp, Alignment.CenterVertically),
                ) {
                    Box(
                        Modifier
                            .size(width = 60.dp, height = 30.dp)
                            .clip(RoundedCornerShape(15.dp))
                            .background(if (on) c.sel else Color.Transparent),
                        contentAlignment = Alignment.Center,
                    ) {
                        Icon(lineIcon(t.icon), contentDescription = null, tint = if (on) c.accent else c.muted, modifier = Modifier.size(22.dp))
                    }
                    ZText(
                        stringResource(t.label),
                        size = 12.5.sp,
                        weight = if (on) FontWeight.SemiBold else FontWeight.Normal,
                        color = if (on) c.accent else c.muted,
                    )
                }
            }
        }
    }
}
