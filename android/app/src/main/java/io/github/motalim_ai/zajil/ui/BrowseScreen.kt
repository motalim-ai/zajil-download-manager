package io.github.motalim_ai.zajil.ui

import android.view.ViewGroup
import androidx.activity.compose.BackHandler
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
import androidx.compose.foundation.layout.asPaddingValues
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.navigationBars
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBars
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.key
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.platform.LocalSoftwareKeyboardController
import androidx.compose.ui.res.pluralStringResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextDirection
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.compose.ui.viewinterop.AndroidView
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import io.github.motalim_ai.zajil.R
import io.github.motalim_ai.zajil.browser.BrowserController
import io.github.motalim_ai.zajil.browser.Finds
import io.github.motalim_ai.zajil.browser.Found
import io.github.motalim_ai.zajil.browser.MediaKind
import io.github.motalim_ai.zajil.browser.PageState
import io.github.motalim_ai.zajil.browser.Shortcut
import io.github.motalim_ai.zajil.browser.SiteIcons
import io.github.motalim_ai.zajil.downloads.FileNames
import io.github.motalim_ai.zajil.video.PlaylistLinks
import java.net.URI
import java.net.URLDecoder

/** The letter tiles' colours, for a site whose own icon Zajil does not have yet. */
private val TILES = listOf(0xFFD42F2C, 0xFF1D63D8, 0xFFB0399B, 0xFF2A3A44, 0xFF121722, 0xFFE08A06, 0xFF1A9AD6).map(::Color)

/** «المتصفح» (A6; D5): the browser's home, or the page with its found videos. */
@Composable
fun BrowseScreen(browser: BrowserController, onFound: (Found) -> Unit, onSignIns: () -> Unit, bottomPadding: PaddingValues) {
    val page by browser.page.collectAsStateWithLifecycle()
    if (page == null) BrowseHome(browser, bottomPadding) else BrowsePage(browser, onFound, onSignIns)
}

@Composable
private fun BrowseHome(browser: BrowserController, bottomPadding: PaddingValues) {
    val c = Z.colors
    val private by browser.privateMode.collectAsStateWithLifecycle()
    val shortcuts by browser.store.shortcuts.collectAsStateWithLifecycle()
    val recent by browser.store.recent.collectAsStateWithLifecycle()
    val icons by browser.store.icons.icons.collectAsStateWithLifecycle()
    // The shortcuts' own icons, asked once of each site itself; not in private mode.
    LaunchedEffect(shortcuts, private) { if (!private) browser.store.icons.fetchMissing(shortcuts.map { it.url }) }
    val tabs by browser.tabs.collectAsStateWithLifecycle()
    var adding by rememberSaveable { mutableStateOf(false) }
    var refused by remember { mutableStateOf(false) }
    var sheet by rememberSaveable { mutableStateOf<String?>(null) }
    // Back from a new tab's home: the last tab again.
    BackHandler(enabled = tabs.isNotEmpty()) { browser.switchTo(tabs.last().id) }
    Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(bottom = bottomPadding.calculateBottomPadding())) {
        TabHeader(stringResource(R.string.browse)) {
            HeaderButton(
                stringResource(if (private) R.string.private_on else R.string.private_mode),
                { refused = !browser.setPrivate(!private) },
                LineIcon.Private,
                background = if (private) c.amberBg else c.surface,
                tint = if (private) c.amber else c.ink,
            )
            if (tabs.isNotEmpty()) TabsButton(tabs.size) { sheet = "tabs" }
        }
        AddressField(Modifier.padding(start = 18.dp, end = 18.dp, top = 18.dp), onGo = browser::open)
        if (refused) ZText(stringResource(R.string.private_unsupported), Modifier.padding(horizontal = 22.dp, vertical = 8.dp), size = 13.sp, color = c.red)
        if (private) Box(Modifier.padding(start = 18.dp, end = 18.dp, top = 14.dp)) { Note(stringResource(R.string.private_note), c.amberBg, c.amber, LineIcon.Private) }
        SectionLabel(stringResource(R.string.sites), Modifier.padding(top = 2.dp))
        // Five to a row, as the design; the last place adds a site.
        val tiles: List<Shortcut?> = shortcuts + null
        Column(Modifier.padding(horizontal = 12.dp), verticalArrangement = Arrangement.spacedBy(16.dp)) {
            tiles.chunked(5).forEach { row ->
                Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(4.dp)) {
                    row.forEach { shortcut ->
                        Box(Modifier.weight(1f)) {
                            if (shortcut == null) {
                                AddTile { adding = true }
                            } else {
                                val icon = SiteIcons.siteOf(shortcut.url)?.let { icons[it] }
                                SiteTile(shortcut, icon, TILES[shortcuts.indexOf(shortcut) % TILES.size], { browser.open(shortcut.url) }) {
                                    browser.store.removeShortcut(shortcut)
                                }
                            }
                        }
                    }
                    repeat(5 - row.size) { Spacer(Modifier.weight(1f)) }
                }
            }
        }
        if (!private) BookmarksCard(browser)
        if (recent.isNotEmpty() && !private) {
            ZCard(Modifier.padding(start = 18.dp, end = 18.dp, top = 22.dp), padding = PaddingValues(start = 16.dp, end = 16.dp, top = 12.dp, bottom = 2.dp)) {
                Row(verticalAlignment = Alignment.CenterVertically) {
                    ZText(stringResource(R.string.visited_lately), Modifier.weight(1f), size = 15.sp, weight = FontWeight.SemiBold)
                    ZText(
                        stringResource(R.string.history),
                        Modifier.clip(RoundedCornerShape(8.dp)).clickable(role = Role.Button) { sheet = "history" }.padding(horizontal = 6.dp, vertical = 8.dp),
                        size = 13.5.sp,
                        color = c.quietLink,
                    )
                }
                recent.forEachIndexed { i, visit ->
                    if (i > 0) RowLine()
                    SiteRow(
                        visit.url,
                        visit.title,
                        hostOf(visit.url),
                        SiteIcons.siteOf(visit.url)?.let { icons[it] },
                        LineIcon.Clock,
                        onClick = { browser.open(visit.url) },
                    )
                }
            }
        }
        Box(Modifier.padding(start = 18.dp, end = 18.dp, top = 22.dp)) { Note(stringResource(R.string.browse_promise), c.greenBg, c.green, LineIcon.Shield) }
    }
    if (adding) AddSiteSheet(onAdd = { browser.store.addShortcut(it) }, onDismiss = { adding = false })
    when (sheet) {
        "tabs" -> TabsSheet(browser, onDismiss = { sheet = null })
        "history" -> HistorySheet(browser, onDismiss = { sheet = null })
    }
}

/** «ابحث أو اكتب رابطًا»: a white pill on the background. */
@Composable
private fun AddressField(modifier: Modifier, onGo: (String) -> Unit) {
    var text by rememberSaveable { mutableStateOf("") }
    val keyboard = LocalSoftwareKeyboardController.current
    ZField(
        text,
        { text = it },
        stringResource(R.string.address_hint),
        modifier.card(28.dp),
        icon = LineIcon.Search,
        type = KeyboardType.Uri,
        imeAction = ImeAction.Go,
        onAction = {
            if (text.isNotBlank()) {
                keyboard?.hide()
                onGo(text)
                text = ""
            }
        },
        background = Z.colors.surface,
    )
}

@OptIn(ExperimentalFoundationApi::class)
@Composable
private fun SiteTile(shortcut: Shortcut, icon: ImageBitmap?, color: Color, onOpen: () -> Unit, onRemove: () -> Unit) {
    var asking by remember { mutableStateOf(false) }
    Column(
        Modifier
            .fillMaxWidth()
            .clip(RoundedCornerShape(16.dp))
            .combinedClickable(onClickLabel = shortcut.name, onClick = onOpen, onLongClick = { asking = true }),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.spacedBy(6.dp),
    ) {
        Box(Modifier.cardShadow(20.dp)) {
            if (icon == null && !isMotalim(shortcut.url)) {
                Box(Modifier.size(60.dp).clip(RoundedCornerShape(20.dp)).background(color), contentAlignment = Alignment.Center) {
                    ZText(shortcut.name.take(1).uppercase(), size = 24.sp, weight = FontWeight.Bold, color = Color.White)
                }
            } else {
                SiteBadge(shortcut.url, icon, 60.dp, 20.dp)
            }
        }
        ZText(shortcut.name, size = 12.5.sp, maxLines = 1, align = TextAlign.Center)
    }
    if (asking) {
        ChoiceSheet(
            shortcut.name,
            listOf(true to stringResource(R.string.remove_site), false to stringResource(R.string.cancel_sheet)),
            chosen = null,
            onChoose = { if (it == true) onRemove() },
            onDismiss = { asking = false },
        )
    }
}

@Composable
private fun AddTile(onClick: () -> Unit) {
    val c = Z.colors
    Column(
        Modifier.fillMaxWidth().clip(RoundedCornerShape(16.dp)).clickable(role = Role.Button, onClick = onClick),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.spacedBy(6.dp),
    ) {
        Box(
            Modifier.size(60.dp).clip(RoundedCornerShape(20.dp)).border(1.5.dp, Color(0xFFB9BFCA), RoundedCornerShape(20.dp)),
            contentAlignment = Alignment.Center,
        ) {
            ZIcon(LineIcon.Plus, c.muted, 24.dp)
        }
        ZText(stringResource(R.string.add_site), size = 12.5.sp, color = c.muted)
    }
}

@Composable
private fun AddSiteSheet(onAdd: (Shortcut) -> Unit, onDismiss: () -> Unit) {
    var name by rememberSaveable { mutableStateOf("") }
    var address by rememberSaveable { mutableStateOf("") }
    val url = Finds.addressOf(address)?.takeIf { address.isNotBlank() && !it.startsWith("https://duckduckgo.com/?q=") }
    ZSheet(onDismiss = onDismiss, label = stringResource(R.string.add_site_title)) {
        Column(verticalArrangement = Arrangement.spacedBy(16.dp)) {
            SheetTitle(stringResource(R.string.add_site_title))
            ZField(name, { name = it }, stringResource(R.string.site_name))
            ZField(address, { address = it }, stringResource(R.string.site_address), icon = LineIcon.Link, type = KeyboardType.Uri, direction = TextDirection.Ltr)
            Row(horizontalArrangement = Arrangement.spacedBy(10.dp)) {
                MainButton(
                    stringResource(R.string.add_site),
                    onClick = {
                        if (url != null) {
                            onAdd(Shortcut(name.trim().ifEmpty { hostOf(url) }.take(40), url))
                            onDismiss()
                        }
                    },
                    modifier = Modifier.weight(2f),
                    enabled = url != null,
                )
                QuietButton(stringResource(R.string.cancel_sheet), onClick = onDismiss, modifier = Modifier.weight(1f))
            }
        }
    }
}

/** A page: its address and tabs above, the page, «تحميل» when videos are found, and the browser's bar below. */
@Composable
private fun BrowsePage(browser: BrowserController, onFound: (Found) -> Unit, onSignIns: () -> Unit) {
    val c = Z.colors
    val page by browser.page.collectAsStateWithLifecycle()
    val found by browser.found.collectAsStateWithLifecycle()
    val private by browser.privateMode.collectAsStateWithLifecycle()
    val tabs by browser.tabs.collectAsStateWithLifecycle()
    val bookmarks by browser.store.bookmarks.collectAsStateWithLifecycle()
    val blocked by browser.blocked.collectAsStateWithLifecycle()
    var listing by rememberSaveable { mutableStateOf(false) }
    var showingTabs by rememberSaveable { mutableStateOf(false) }
    var protecting by rememberSaveable { mutableStateOf(false) }
    val warning by browser.warning.collectAsStateWithLifecycle()
    val state = page ?: return
    BackHandler { browser.back() }
    val nav = WindowInsets.navigationBars.asPaddingValues().calculateBottomPadding()
    Box(Modifier.fillMaxSize()) {
        Column(Modifier.fillMaxSize()) {
            Row(
                Modifier.windowInsetsPadding(WindowInsets.statusBars).padding(start = 14.dp, end = 14.dp, top = 18.dp, bottom = 14.dp),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                val marked = bookmarks.any { it.url == state.url }
                AddressBox(state.url, private, marked, Modifier.weight(1f), onBookmark = { browser.store.toggleBookmark(state.title, state.url) }) { protecting = true }
                TabsButton(tabs.size) { showingTabs = true }
            }
            if (state.loading) {
                ThinBar(state.progress / 100f, height = 2.dp, track = Color.Transparent)
            }
            Box(Modifier.weight(1f).fillMaxWidth().padding(bottom = 66.dp + 18.dp + nav)) {
                val view = browser.webView
                if (view != null) {
                    // Another tab is another view.
                    key(view) {
                        AndroidView(
                            factory = { view.also { (it.parent as? ViewGroup)?.removeView(it) } },
                            modifier = Modifier.fillMaxSize(),
                        )
                    }
                }
                warning?.let { w -> WarningScreen(w, onBack = browser::leaveWarning, onProceed = browser::proceedAnyway) }
            }
        }
        val downloadable = found.count { !(it is Found.Element && it.protected) }
        if (found.isNotEmpty() && warning == null) {
            FoundButton(downloadable.coerceAtLeast(0), Modifier.align(Alignment.BottomEnd).padding(end = 18.dp, bottom = 104.dp + nav)) { listing = true }
        }
        PageBar(
            state,
            blocked,
            Modifier.align(Alignment.BottomCenter).padding(start = 14.dp, end = 14.dp, bottom = 18.dp + nav),
            onBack = browser::back,
            onForward = browser::forward,
            onHome = browser::newTabHome,
            onReload = { browser.reload() },
            onProtection = { protecting = true },
        )
    }
    if (showingTabs) TabsSheet(browser, onDismiss = { showingTabs = false })
    if (protecting) ProtectionSheet(browser, hostOf(state.url), onDismiss = { protecting = false })
    if (listing) {
        FoundSheet(
            found,
            state,
            onPick = {
                listing = false
                onFound(it)
            },
            onSignIns = {
                listing = false
                onSignIns()
            },
            onDismiss = { listing = false },
        )
    }
}

/** The address: the lock (the site's protection), the address left to right, private mode, and the bookmark. */
@Composable
private fun AddressBox(url: String, private: Boolean, marked: Boolean, modifier: Modifier, onBookmark: () -> Unit, onProtection: () -> Unit) {
    val c = Z.colors
    val secure = url.startsWith("https://")
    Row(
        modifier
            .height(48.dp)
            .dropShadow(Color(0x4D121722), 12.dp, 4.dp, (-6).dp, 24.dp)
            .clip(CircleShape)
            .background(c.surface)
            .padding(start = 4.dp, end = 4.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        val lock = stringResource(R.string.protection_here)
        Box(
            Modifier.size(40.dp).clip(CircleShape).clickable(onClickLabel = lock, role = Role.Button, onClick = onProtection),
            contentAlignment = Alignment.Center,
        ) {
            ZIcon(
                if (secure) LineIcon.Lock else LineIcon.LockOpen,
                if (secure) c.green else c.red,
                18.dp,
                label = stringResource(if (secure) R.string.secure else R.string.not_secure),
            )
        }
        ZText(
            url.removePrefix("https://").removePrefix("http://").removePrefix("www."),
            Modifier.weight(1f),
            size = 14.sp,
            maxLines = 1,
            style = TextStyle(textDirection = TextDirection.Ltr),
        )
        if (private) ZIcon(LineIcon.Private, c.amber, 16.dp, label = stringResource(R.string.private_on))
        val label = stringResource(if (marked) R.string.remove_bookmark else R.string.add_bookmark)
        Box(
            Modifier.size(40.dp).clip(CircleShape).clickable(onClickLabel = label, role = Role.Button, onClick = onBookmark).semantics { contentDescription = label },
            contentAlignment = Alignment.Center,
        ) {
            ZIcon(if (marked) LineIcon.StarFilled else LineIcon.Star, if (marked) c.amber else c.chevron, 20.dp)
        }
    }
}

/** The browser's bar: back, forward, the home, reload, and the shield with what it blocked on the page. */
@Composable
private fun PageBar(
    state: PageState,
    blocked: Int,
    modifier: Modifier,
    onBack: () -> Unit,
    onForward: () -> Unit,
    onHome: () -> Unit,
    onReload: () -> Unit,
    onProtection: () -> Unit,
) {
    val c = Z.colors
    Row(
        modifier
            .fillMaxWidth()
            .height(66.dp)
            .dropShadow(Color(0x59121722), 22.dp, 10.dp, (-10).dp, 33.dp)
            .clip(CircleShape)
            .background(c.surface),
        horizontalArrangement = Arrangement.SpaceAround,
        verticalAlignment = Alignment.CenterVertically,
    ) {
        BarButton(LineIcon.ArrowBack, stringResource(R.string.back), c.softInk, onClick = onBack)
        BarButton(LineIcon.ArrowForward, stringResource(R.string.forward), if (state.canGoForward) c.softInk else Color(0xFFB9BFCA), enabled = state.canGoForward, onClick = onForward)
        BarButton(LineIcon.Home, stringResource(R.string.browser_home), c.softInk, onClick = onHome)
        BarButton(LineIcon.Reload, stringResource(R.string.reload), c.softInk, onClick = onReload)
        Box {
            BarButton(LineIcon.Shield, pluralStringResource(R.plurals.blocked_here, blocked, blocked), c.green, onClick = onProtection)
            if (blocked > 0) {
                Box(
                    Modifier
                        .align(Alignment.TopStart)
                        .offset(x = 2.dp)
                        .heightIn(min = 20.dp)
                        .widthIn(min = 20.dp)
                        .clip(CircleShape)
                        .background(c.green)
                        .padding(horizontal = 5.dp),
                    contentAlignment = Alignment.Center,
                ) {
                    ZText(if (blocked > 99) "99+" else "$blocked", size = 11.sp, weight = FontWeight.Bold, color = Color.White)
                }
            }
        }
    }
}

@Composable
private fun BarButton(icon: LineIcon, label: String, tint: Color, enabled: Boolean = true, onClick: () -> Unit) {
    Box(
        Modifier
            .size(48.dp)
            .clip(CircleShape)
            .clickable(enabled = enabled, onClickLabel = label, role = Role.Button, onClick = onClick)
            .semantics { contentDescription = label },
        contentAlignment = Alignment.Center,
    ) {
        ZIcon(icon, tint, 24.dp)
    }
}

/** «تحميل» with how many the page has: the floating pill over the page. */
@Composable
private fun FoundButton(count: Int, modifier: Modifier, onClick: () -> Unit) {
    val c = Z.colors
    Row(
        modifier
            .height(56.dp)
            .dropShadow(Color(0xBF0A5FE0), 24.dp, 12.dp, (-10).dp, 28.dp)
            .clip(CircleShape)
            .background(c.accent)
            .clickable(role = Role.Button, onClick = onClick)
            .padding(start = 18.dp, end = 10.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(10.dp),
    ) {
        ZIcon(LineIcon.Download, Color.White, 24.dp)
        ZText(stringResource(R.string.download), size = 15.5.sp, weight = FontWeight.SemiBold, color = Color.White)
        Box(Modifier.size(32.dp).clip(CircleShape).background(Color.White), contentAlignment = Alignment.Center) {
            ZText("$count", size = 14.sp, weight = FontWeight.Bold, color = c.accent)
        }
    }
}

/** The videos found on the page (D5), each with its download button; the sign-in note at the end. */
@Composable
private fun FoundSheet(found: List<Found>, page: PageState, onPick: (Found) -> Unit, onSignIns: () -> Unit, onDismiss: () -> Unit) {
    val c = Z.colors
    val title = pluralStringResource(R.plurals.found_on_page, found.size, found.size)
    ZSheet(onDismiss = onDismiss, label = title) {
        Column(verticalArrangement = Arrangement.spacedBy(16.dp)) {
            SheetTitle(title) { SheetClose(onDismiss) }
            Column(Modifier.heightIn(max = 460.dp).verticalScroll(rememberScrollState()), verticalArrangement = Arrangement.spacedBy(12.dp)) {
                found.forEach { item -> FoundRow(item, page) { onPick(item) } }
            }
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                ZIcon(LineIcon.Lock, c.muted, 18.dp, Modifier.offset(y = 2.dp))
                Column(verticalArrangement = Arrangement.spacedBy(2.dp)) {
                    ZText(stringResource(R.string.sign_in_note), size = 13.sp, color = c.muted)
                    ZText(
                        stringResource(R.string.manage_sign_ins),
                        Modifier.heightIn(min = 40.dp).clickable(role = Role.Button, onClick = onSignIns).padding(vertical = 8.dp),
                        size = 13.sp,
                        weight = FontWeight.SemiBold,
                        color = c.link,
                    )
                }
            }
        }
    }
}

@Composable
private fun FoundRow(item: Found, page: PageState, onPick: () -> Unit) {
    val c = Z.colors
    val (title, detail, duration) = describe(item, page)
    val usable = !(item is Found.Element && item.protected)
    val shape = RoundedCornerShape(22.dp)
    Row(
        Modifier.fillMaxWidth().clip(shape).border(1.dp, c.line, shape).padding(12.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        Box(
            Modifier
                .size(width = 92.dp, height = 54.dp)
                .clip(RoundedCornerShape(14.dp))
                .background(Brush.linearGradient(listOf(Color(0xFF1D2A40), Color(0xFF0B0F17)))),
            contentAlignment = Alignment.Center,
        ) {
            ZIcon(LineIcon.PlayFill, Color.White, 22.dp)
        }
        Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(3.dp)) {
            // A file's name mixes scripts: one unit, as in the lists.
            ZText(FileNames.display(title), size = 14.5.sp, weight = FontWeight.SemiBold, maxLines = 1, style = TextStyle(textDirection = TextDirection.Content))
            ZText(detail, size = 12.5.sp, color = if (usable) c.faint else c.amber, maxLines = 2)
            val tag = if (duration > 0) clock(duration) else tagOf(item)
            if (tag != null) Pill(tag, c.sunken, c.softInk, height = 24.dp, radius = 8.dp, size = 12.sp, weight = FontWeight.Normal)
        }
        if (usable) {
            val label = stringResource(R.string.download_named, title)
            Box(
                Modifier
                    .size(48.dp)
                    .clip(CircleShape)
                    .background(c.accent)
                    .clickable(role = Role.Button, onClickLabel = label, onClick = onPick)
                    .semantics { contentDescription = label },
                contentAlignment = Alignment.Center,
            ) {
                ZIcon(LineIcon.Download, Color.White, 22.dp)
            }
        }
    }
}

/** A stream's kind as its file names it, when its length is not known: «m3u8», «mpd». */
private fun tagOf(item: Found): String? = when (item) {
    is Found.Media -> when (item.kind) {
        MediaKind.Hls -> "m3u8"
        MediaKind.Dash -> "mpd"
        MediaKind.File -> extOf(item.url).lowercase().ifEmpty { null }
    }
    else -> null
}

/** A found video's title, its line under it, and its length (0: unknown). */
@Composable
private fun describe(item: Found, page: PageState): Triple<String, String, Int> {
    val pageTitle = page.title.ifBlank { hostOf(page.url) }
    return when (item) {
        is Found.SitePage -> Triple(
            item.title.ifBlank { pageTitle },
            stringResource(if (PlaylistLinks.isList(item.url)) R.string.found_playlist else R.string.found_site_page),
            0,
        )
        is Found.Media -> when (item.kind) {
            MediaKind.File -> Triple(nameOf(item.url), stringResource(R.string.found_file, extOf(item.url).uppercase(), hostOf(item.url)), item.duration)
            MediaKind.Hls -> Triple(pageTitle, stringResource(R.string.found_hls, hostOf(item.url)), item.duration)
            MediaKind.Dash -> Triple(pageTitle, stringResource(R.string.found_dash, hostOf(item.url)), item.duration)
        }
        is Found.Element -> if (item.protected) {
            Triple(pageTitle, stringResource(R.string.found_protected), item.duration)
        } else {
            Triple(nameOf(item.src.orEmpty()), stringResource(R.string.found_element, item.width, item.height), item.duration)
        }
    }
}

internal fun hostOf(url: String) = runCatching { URI(url).host }.getOrNull()?.removePrefix("www.").orEmpty()

private fun extOf(url: String) = runCatching { URI(url).rawPath }.getOrNull().orEmpty().substringAfterLast('/').substringAfterLast('.', "")

/** The address's last part, decoded, as a name to show (never its query). */
private fun nameOf(url: String): String {
    val last = runCatching { URI(url).rawPath }.getOrNull().orEmpty().substringAfterLast('/')
    return runCatching { URLDecoder.decode(last, "UTF-8") }.getOrDefault(last).ifBlank { hostOf(url) }
}

private fun clock(seconds: Int): String =
    if (seconds >= 3600) "%d:%02d:%02d".format(seconds / 3600, seconds % 3600 / 60, seconds % 60) else "%d:%02d".format(seconds / 60, seconds % 60)

/** «Manage sign-ins»: the sites that keep cookies in the browser, each to forget, or all. */
@Composable
fun SignInsSheet(browser: BrowserController, onDismiss: () -> Unit) {
    val c = Z.colors
    val sites by browser.store.sites.collectAsStateWithLifecycle()
    ZSheet(onDismiss = onDismiss, label = stringResource(R.string.manage_sign_ins)) {
        Column(verticalArrangement = Arrangement.spacedBy(16.dp)) {
            SheetTitle(stringResource(R.string.manage_sign_ins)) { SheetClose(onDismiss) }
            ZText(stringResource(R.string.sign_ins_hint), size = 13.sp, color = c.muted)
            if (sites.isEmpty()) {
                ZText(stringResource(R.string.no_sign_ins), color = c.muted)
            } else {
                Column(Modifier.fillMaxWidth().clip(RoundedCornerShape(22.dp)).border(1.dp, c.line, RoundedCornerShape(22.dp))) {
                    sites.forEachIndexed { i, site ->
                        if (i > 0) RowLine()
                        Row(
                            Modifier.fillMaxWidth().heightIn(min = 56.dp).padding(start = 16.dp, end = 4.dp),
                            verticalAlignment = Alignment.CenterVertically,
                        ) {
                            ZText(site, Modifier.weight(1f), size = 15.sp, style = TextStyle(textDirection = TextDirection.Ltr))
                            RowClose(stringResource(R.string.forget_site, site)) { browser.store.forget(site) }
                        }
                    }
                }
                QuietButton(stringResource(R.string.forget_all), onClick = { browser.store.forgetAll() }, modifier = Modifier.fillMaxWidth(), color = c.red)
            }
        }
    }
}
