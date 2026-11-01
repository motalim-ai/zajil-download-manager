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
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Icon
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.FilterQuality
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextDirection
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import io.github.motalim_ai.zajil.R
import io.github.motalim_ai.zajil.browser.BrowserController
import io.github.motalim_ai.zajil.browser.SiteIcons

/**
 * The browser's tabs, bookmarks and history (A6b, 0.3.0). Not in the
 * approved design: made of its parts (the bar's buttons, cards, sheets and
 * rows of «Visited lately»).
 */

/** The tabs: a rounded square with how many are open, as browsers show it. */
@Composable
fun TabsButton(count: Int, onClick: () -> Unit) {
    val c = Z.colors
    val label = stringResource(R.string.tabs_open, count)
    Box(
        Modifier.size(48.dp).clip(RoundedCornerShape(12.dp)).clickable(onClickLabel = label, role = Role.Button, onClick = onClick)
            .semantics { contentDescription = label },
        contentAlignment = Alignment.Center,
    ) {
        Box(
            Modifier.size(24.dp).clip(RoundedCornerShape(6.dp)).border(2.dp, c.muted, RoundedCornerShape(6.dp)),
            contentAlignment = Alignment.Center,
        ) {
            ZText(if (count > 99) "∞" else "$count", size = 11.5.sp, weight = FontWeight.Bold, color = c.muted)
        }
    }
}

/** The open tabs: each with its icon, title and address, «×» to close it; a new tab. */
@Composable
fun TabsSheet(browser: BrowserController, onDismiss: () -> Unit) {
    val c = Z.colors
    val tabs by browser.tabs.collectAsStateWithLifecycle()
    val page by browser.page.collectAsStateWithLifecycle()
    val icons by browser.store.icons.icons.collectAsStateWithLifecycle()
    val current = page?.url
    ZSheet(onDismiss = onDismiss, label = stringResource(R.string.tabs)) {
        Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
            ZText(stringResource(R.string.tabs), size = 19.sp, weight = FontWeight.Bold)
            if (tabs.isEmpty()) ZText(stringResource(R.string.no_tabs), size = 13.5.sp, color = c.muted)
            ZCard {
                tabs.forEachIndexed { i, tab ->
                    if (i > 0) RowLine()
                    Row(
                        Modifier
                            .fillMaxWidth()
                            .heightIn(min = 60.dp)
                            .background(if (tab.url == current && current != null) c.picked else c.surface)
                            .clickable {
                                browser.switchTo(tab.id)
                                onDismiss()
                            }
                            .padding(start = 14.dp, end = 4.dp),
                        verticalAlignment = Alignment.CenterVertically,
                        horizontalArrangement = Arrangement.spacedBy(12.dp),
                    ) {
                        SiteIcon(SiteIcons.siteOf(tab.url)?.let { icons[it] }, LineIcon.Globe)
                        Column(Modifier.weight(1f)) {
                            ZText(tab.title.ifBlank { hostOf(tab.url) }, maxLines = 1, style = TextStyle(textDirection = TextDirection.Content))
                            ZText(hostOf(tab.url), size = 12.sp, color = c.muted, maxLines = 1, style = TextStyle(textDirection = TextDirection.Ltr))
                        }
                        BarIconButton(LineIcon.Close, stringResource(R.string.close_tab)) { browser.closeTab(tab.id) }
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
                    icon = lineIcon(LineIcon.Plus),
                )
                if (tabs.size > 1) {
                    QuietButton(stringResource(R.string.close_all_tabs), onClick = {
                        browser.close()
                        onDismiss()
                    }, modifier = Modifier.weight(1f))
                }
            }
        }
    }
}

/** The history: every page opened, newest first, with when; «×» forgets one, and «Clear history». */
@Composable
fun HistorySheet(browser: BrowserController, onDismiss: () -> Unit) {
    val c = Z.colors
    val history by browser.store.history.collectAsStateWithLifecycle()
    val icons by browser.store.icons.icons.collectAsStateWithLifecycle()
    ZSheet(onDismiss = onDismiss, label = stringResource(R.string.history)) {
        Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
            ZText(stringResource(R.string.history), size = 19.sp, weight = FontWeight.Bold)
            ZText(stringResource(R.string.history_hint), size = 12.5.sp, color = c.muted)
            if (history.isEmpty()) {
                ZText(stringResource(R.string.no_history), size = 13.5.sp, color = c.muted)
            } else {
                LazyColumn(
                    Modifier.fillMaxWidth().heightIn(max = 420.dp).clip(RoundedCornerShape(16.dp)).border(1.dp, c.line, RoundedCornerShape(16.dp)),
                ) {
                    itemsIndexed(history, key = { _, e -> "${e.at}|${e.url}" }) { i, entry ->
                        if (i > 0) RowLine()
                        Row(
                            Modifier
                                .fillMaxWidth()
                                .heightIn(min = 56.dp)
                                .clickable {
                                    browser.open(entry.url)
                                    onDismiss()
                                }
                                .padding(start = 14.dp, end = 4.dp),
                            verticalAlignment = Alignment.CenterVertically,
                            horizontalArrangement = Arrangement.spacedBy(12.dp),
                        ) {
                            SiteIcon(SiteIcons.siteOf(entry.url)?.let { icons[it] }, LineIcon.Clock)
                            Column(Modifier.weight(1f)) {
                                ZText(entry.title, maxLines = 1, style = TextStyle(textDirection = TextDirection.Content))
                                ZText(
                                    // The address as one left-to-right unit, so that the time beside it stays whole.
                                    DateUtils.getRelativeTimeSpanString(entry.at, System.currentTimeMillis(), DateUtils.MINUTE_IN_MILLIS).toString() +
                                        " · \u2066" + hostOf(entry.url) + "\u2069",
                                    size = 12.sp,
                                    color = c.muted,
                                    maxLines = 1,
                                )
                            }
                            BarIconButton(LineIcon.Close, stringResource(R.string.forget_page)) { browser.store.forgetVisit(entry) }
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
@OptIn(ExperimentalFoundationApi::class)
@Composable
fun BookmarksCard(browser: BrowserController) {
    val c = Z.colors
    val bookmarks by browser.store.bookmarks.collectAsStateWithLifecycle()
    val icons by browser.store.icons.icons.collectAsStateWithLifecycle()
    var removing by remember { mutableStateOf<String?>(null) }
    if (bookmarks.isEmpty()) return
    ZText(stringResource(R.string.bookmarks), size = 14.sp, weight = FontWeight.Bold, color = c.muted)
    ZCard {
        bookmarks.forEachIndexed { i, mark ->
            if (i > 0) RowLine()
            Row(
                Modifier
                    .fillMaxWidth()
                    .heightIn(min = 56.dp)
                    .combinedClickable(onClick = { browser.open(mark.url) }, onLongClick = { removing = mark.url })
                    .padding(horizontal = 14.dp),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(12.dp),
            ) {
                SiteIcon(SiteIcons.siteOf(mark.url)?.let { icons[it] }, LineIcon.Star)
                Column(Modifier.weight(1f)) {
                    ZText(mark.title, maxLines = 1, style = TextStyle(textDirection = TextDirection.Content))
                    ZText(hostOf(mark.url), size = 12.sp, color = c.muted, maxLines = 1, style = TextStyle(textDirection = TextDirection.Ltr))
                }
            }
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

/** A site's own icon, or a line icon in its place. */
@Composable
private fun SiteIcon(icon: ImageBitmap?, otherwise: LineIcon) {
    if (icon != null) {
        Image(icon, null, Modifier.size(20.dp).clip(RoundedCornerShape(5.dp)), filterQuality = FilterQuality.Medium)
    } else {
        Icon(lineIcon(otherwise), null, tint = Z.colors.muted, modifier = Modifier.size(20.dp))
    }
}
