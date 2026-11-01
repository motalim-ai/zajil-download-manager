package io.github.motalim_ai.zajil.ui

import android.view.ViewGroup
import androidx.activity.compose.BackHandler
import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.Image
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
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBars
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
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
import androidx.compose.ui.draw.shadow
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.FilterQuality
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.platform.LocalLayoutDirection
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
import androidx.compose.ui.unit.LayoutDirection
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.compose.ui.viewinterop.AndroidView
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import io.github.motalim_ai.zajil.R
import io.github.motalim_ai.zajil.browser.BrowserController
import io.github.motalim_ai.zajil.browser.Finds
import io.github.motalim_ai.zajil.browser.Found
import io.github.motalim_ai.zajil.browser.MediaKind
import io.github.motalim_ai.zajil.browser.Shortcut
import io.github.motalim_ai.zajil.browser.SiteIcons
import java.net.URI
import java.net.URLDecoder

/** The tiles' colours, from the design (letters, never the sites' own logos). */
private val TILES = listOf(0xFFD42F2C, 0xFF1D63D8, 0xFFB0399B, 0xFF2A3A44, 0xFF10263D, 0xFFE08A06, 0xFF1A9AD6).map(::Color)

/** «Browse» (A6): the browser's home, or the page with its found videos. */
@Composable
fun BrowseScreen(browser: BrowserController, onFound: (Found) -> Unit, onSignIns: () -> Unit, bottomPadding: PaddingValues) {
    val page by browser.page.collectAsStateWithLifecycle()
    if (page == null) BrowseHome(browser, bottomPadding) else BrowsePage(browser, onFound, onSignIns, bottomPadding)
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
    var adding by rememberSaveable { mutableStateOf(false) }
    var refused by remember { mutableStateOf(false) }
    Column(Modifier.fillMaxSize().background(c.bg)) {
        Column(
            Modifier.background(c.surface).windowInsetsPadding(WindowInsets.statusBars).padding(16.dp),
            verticalArrangement = Arrangement.spacedBy(14.dp),
        ) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                ZText(stringResource(R.string.browse), Modifier.weight(1f), size = 21.sp, weight = FontWeight.Bold)
                PrivatePill(private) { refused = !browser.setPrivate(!private) }
            }
            AddressField(onGo = browser::open)
        }
        RowLine()
        Column(
            Modifier
                .verticalScroll(rememberScrollState())
                .padding(16.dp)
                .padding(bottom = bottomPadding.calculateBottomPadding()),
            verticalArrangement = Arrangement.spacedBy(18.dp),
        ) {
            if (refused) ZText(stringResource(R.string.private_unsupported), size = 13.sp, color = c.red)
            if (private) Note(stringResource(R.string.private_note), c.amberBg, c.amber, LineIcon.Private)
            ZText(stringResource(R.string.sites), size = 14.sp, weight = FontWeight.Bold, color = c.muted)
            (shortcuts.map<Shortcut, Shortcut?> { it } + null).chunked(4).forEach { row ->
                Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    row.forEachIndexed { i, shortcut ->
                        Box(Modifier.weight(1f)) {
                            if (shortcut == null) {
                                AddTile { adding = true }
                            } else {
                                val icon = SiteIcons.siteOf(shortcut.url)?.let { icons[it] }
                                SiteTile(shortcut, icon, TILES[(shortcuts.indexOf(shortcut)) % TILES.size], { browser.open(shortcut.url) }) {
                                    browser.store.removeShortcut(shortcut)
                                }
                            }
                        }
                        if (i == row.lastIndex) repeat(4 - row.size) { Spacer(Modifier.weight(1f)) }
                    }
                }
            }
            if (recent.isNotEmpty() && !private) {
                ZText(stringResource(R.string.visited_lately), size = 14.sp, weight = FontWeight.Bold, color = c.muted)
                ZCard {
                    recent.forEachIndexed { i, visit ->
                        if (i > 0) RowLine()
                        Row(
                            Modifier.fillMaxWidth().heightIn(min = 56.dp).clickable { browser.open(visit.url) }.padding(horizontal = 14.dp),
                            verticalAlignment = Alignment.CenterVertically,
                            horizontalArrangement = Arrangement.spacedBy(12.dp),
                        ) {
                            val icon = SiteIcons.siteOf(visit.url)?.let { icons[it] }
                            if (icon != null) {
                                Image(icon, null, Modifier.size(20.dp).clip(RoundedCornerShape(5.dp)), filterQuality = FilterQuality.Medium)
                            } else {
                                Icon(lineIcon(LineIcon.Clock), null, tint = c.muted, modifier = Modifier.size(20.dp))
                            }
                            Column(Modifier.weight(1f)) {
                                ZText(visit.title, maxLines = 1, style = TextStyle(textDirection = TextDirection.Content))
                                ZText(hostOf(visit.url), size = 12.sp, color = c.muted, maxLines = 1, style = TextStyle(textDirection = TextDirection.Ltr))
                            }
                        }
                    }
                }
            }
            Note(stringResource(R.string.browse_promise), c.greenBg, c.green, LineIcon.Shield)
        }
    }
    if (adding) AddSiteSheet(onAdd = { browser.store.addShortcut(it) }, onDismiss = { adding = false })
}

@Composable
private fun PrivatePill(on: Boolean, onClick: () -> Unit) {
    val c = Z.colors
    val shape = CircleShape
    Row(
        Modifier
            .heightIn(min = 48.dp)
            .clip(shape)
            .clickable(role = Role.Switch, onClick = onClick),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Row(
            Modifier
                .height(36.dp)
                .clip(shape)
                .background(if (on) c.amberBg else c.surface)
                .border(1.dp, if (on) c.amber else c.outline, shape)
                .padding(horizontal = 12.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(6.dp),
        ) {
            Icon(lineIcon(LineIcon.Private), null, tint = if (on) c.amber else c.muted, modifier = Modifier.size(16.dp))
            ZText(
                stringResource(if (on) R.string.private_on else R.string.private_mode),
                size = 13.sp,
                weight = FontWeight.SemiBold,
                color = if (on) c.amber else c.muted,
            )
        }
    }
}

@Composable
private fun AddressField(onGo: (String) -> Unit) {
    val c = Z.colors
    var text by rememberSaveable { mutableStateOf("") }
    val keyboard = LocalSoftwareKeyboardController.current
    val shape = RoundedCornerShape(16.dp)
    val go = {
        if (text.isNotBlank()) {
            keyboard?.hide()
            onGo(text)
            text = ""
        }
    }
    Row(
        Modifier
            .fillMaxWidth()
            .height(52.dp)
            .clip(shape)
            .background(c.sunken)
            .border(1.5.dp, c.outline, shape)
            .padding(horizontal = 14.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(10.dp),
    ) {
        Icon(lineIcon(LineIcon.Search), null, tint = c.muted, modifier = Modifier.size(20.dp))
        Box(Modifier.weight(1f), contentAlignment = Alignment.CenterStart) {
            if (text.isEmpty()) ZText(stringResource(R.string.address_hint), color = c.muted, size = 15.sp)
            val label = stringResource(R.string.address_hint)
            BasicTextField(
                value = text,
                onValueChange = { text = it },
                singleLine = true,
                textStyle = TextStyle(fontFamily = Plex, fontSize = 15.sp, color = c.ink, textDirection = TextDirection.Content),
                keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Uri, imeAction = ImeAction.Go),
                keyboardActions = KeyboardActions(onGo = { go() }),
                cursorBrush = SolidColor(c.accent),
                modifier = Modifier.fillMaxWidth().semantics { contentDescription = label },
            )
        }
    }
}

@OptIn(ExperimentalFoundationApi::class)
@Composable
private fun SiteTile(shortcut: Shortcut, icon: ImageBitmap?, color: Color, onOpen: () -> Unit, onRemove: () -> Unit) {
    var asking by remember { mutableStateOf(false) }
    Column(
        Modifier
            .fillMaxWidth()
            .clip(RoundedCornerShape(12.dp))
            .combinedClickable(onClickLabel = shortcut.name, onClick = onOpen, onLongClick = { asking = true }),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.spacedBy(6.dp),
    ) {
        if (icon != null) {
            // The site's own icon, on the card's color, as browsers show it.
            val shape = RoundedCornerShape(18.dp)
            Box(
                Modifier.size(56.dp).clip(shape).background(Z.colors.surface).border(1.dp, Z.colors.line, shape),
                contentAlignment = Alignment.Center,
            ) {
                Image(icon, null, Modifier.size(34.dp).clip(RoundedCornerShape(8.dp)), filterQuality = FilterQuality.Medium)
            }
        } else {
            Box(Modifier.size(56.dp).clip(RoundedCornerShape(18.dp)).background(color), contentAlignment = Alignment.Center) {
                ZText(shortcut.name.take(1).uppercase(), size = 22.sp, weight = FontWeight.Bold, color = Color.White)
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
        Modifier.fillMaxWidth().clip(RoundedCornerShape(12.dp)).clickable(role = Role.Button, onClick = onClick),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.spacedBy(6.dp),
    ) {
        Box(
            Modifier
                .size(56.dp)
                .clip(RoundedCornerShape(18.dp))
                .background(c.surface)
                .border(1.5.dp, c.outline, RoundedCornerShape(18.dp)),
            contentAlignment = Alignment.Center,
        ) {
            Icon(lineIcon(LineIcon.Plus), null, tint = c.accent, modifier = Modifier.size(22.dp))
        }
        ZText(stringResource(R.string.add_site), size = 12.5.sp, color = c.accent)
    }
}

@Composable
private fun AddSiteSheet(onAdd: (Shortcut) -> Unit, onDismiss: () -> Unit) {
    var name by rememberSaveable { mutableStateOf("") }
    var address by rememberSaveable { mutableStateOf("") }
    val url = Finds.addressOf(address)?.takeIf { address.isNotBlank() && !it.startsWith("https://duckduckgo.com/?q=") }
    ZSheet(onDismiss = onDismiss, label = stringResource(R.string.add_site_title)) {
        Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
            ZText(stringResource(R.string.add_site_title), size = 19.sp, weight = FontWeight.Bold)
            SheetField(name, { name = it }, stringResource(R.string.site_name), KeyboardType.Text, TextDirection.Content)
            SheetField(address, { address = it }, stringResource(R.string.site_address), KeyboardType.Uri, TextDirection.Ltr)
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

@Composable
internal fun SheetField(value: String, onValue: (String) -> Unit, hint: String, type: KeyboardType, direction: TextDirection) {
    val c = Z.colors
    val shape = RoundedCornerShape(14.dp)
    Box(
        Modifier.fillMaxWidth().heightIn(min = 50.dp).clip(shape).background(c.sunken).border(1.5.dp, c.outline, shape).padding(horizontal = 14.dp),
        contentAlignment = Alignment.CenterStart,
    ) {
        if (value.isEmpty()) ZText(hint, color = c.muted, size = 15.sp)
        BasicTextField(
            value = value,
            onValueChange = onValue,
            singleLine = true,
            textStyle = TextStyle(fontFamily = Plex, fontSize = 15.sp, color = c.ink, textDirection = direction),
            keyboardOptions = KeyboardOptions(keyboardType = type),
            cursorBrush = SolidColor(c.accent),
            modifier = Modifier.fillMaxWidth().semantics { contentDescription = hint },
        )
    }
}

@Composable
private fun BrowsePage(browser: BrowserController, onFound: (Found) -> Unit, onSignIns: () -> Unit, bottomPadding: PaddingValues) {
    val c = Z.colors
    val page by browser.page.collectAsStateWithLifecycle()
    val found by browser.found.collectAsStateWithLifecycle()
    val private by browser.privateMode.collectAsStateWithLifecycle()
    var listing by rememberSaveable { mutableStateOf(false) }
    val state = page ?: return
    BackHandler { browser.back() }
    Column(Modifier.fillMaxSize().background(c.surface).padding(bottom = bottomPadding.calculateBottomPadding())) {
        Column(Modifier.background(c.surface).windowInsetsPadding(WindowInsets.statusBars)) {
            Row(
                Modifier.padding(start = 4.dp, end = 4.dp, top = 8.dp, bottom = 8.dp),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(4.dp),
            ) {
                // «Back» points to the start of the line: right in Arabic, left in English.
                val mirror = LocalLayoutDirection.current == LayoutDirection.Ltr
                Box(Modifier.graphicsLayer { scaleX = if (mirror) -1f else 1f }) {
                    BarIconButton(LineIcon.Back, stringResource(R.string.back)) { browser.back() }
                }
                AddressBox(state.url, private, Modifier.weight(1f))
                BarIconButton(LineIcon.Reload, stringResource(R.string.reload)) { browser.reload() }
            }
            if (state.loading) {
                LinearProgressIndicator(
                    progress = { state.progress / 100f },
                    modifier = Modifier.fillMaxWidth().height(2.dp),
                    color = c.accent,
                    trackColor = c.line,
                    gapSize = 0.dp,
                    drawStopIndicator = {},
                )
            } else {
                RowLine()
            }
        }
        Box(Modifier.weight(1f).fillMaxWidth()) {
            val view = browser.webView
            if (view != null) {
                AndroidView(
                    factory = { view.also { (it.parent as? ViewGroup)?.removeView(it) } },
                    modifier = Modifier.fillMaxSize(),
                )
            }
            val downloadable = found.count { !(it is Found.Element && it.protected) }
            if (found.isNotEmpty()) {
                FoundButton(downloadable.coerceAtLeast(0), Modifier.align(Alignment.BottomStart).padding(16.dp)) { listing = true }
            }
        }
    }
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

@Composable
private fun AddressBox(url: String, private: Boolean, modifier: Modifier) {
    val c = Z.colors
    val secure = url.startsWith("https://")
    val shape = RoundedCornerShape(14.dp)
    Row(
        modifier.height(42.dp).clip(shape).background(c.sunken).border(1.dp, c.line, shape).padding(horizontal = 12.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        Icon(
            lineIcon(if (secure) LineIcon.Lock else LineIcon.LockOpen),
            contentDescription = stringResource(if (secure) R.string.secure else R.string.not_secure),
            tint = if (secure) c.green else c.red,
            modifier = Modifier.size(15.dp),
        )
        ZText(
            url.removePrefix("https://").removePrefix("http://").removePrefix("www."),
            Modifier.weight(1f),
            size = 14.sp,
            maxLines = 1,
            style = TextStyle(textDirection = TextDirection.Ltr),
        )
        if (private) Icon(lineIcon(LineIcon.Private), stringResource(R.string.private_on), tint = c.amber, modifier = Modifier.size(16.dp))
    }
}

@Composable
private fun FoundButton(count: Int, modifier: Modifier, onClick: () -> Unit) {
    val c = Z.colors
    val shape = RoundedCornerShape(18.dp)
    Row(
        modifier
            .shadow(10.dp, shape)
            .clip(shape)
            .background(c.accent)
            .clickable(role = Role.Button, onClick = onClick)
            .height(56.dp)
            .padding(start = 14.dp, end = 18.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(10.dp),
    ) {
        Icon(lineIcon(LineIcon.Downloads), null, tint = c.accentInk, modifier = Modifier.size(22.dp))
        ZText(stringResource(R.string.download), size = 15.5.sp, weight = FontWeight.SemiBold, color = c.accentInk)
        Box(
            Modifier.widthIn(min = 24.dp).height(24.dp).clip(RoundedCornerShape(12.dp)).background(c.accentInk).padding(horizontal = 6.dp),
            contentAlignment = Alignment.Center,
        ) {
            ZText("$count", size = 13.sp, weight = FontWeight.Bold, color = c.accent)
        }
    }
}

/** The videos found on the page, each to the sheet that downloads it. */
@Composable
private fun FoundSheet(found: List<Found>, page: io.github.motalim_ai.zajil.browser.PageState, onPick: (Found) -> Unit, onSignIns: () -> Unit, onDismiss: () -> Unit) {
    val c = Z.colors
    val title = pluralStringResource(R.plurals.found_on_page, found.size, found.size)
    ZSheet(onDismiss = onDismiss, label = title) {
        Column(verticalArrangement = Arrangement.spacedBy(14.dp)) {
            Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                ZText(title, Modifier.weight(1f), size = 19.sp, weight = FontWeight.Bold)
                ZText(hostOf(page.url), size = 12.5.sp, color = c.muted, maxLines = 1, style = TextStyle(textDirection = TextDirection.Ltr))
            }
            found.forEach { item -> FoundRow(item, page) { onPick(item) } }
            Row(
                Modifier.fillMaxWidth().clip(RoundedCornerShape(16.dp)).background(c.amberBg).padding(horizontal = 14.dp, vertical = 12.dp),
                horizontalArrangement = Arrangement.spacedBy(12.dp),
            ) {
                Icon(lineIcon(LineIcon.Lock), null, tint = c.amber, modifier = Modifier.size(22.dp))
                Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
                    ZText(stringResource(R.string.sign_in_note), size = 13.sp, color = c.ink)
                    ZText(
                        stringResource(R.string.manage_sign_ins),
                        Modifier.heightIn(min = 32.dp).clickable(role = Role.Button, onClick = onSignIns),
                        size = 13.sp,
                        weight = FontWeight.Bold,
                        color = c.amber,
                    )
                }
            }
            QuietButton(stringResource(R.string.close), onClick = onDismiss, modifier = Modifier.fillMaxWidth(), height = 50.dp)
        }
    }
}

@Composable
private fun FoundRow(item: Found, page: io.github.motalim_ai.zajil.browser.PageState, onPick: () -> Unit) {
    val c = Z.colors
    val (title, detail, duration) = describe(item, page)
    val usable = !(item is Found.Element && item.protected)
    val shape = RoundedCornerShape(16.dp)
    Row(
        Modifier.fillMaxWidth().clip(shape).background(c.sunken).border(1.dp, c.line, shape).padding(12.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        Box(
            Modifier.size(width = 96.dp, height = 54.dp).clip(RoundedCornerShape(10.dp)).background(Color(0xFF111A24)),
            contentAlignment = Alignment.Center,
        ) {
            Icon(lineIcon(LineIcon.Play), null, tint = Color.Unspecified, modifier = Modifier.size(22.dp))
            if (duration > 0) {
                ZText(
                    clock(duration),
                    Modifier.align(Alignment.BottomStart).padding(4.dp).clip(RoundedCornerShape(4.dp)).background(Color(0x99000000)).padding(horizontal = 4.dp),
                    size = 10.5.sp,
                    color = Color.White,
                )
            }
        }
        Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(4.dp)) {
            ZText(title, size = 14.5.sp, weight = FontWeight.SemiBold, maxLines = 1, style = TextStyle(textDirection = TextDirection.Content))
            ZText(detail, size = 12.sp, color = if (usable) c.muted else c.amber, maxLines = 2)
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
                Icon(lineIcon(LineIcon.Downloads), null, tint = c.accentInk, modifier = Modifier.size(22.dp))
            }
        }
    }
}

/** A found video's title, its line under it, and its length (0: unknown). */
@Composable
private fun describe(item: Found, page: io.github.motalim_ai.zajil.browser.PageState): Triple<String, String, Int> {
    val pageTitle = page.title.ifBlank { hostOf(page.url) }
    return when (item) {
        is Found.SitePage -> Triple(item.title.ifBlank { pageTitle }, stringResource(R.string.found_site_page), 0)
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

private fun hostOf(url: String) = runCatching { URI(url).host }.getOrNull()?.removePrefix("www.").orEmpty()

private fun extOf(url: String) = runCatching { URI(url).rawPath }.getOrNull().orEmpty().substringAfterLast('/').substringAfterLast('.', "")

/** The address's last part, decoded, as a name to show (never its query). */
private fun nameOf(url: String): String {
    val last = runCatching { URI(url).rawPath }.getOrNull().orEmpty().substringAfterLast('/')
    return runCatching { URLDecoder.decode(last, "UTF-8") }.getOrDefault(last).ifBlank { hostOf(url) }
}

private fun clock(seconds: Int): String =
    if (seconds >= 3600) "%d:%02d:%02d".format(seconds / 3600, seconds % 3600 / 60, seconds % 60) else "%d:%02d".format(seconds / 60, seconds % 60)

/** A short note on a coloured ground, as the design's. */
@Composable
private fun Note(text: String, background: Color, color: Color, icon: LineIcon) {
    Row(
        Modifier.fillMaxWidth().clip(RoundedCornerShape(16.dp)).background(background).padding(horizontal = 14.dp, vertical = 12.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(10.dp),
    ) {
        Icon(lineIcon(icon), null, tint = color, modifier = Modifier.size(22.dp))
        ZText(text, size = 13.sp, color = color)
    }
}

/** «Manage sign-ins»: the sites that keep cookies in the browser, each to forget, or all. */
@Composable
fun SignInsSheet(browser: BrowserController, onDismiss: () -> Unit) {
    val c = Z.colors
    val sites by browser.store.sites.collectAsStateWithLifecycle()
    ZSheet(onDismiss = onDismiss, label = stringResource(R.string.manage_sign_ins)) {
        Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
            ZText(stringResource(R.string.manage_sign_ins), size = 19.sp, weight = FontWeight.Bold)
            ZText(stringResource(R.string.sign_ins_hint), size = 13.sp, color = c.muted)
            if (sites.isEmpty()) {
                ZText(stringResource(R.string.no_sign_ins), color = c.muted)
            } else {
                ZCard {
                    sites.forEachIndexed { i, site ->
                        if (i > 0) RowLine()
                        Row(
                            Modifier.fillMaxWidth().heightIn(min = 56.dp).padding(start = 14.dp, end = 6.dp),
                            verticalAlignment = Alignment.CenterVertically,
                        ) {
                            ZText(site, Modifier.weight(1f), style = TextStyle(textDirection = TextDirection.Ltr))
                            val label = stringResource(R.string.forget_site, site)
                            Box(
                                Modifier.size(48.dp).clip(CircleShape).clickable(role = Role.Button, onClickLabel = label) { browser.store.forget(site) }
                                    .semantics { contentDescription = label },
                                contentAlignment = Alignment.Center,
                            ) {
                                Icon(lineIcon(LineIcon.Close), null, tint = c.red, modifier = Modifier.size(20.dp))
                            }
                        }
                    }
                }
                QuietButton(stringResource(R.string.forget_all), onClick = { browser.store.forgetAll() }, modifier = Modifier.fillMaxWidth(), color = c.red)
            }
            QuietButton(stringResource(R.string.close), onClick = onDismiss, modifier = Modifier.fillMaxWidth(), height = 50.dp)
        }
    }
}

