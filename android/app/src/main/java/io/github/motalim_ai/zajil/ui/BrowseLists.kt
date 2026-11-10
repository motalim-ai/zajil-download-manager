package io.github.motalim_ai.zajil.ui

import android.text.format.DateUtils
import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.FilterQuality
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextDirection
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import io.github.motalim_ai.zajil.R
import io.github.motalim_ai.zajil.browser.BrowserController
import io.github.motalim_ai.zajil.browser.BrowserTabs
import io.github.motalim_ai.zajil.browser.SiteIcons
import java.net.URI

/**
 * The browser's tabs, bookmarks and history (A6b, 0.3.0), in the D5
 * design's parts: its sheets, white cards and rows.
 */

/** The tabs: a rounded square with how many are open, as browsers show it, on a header button. */
@Composable
fun TabsButton(count: Int, onClick: () -> Unit) {
    val c = Z.colors
    HeaderButton(stringResource(R.string.tabs_open, count), onClick) {
        Box(
            Modifier.size(22.dp).clip(RoundedCornerShape(7.dp)).border(2.dp, c.ink, RoundedCornerShape(7.dp)),
            contentAlignment = Alignment.Center,
        ) {
            ZText(if (count > 99) "∞" else "$count", size = 12.sp, weight = FontWeight.Bold, color = c.ink)
        }
    }
}

@Composable
fun ClosePrivateDialog(onConfirm: () -> Unit, onDismiss: () -> Unit) {
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text(stringResource(R.string.close_private_tabs_title)) },
        text = { Text(stringResource(R.string.close_private_tabs_message)) },
        confirmButton = {
            TextButton(onClick = onConfirm) { Text(stringResource(R.string.close_private_tabs)) }
        },
        dismissButton = {
            TextButton(onClick = onDismiss) { Text(stringResource(R.string.cancel_sheet)) }
        },
    )
}

/** Whether [url] is the Motalim channel, which shows its own mark (the D5 hand-over). */
fun isMotalim(url: String): Boolean {
    val uri = runCatching { URI(url) }.getOrNull() ?: return false
    val host = uri.host?.lowercase() ?: return false
    return (host == "youtube.com" || host.endsWith(".youtube.com")) && uri.path.orEmpty().startsWith("/@motalim-ai")
}

/**
 * A site's picture: the Motalim channel's own mark, the site's own icon
 * on white, or [otherwise] on soft grey.
 */
@Composable
fun SiteBadge(url: String, icon: ImageBitmap?, size: Dp, radius: Dp, otherwise: LineIcon = LineIcon.Globe) {
    val c = Z.colors
    val shape = RoundedCornerShape(radius)
    when {
        isMotalim(url) -> Box(Modifier.size(size).clip(shape).background(Color(0xFF0354D6)), contentAlignment = Alignment.Center) {
            Icon(painterResource(R.drawable.ic_motalim), null, tint = Color.Unspecified, modifier = Modifier.size(size * 0.66f))
        }
        icon != null -> Box(Modifier.size(size).clip(shape).background(Color.White).border(1.dp, c.line, shape), contentAlignment = Alignment.Center) {
            Image(icon, null, Modifier.size(size * 0.6f).clip(RoundedCornerShape(size / 8)), filterQuality = FilterQuality.Medium)
        }
        else -> Box(Modifier.size(size).clip(shape).background(c.sunken), contentAlignment = Alignment.Center) {
            ZIcon(otherwise, c.muted, size * 0.5f)
        }
    }
}

/** A row of a site list: its picture, its title and address, and what goes at the end. */
@OptIn(ExperimentalFoundationApi::class)
@Composable
fun SiteRow(
    url: String,
    title: String,
    sub: String,
    icon: ImageBitmap?,
    otherwise: LineIcon,
    onClick: () -> Unit,
    onLongClick: (() -> Unit)? = null,
    end: (@Composable () -> Unit)? = null,
) {
    Row(
        Modifier
            .fillMaxWidth()
            .heightIn(min = 58.dp)
            .combinedClickable(onClick = onClick, onLongClick = onLongClick)
            .padding(vertical = 11.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        SiteBadge(url, icon, 36.dp, 12.dp, otherwise)
        Column(Modifier.weight(1f)) {
            ZText(title, size = 15.sp, weight = FontWeight.Medium, maxLines = 1, style = TextStyle(textDirection = TextDirection.Content))
            ZText(sub, size = 12.5.sp, color = Z.colors.faint, maxLines = 1)
        }
        end?.invoke()
    }
}

/** A small «×» at a row's end. */
@Composable
fun RowClose(label: String, onClick: () -> Unit) {
    Box(
        Modifier.size(44.dp).clip(CircleShape).clickable(onClickLabel = label, role = Role.Button, onClick = onClick).semantics { contentDescription = label },
        contentAlignment = Alignment.Center,
    ) {
        ZIcon(LineIcon.Close, Z.colors.muted, 20.dp)
    }
}

/** The open tabs: each with its icon, title and address, «×» to close it; a new tab. */
@Composable
fun TabsSheet(browser: BrowserController, onDismiss: () -> Unit) {
    val c = Z.colors
    val tabs by browser.tabs.collectAsStateWithLifecycle()
    val privateMode by browser.privateMode.collectAsStateWithLifecycle()
    val page by browser.page.collectAsStateWithLifecycle()
    val icons by browser.store.icons.icons.collectAsStateWithLifecycle()
    val currentTabs = BrowserTabs.inGroup(tabs, privateMode)
    var closePrivate by remember { mutableStateOf(false) }
    var refused by remember { mutableStateOf(false) }
    ZSheet(onDismiss = onDismiss, label = stringResource(R.string.tabs)) {
        Column(verticalArrangement = Arrangement.spacedBy(16.dp)) {
            SheetTitle(stringResource(R.string.tabs)) { SheetClose(onDismiss) }
            Row(horizontalArrangement = Arrangement.spacedBy(10.dp)) {
                if (privateMode) {
                    QuietButton(stringResource(R.string.regular_tabs), onClick = { browser.setPrivate(false) }, modifier = Modifier.weight(1f))
                    MainButton(stringResource(R.string.private_tabs), onClick = {}, modifier = Modifier.weight(1f))
                } else {
                    MainButton(stringResource(R.string.regular_tabs), onClick = {}, modifier = Modifier.weight(1f))
                    QuietButton(
                        stringResource(R.string.private_tabs),
                        onClick = { refused = !browser.setPrivate(true) },
                        modifier = Modifier.weight(1f),
                    )
                }
            }
            if (refused) ZText(stringResource(R.string.private_unsupported), size = 13.sp, color = c.red)
            if (currentTabs.isEmpty()) ZText(stringResource(R.string.no_tabs), size = 13.5.sp, color = c.muted)
            if (currentTabs.isNotEmpty()) {
                Column(Modifier.fillMaxWidth().clip(RoundedCornerShape(22.dp)).border(1.dp, c.line, RoundedCornerShape(22.dp))) {
                    currentTabs.forEachIndexed { i, tab ->
                        if (i > 0) RowLine()
                        Box(Modifier.background(if (page?.url == tab.url) c.picked else Color.Transparent).padding(start = 14.dp, end = 4.dp)) {
                            SiteRow(
                                tab.url,
                                tab.title.ifBlank { hostOf(tab.url) },
                                hostOf(tab.url),
                                SiteIcons.siteOf(tab.url)?.let { icons[it] },
                                LineIcon.Globe,
                                onClick = {
                                    browser.switchTo(tab.id)
                                    onDismiss()
                                },
                            ) {
                                RowClose(stringResource(R.string.close_tab)) {
                                    if (!browser.closeTab(tab.id)) closePrivate = true
                                }
                            }
                        }
                    }
                }
            }
            Row(horizontalArrangement = Arrangement.spacedBy(10.dp)) {
                MainButton(
                    stringResource(R.string.new_tab),
                    onClick = {
                        browser.newTabHome()
                        onDismiss()
                    },
                    modifier = Modifier.weight(2f),
                    icon = LineIcon.Plus,
                )
                if (privateMode || currentTabs.size > 1) {
                    QuietButton(
                        stringResource(if (privateMode) R.string.close_private_tabs else R.string.close_all_tabs),
                        onClick = {
                            if (privateMode) closePrivate = true else {
                                browser.closeRegularTabs()
                                onDismiss()
                            }
                        },
                        modifier = Modifier.weight(1f),
                    )
                }
            }
        }
    }
    if (closePrivate) {
        ClosePrivateDialog(
            onConfirm = {
                browser.closePrivateTabs()
                closePrivate = false
                onDismiss()
            },
            onDismiss = { closePrivate = false },
        )
    }
}

/** The history: every page opened, newest first, with when; «×» forgets one, and «Clear history». */
@Composable
fun HistorySheet(browser: BrowserController, onDismiss: () -> Unit) {
    val c = Z.colors
    val history by browser.store.history.collectAsStateWithLifecycle()
    val icons by browser.store.icons.icons.collectAsStateWithLifecycle()
    ZSheet(onDismiss = onDismiss, label = stringResource(R.string.history)) {
        Column(verticalArrangement = Arrangement.spacedBy(16.dp)) {
            SheetTitle(stringResource(R.string.history)) { SheetClose(onDismiss) }
            ZText(stringResource(R.string.history_hint), size = 13.sp, color = c.muted)
            if (history.isEmpty()) {
                ZText(stringResource(R.string.no_history), size = 13.5.sp, color = c.muted)
            } else {
                LazyColumn(
                    Modifier.fillMaxWidth().heightIn(max = 420.dp).clip(RoundedCornerShape(22.dp)).border(1.dp, c.line, RoundedCornerShape(22.dp)),
                ) {
                    itemsIndexed(history, key = { _, e -> "${e.at}|${e.url}" }) { i, entry ->
                        if (i > 0) RowLine()
                        Box(Modifier.padding(start = 14.dp, end = 4.dp)) {
                            SiteRow(
                                entry.url,
                                entry.title,
                                // The address as one left-to-right unit, so that the time beside it stays whole.
                                DateUtils.getRelativeTimeSpanString(entry.at, System.currentTimeMillis(), DateUtils.MINUTE_IN_MILLIS).toString() +
                                    stringResource(R.string.list_sep) + "\u2066" + hostOf(entry.url) + "\u2069",
                                SiteIcons.siteOf(entry.url)?.let { icons[it] },
                                LineIcon.Clock,
                                onClick = {
                                    browser.open(entry.url)
                                    onDismiss()
                                },
                            ) { RowClose(stringResource(R.string.forget_page)) { browser.store.forgetVisit(entry) } }
                        }
                    }
                }
                QuietButton(stringResource(R.string.clear_history), onClick = {
                    browser.store.clearRecent()
                    onDismiss()
                }, modifier = Modifier.fillMaxWidth(), color = c.red)
            }
        }
    }
}

/** The bookmarks on the browser's home: a tap opens one, a long press asks to remove it. */
@Composable
fun BookmarksCard(browser: BrowserController) {
    val bookmarks by browser.store.bookmarks.collectAsStateWithLifecycle()
    val icons by browser.store.icons.icons.collectAsStateWithLifecycle()
    var removing by remember { mutableStateOf<String?>(null) }
    if (bookmarks.isEmpty()) return
    SectionLabel(stringResource(R.string.bookmarks))
    ZCard(Modifier.padding(horizontal = 18.dp)) {
        bookmarks.forEachIndexed { i, mark ->
            if (i > 0) RowLine()
            SiteRow(
                mark.url,
                mark.title,
                hostOf(mark.url),
                SiteIcons.siteOf(mark.url)?.let { icons[it] },
                LineIcon.Star,
                onClick = { browser.open(mark.url) },
                onLongClick = { removing = mark.url },
            )
        }
    }
    removing?.let { url ->
        val mark = bookmarks.firstOrNull { it.url == url }
        if (mark != null) {
            ChoiceSheet(
                mark.title,
                listOf(true to stringResource(R.string.remove_bookmark), false to stringResource(R.string.cancel_sheet)),
                chosen = null,
                onChoose = { if (it == true) browser.store.toggleBookmark(mark.title, mark.url) },
                onDismiss = { removing = null },
            )
        }
    }
}
