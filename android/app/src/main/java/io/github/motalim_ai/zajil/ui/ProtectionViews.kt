package io.github.motalim_ai.zajil.ui

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.RowScope
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.navigationBars
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
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
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextDirection
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import io.github.motalim_ai.zajil.R
import io.github.motalim_ai.zajil.browser.BrowserController
import io.github.motalim_ai.zajil.browser.Guard
import io.github.motalim_ai.zajil.browser.Warning
import io.github.motalim_ai.zajil.protection
import java.text.NumberFormat
import java.time.LocalDate
import java.util.Locale
import kotlinx.coroutines.launch

/**
 * The browser's protection (0.3.0; D5 from 0.4.0): the panel from the
 * home's tile and Settings, the sheet behind the page's shield, and the
 * warning before a harmful page.
 */

@Composable
private fun guardTitle(guard: Guard) = stringResource(
    when (guard) {
        Guard.Ads -> R.string.guard_ads
        Guard.Malware -> R.string.guard_malware
    },
)

@Composable
private fun guardHint(guard: Guard) = stringResource(
    when (guard) {
        Guard.Ads -> R.string.guard_ads_hint
        Guard.Malware -> R.string.guard_malware_hint
    },
)

private fun guardIcon(guard: Guard) = when (guard) {
    Guard.Ads -> LineIcon.Ad
    Guard.Malware -> LineIcon.Malware
}

/** The lists' date, Western digits as everywhere in Zajil: «1 نوفمبر 2026». */
@Composable
private fun listsDate(seconds: Long): String {
    val locale = LocalConfiguration.current.locales[0]
    return java.time.Instant.ofEpochSecond(seconds).atZone(java.time.ZoneId.systemDefault())
        .format(java.time.format.DateTimeFormatter.ofPattern("d MMMM y", locale).withDecimalStyle(java.time.format.DecimalStyle.STANDARD))
}

/** A count with Western digits and the language's grouping: «1,284». */
private fun count(n: Long): String = NumberFormat.getIntegerInstance(Locale.US).format(n)

/**
 * «الحماية» (D5): what was blocked today, by list; the two switches, and
 * phishing through Safe Browsing, always on; the lists' update and the
 * sites with exceptions.
 */
@Composable
fun ProtectionScreen(onBack: () -> Unit) {
    val c = Z.colors
    val protection = LocalContext.current.protection
    val settings by protection.settings.collectAsStateWithLifecycle()
    val sequence by protection.sequence.collectAsStateWithLifecycle()
    val stored by protection.today.collectAsStateWithLifecycle()
    val today = stored.on(LocalDate.now().toEpochDay())
    val scope = rememberCoroutineScope()
    var updating by remember { mutableStateOf<Int?>(null) }
    var exceptions by remember { mutableStateOf(false) }
    BackHandler(onBack = onBack)
    Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState()).windowInsetsPadding(WindowInsets.navigationBars).padding(bottom = 24.dp)) {
        SubHeader(stringResource(R.string.group_protection), onBack)
        LayeredCard(Modifier.padding(start = 18.dp, end = 18.dp, top = 34.dp), gap = 14.dp) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                ZText(stringResource(R.string.blocked_today), Modifier.weight(1f), size = 14.sp, color = c.heroSub)
                val on = settings.on.isNotEmpty()
                Row(
                    Modifier.height(30.dp).clip(CircleShape).background(Color(0x38FFFFFF)).padding(horizontal = 12.dp),
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(6.dp),
                ) {
                    Box(Modifier.size(8.dp).clip(CircleShape).background(if (on) Color(0xFF5FDCA0) else Color(0x99FFFFFF)))
                    ZText(stringResource(if (on) R.string.guards_on else R.string.guards_off), size = 13.sp, color = Color.White)
                }
            }
            Row(verticalAlignment = Alignment.Bottom, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                ZText(count(today.total), size = 46.sp, weight = FontWeight.Bold, color = Color.White, maxLines = 1)
                val n = today.total.coerceAtMost(Int.MAX_VALUE.toLong()).toInt()
                ZText(pluralStringResource(R.plurals.requests, n), Modifier.padding(bottom = 8.dp), size = 16.sp, color = Color.White)
            }
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                MiniCount(today.ads.toLong(), stringResource(R.string.blocked_ads))
                MiniCount(today.trackers.toLong(), stringResource(R.string.blocked_trackers))
                MiniCount(today.malware.toLong(), stringResource(R.string.blocked_malware))
            }
        }
        SectionLabel(stringResource(R.string.what_is_blocked))
        ZCard(Modifier.padding(horizontal = 18.dp)) {
            Guard.entries.forEachIndexed { i, guard ->
                if (i > 0) RowLine()
                val (tint, color) = if (guard == Guard.Ads) c.sel to c.blue else c.redBg to c.red
                SettingRow(
                    guardTitle(guard),
                    hint = guardHint(guard),
                    icon = guardIcon(guard),
                    iconBackground = tint,
                    iconTint = color,
                    onClick = { protection.set(guard, guard !in settings.on) },
                ) {
                    ZSwitch(guard in settings.on, { protection.set(guard, it) })
                }
            }
            RowLine()
            // Phishing: WebView's Safe Browsing, always on; Zajil has no list of it (the owner's decision).
            SettingRow(
                stringResource(R.string.guard_phishing),
                hint = stringResource(R.string.guard_phishing_hint),
                icon = LineIcon.Phish,
                iconBackground = c.amberBg,
                iconTint = c.amber,
            ) {
                Pill(stringResource(R.string.always_on), c.greenBg, c.green, height = 28.dp)
            }
        }
        SectionLabel(stringResource(R.string.protection_lists))
        ZCard(Modifier.padding(horizontal = 18.dp)) {
            val update = {
                updating = R.string.protection_checking
                scope.launch {
                    updating = protection.update().fold(
                        onSuccess = { if (it) R.string.protection_updated else R.string.protection_latest },
                        onFailure = { R.string.protection_update_failed },
                    )
                }
            }
            SettingRow(
                stringResource(R.string.last_update),
                hint = updating?.let { stringResource(it) }
                    ?: if (sequence > 0) stringResource(R.string.lists_signed, listsDate(sequence)) else null,
                icon = LineIcon.Reload,
            ) {
                SoftButton(stringResource(R.string.update_now), if (updating == R.string.protection_checking) null else ({ update() }))
            }
            RowLine()
            SettingRow(
                stringResource(R.string.protection_exceptions),
                hint = stringResource(R.string.protection_exceptions_hint),
                value = "${settings.off.size}",
                icon = LineIcon.Globe,
                onClick = { exceptions = true },
            )
        }
        ZText(stringResource(R.string.protection_note), Modifier.padding(horizontal = 22.dp, vertical = 16.dp), size = 12.5.sp, color = c.muted)
    }
    if (exceptions) {
        ZSheet(onDismiss = { exceptions = false }, label = stringResource(R.string.protection_exceptions)) {
            Column(verticalArrangement = Arrangement.spacedBy(16.dp)) {
                SheetTitle(stringResource(R.string.protection_exceptions)) { SheetClose { exceptions = false } }
                if (settings.off.isEmpty()) ZText(stringResource(R.string.no_exceptions), size = 13.5.sp, color = c.muted)
                Column(Modifier.fillMaxWidth().clip(RoundedCornerShape(22.dp)).background(c.sunken).padding(horizontal = 14.dp)) {
                    settings.off.entries.sortedBy { it.key }.forEachIndexed { i, (site, off) ->
                        if (i > 0) RowLine(c.line)
                        SettingRow(site, hint = off.map { guardTitle(it) }.joinToString(stringResource(R.string.list_sep))) {
                            HeaderButton(stringResource(R.string.remove_exception), { off.forEach { protection.setFor(site, it, true) } }, LineIcon.Close, tint = c.red)
                        }
                    }
                }
            }
        }
    }
}

@Composable
private fun RowScope.MiniCount(n: Long, label: String) {
    Column(Modifier.weight(1f).clip(RoundedCornerShape(16.dp)).background(Color(0x29FFFFFF)).padding(horizontal = 12.dp, vertical = 10.dp)) {
        ZText(count(n), size = 18.sp, weight = FontWeight.Bold, color = Color.White, maxLines = 1)
        ZText(label, size = 12.5.sp, color = Z.colors.heroSub, maxLines = 1)
    }
}

/** «Protection on this site»: the site, what was blocked on the page, and the switches for it. */
@Composable
fun ProtectionSheet(browser: BrowserController, host: String, onDismiss: () -> Unit) {
    val c = Z.colors
    val protection = LocalContext.current.protection
    val settings by protection.settings.collectAsStateWithLifecycle()
    val blocked by browser.blocked.collectAsStateWithLifecycle()
    val site = browser.site
    ZSheet(onDismiss = onDismiss, label = stringResource(R.string.protection_here)) {
        Column(verticalArrangement = Arrangement.spacedBy(16.dp)) {
            Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                IconCircle(LineIcon.Shield, c.greenBg, c.green, size = 44.dp, iconSize = 24.dp)
                Column(Modifier.weight(1f)) {
                    ZText(stringResource(R.string.protection_here), size = 21.sp, weight = FontWeight.Bold)
                    ZText(host, size = 13.sp, color = c.muted, maxLines = 1, style = TextStyle(textDirection = TextDirection.Ltr))
                }
                SheetClose(onDismiss)
            }
            Note(pluralStringResource(R.plurals.blocked_here, blocked, blocked), c.greenBg, c.green, LineIcon.Shield)
            Column(Modifier.fillMaxWidth().clip(RoundedCornerShape(22.dp)).background(c.sunken).padding(horizontal = 14.dp)) {
                Guard.entries.forEachIndexed { i, guard ->
                    if (i > 0) RowLine(c.line)
                    val everywhere = guard in settings.on
                    val here = settings.active(guard, site)
                    SettingRow(
                        guardTitle(guard),
                        // Off everywhere: the site's switch cannot turn it on.
                        hint = if (everywhere) guardHint(guard) else stringResource(R.string.guard_off_everywhere),
                        icon = guardIcon(guard),
                        iconBackground = c.surface,
                        onClick = if (everywhere && site != null) ({ protection.setFor(site, guard, !here) }) else null,
                    ) {
                        ZSwitch(here, { on -> if (site != null) protection.setFor(site, guard, on) }, enabled = everywhere && site != null)
                    }
                }
            }
            QuietNote(stringResource(R.string.protection_note), LineIcon.Info)
        }
    }
}

/** Before a harmful page: what it is, «Back», and a small «Continue on my own responsibility». */
@Composable
fun WarningScreen(warning: Warning, onBack: () -> Unit, onProceed: () -> Unit) {
    val c = Z.colors
    ZBackground {
        Column(
            Modifier.align(Alignment.Center).widthIn(max = 480.dp).verticalScroll(rememberScrollState()).padding(24.dp),
            horizontalAlignment = Alignment.CenterHorizontally,
            verticalArrangement = Arrangement.spacedBy(14.dp),
        ) {
            IconCircle(LineIcon.ShieldAlert, c.redBg, c.red, size = 76.dp, iconSize = 38.dp)
            ZText(stringResource(R.string.warning_malware_title), size = 21.sp, weight = FontWeight.Bold, align = TextAlign.Center)
            ZText(
                warning.host,
                Modifier.clip(RoundedCornerShape(10.dp)).background(c.sunken).padding(horizontal = 10.dp, vertical = 4.dp),
                size = 14.sp,
                style = TextStyle(textDirection = TextDirection.Ltr),
            )
            ZText(stringResource(R.string.warning_malware_body), size = 15.sp, color = c.muted, align = TextAlign.Center)
            MainButton(stringResource(R.string.warning_back), onClick = onBack, modifier = Modifier.fillMaxWidth())
            ZText(
                stringResource(R.string.warning_proceed),
                Modifier.clip(RoundedCornerShape(8.dp)).clickable(role = Role.Button, onClick = onProceed).padding(12.dp),
                size = 13.sp,
                color = c.muted,
            )
        }
    }
}
