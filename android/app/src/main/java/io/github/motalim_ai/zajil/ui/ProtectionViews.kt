package io.github.motalim_ai.zajil.ui

import android.text.format.DateUtils
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Icon
import androidx.compose.material3.Switch
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.pluralStringResource
import androidx.compose.ui.res.stringResource
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
import kotlinx.coroutines.launch

/**
 * The browser's protection (0.3.0): the sheet behind the lock, the warning
 * before a harmful page, and the settings' group. Not in the approved
 * design: made of its parts (sheets, cards, switches, the green and amber
 * notes), in Zajil's own look.
 */

@Composable
private fun guardTitle(guard: Guard) = stringResource(
    when (guard) {
        Guard.Ads -> R.string.guard_ads
        Guard.Malware -> R.string.guard_malware
        Guard.Phishing -> R.string.guard_phishing
    },
)

@Composable
private fun guardHint(guard: Guard) = stringResource(
    when (guard) {
        Guard.Ads -> R.string.guard_ads_hint
        Guard.Malware -> R.string.guard_malware_hint
        Guard.Phishing -> R.string.guard_phishing_hint
    },
)

/** «Protection on this site»: the site, what was blocked on the page, and the three switches for it. */
@Composable
fun ProtectionSheet(browser: BrowserController, host: String, onDismiss: () -> Unit) {
    val c = Z.colors
    val protection = LocalContext.current.protection
    val settings by protection.settings.collectAsStateWithLifecycle()
    val blocked by browser.blocked.collectAsStateWithLifecycle()
    val site = browser.site
    ZSheet(onDismiss = onDismiss, label = stringResource(R.string.protection_here)) {
        Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
            Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(10.dp)) {
                Icon(lineIcon(LineIcon.Shield), null, tint = c.green, modifier = Modifier.size(26.dp))
                Column(Modifier.weight(1f)) {
                    ZText(stringResource(R.string.protection_here), size = 19.sp, weight = FontWeight.Bold)
                    ZText(host, size = 13.sp, color = c.muted, maxLines = 1, style = TextStyle(textDirection = TextDirection.Ltr))
                }
            }
            Note(pluralStringResource(R.plurals.blocked_here, blocked, blocked), c.greenBg, c.green, LineIcon.Shield)
            ZCard {
                Guard.entries.forEachIndexed { i, guard ->
                    if (i > 0) RowLine()
                    val everywhere = guard in settings.on
                    val here = settings.active(guard, site)
                    SettingRow(
                        guardTitle(guard),
                        // Off everywhere: the site's switch cannot turn it on.
                        hint = if (everywhere) guardHint(guard) else stringResource(R.string.guard_off_everywhere),
                        onClick = if (everywhere && site != null) ({ protection.setFor(site, guard, !here) }) else null,
                    ) {
                        Switch(
                            checked = here,
                            enabled = everywhere && site != null,
                            onCheckedChange = { on -> if (site != null) protection.setFor(site, guard, on) },
                            colors = switchColors(),
                        )
                    }
                }
            }
            ZText(stringResource(R.string.protection_note), size = 12.sp, color = c.muted)
            QuietButton(stringResource(R.string.close), onClick = onDismiss, modifier = Modifier.fillMaxWidth())
        }
    }
}

/** Before a harmful page: what it is, «Back», and a small «Continue on my own responsibility». */
@Composable
fun WarningScreen(warning: Warning, onBack: () -> Unit, onProceed: () -> Unit) {
    val c = Z.colors
    Box(Modifier.fillMaxSize().background(c.bg), contentAlignment = Alignment.Center) {
        Column(
            Modifier.widthIn(max = 480.dp).verticalScroll(rememberScrollState()).padding(24.dp),
            horizontalAlignment = Alignment.CenterHorizontally,
            verticalArrangement = Arrangement.spacedBy(14.dp),
        ) {
            Box(Modifier.size(72.dp).clip(CircleShape).background(c.redBg), contentAlignment = Alignment.Center) {
                Icon(lineIcon(LineIcon.ShieldAlert), null, tint = c.red, modifier = Modifier.size(38.dp))
            }
            ZText(
                stringResource(if (warning.guard == Guard.Phishing) R.string.warning_phishing_title else R.string.warning_malware_title),
                size = 21.sp,
                weight = FontWeight.Bold,
                align = TextAlign.Center,
            )
            ZText(
                warning.host,
                Modifier.clip(RoundedCornerShape(8.dp)).background(c.sunken).padding(horizontal = 10.dp, vertical = 4.dp),
                size = 14.sp,
                style = TextStyle(textDirection = TextDirection.Ltr),
            )
            ZText(
                stringResource(if (warning.guard == Guard.Phishing) R.string.warning_phishing_body else R.string.warning_malware_body),
                size = 15.sp,
                color = c.muted,
                align = TextAlign.Center,
            )
            MainButton(stringResource(R.string.warning_back), onClick = onBack, modifier = Modifier.fillMaxWidth())
            ZText(
                stringResource(R.string.warning_proceed),
                Modifier.clip(RoundedCornerShape(8.dp)).clickable(role = androidx.compose.ui.semantics.Role.Button, onClick = onProceed).padding(8.dp),
                size = 13.sp,
                color = c.muted,
            )
        }
    }
}

/** The settings' «Protection» group: the three switches, the sites with exceptions, and the lists. */
@Composable
fun ProtectionGroup() {
    val c = Z.colors
    val protection = LocalContext.current.protection
    val settings by protection.settings.collectAsStateWithLifecycle()
    val sequence by protection.sequence.collectAsStateWithLifecycle()
    val scope = rememberCoroutineScope()
    var updating by remember { mutableStateOf<Int?>(null) }
    var exceptions by remember { mutableStateOf(false) }
    Guard.entries.forEachIndexed { i, guard ->
        if (i > 0) RowLine()
        SettingRow(guardTitle(guard), hint = guardHint(guard), onClick = { protection.set(guard, guard !in settings.on) }) {
            Switch(checked = guard in settings.on, onCheckedChange = { protection.set(guard, it) }, colors = switchColors())
        }
    }
    RowLine()
    SettingRow(
        stringResource(R.string.protection_exceptions),
        hint = stringResource(R.string.protection_exceptions_hint),
        value = pluralStringResource(R.plurals.sites, settings.off.size, settings.off.size),
        onClick = { exceptions = true },
    )
    RowLine()
    SettingRow(
        stringResource(R.string.protection_lists),
        hint = updating?.let { stringResource(it) }
            ?: if (sequence > 0) stringResource(R.string.protection_lists_date, DateUtils.formatDateTime(null, sequence * 1000, DateUtils.FORMAT_SHOW_DATE)) else null,
        value = stringResource(R.string.check_now),
        onClick = {
            updating = R.string.protection_checking
            scope.launch {
                updating = protection.update().fold(
                    onSuccess = { if (it) R.string.protection_updated else R.string.protection_latest },
                    onFailure = { R.string.protection_update_failed },
                )
            }
        },
    )
    if (exceptions) {
        ZSheet(onDismiss = { exceptions = false }, label = stringResource(R.string.protection_exceptions)) {
            Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
                ZText(stringResource(R.string.protection_exceptions), size = 19.sp, weight = FontWeight.Bold)
                if (settings.off.isEmpty()) ZText(stringResource(R.string.no_exceptions), size = 13.5.sp, color = c.muted)
                ZCard {
                    settings.off.entries.sortedBy { it.key }.forEachIndexed { i, (site, off) ->
                        if (i > 0) RowLine()
                        SettingRow(
                            site,
                            hint = off.map { guardTitle(it) }.joinToString(" · "),
                        ) {
                            BarIconButton(LineIcon.Close, stringResource(R.string.remove_exception)) { off.forEach { protection.setFor(site, it, true) } }
                        }
                    }
                }
                QuietButton(stringResource(R.string.close), onClick = { exceptions = false }, modifier = Modifier.fillMaxWidth())
            }
        }
    }
}
