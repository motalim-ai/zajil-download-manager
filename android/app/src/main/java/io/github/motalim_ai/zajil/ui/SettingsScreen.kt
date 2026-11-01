package io.github.motalim_ai.zajil.ui

import android.app.LocaleManager
import android.content.Context
import android.content.Intent
import android.net.Uri
import android.os.Build
import android.os.LocaleList
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.pluralStringResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import io.github.motalim_ai.zajil.R
import io.github.motalim_ai.zajil.browser.BrowserController
import io.github.motalim_ai.zajil.downloads
import io.github.motalim_ai.zajil.downloads.Look
import io.github.motalim_ai.zajil.downloads.Prefs
import io.github.motalim_ai.zajil.downloads.Publisher
import io.github.motalim_ai.zajil.protection
import io.github.motalim_ai.zajil.video.ToolsUpdate
import io.github.motalim_ai.zajil.video.YtDlp
import kotlinx.coroutines.launch

private enum class Asking { Language, Look, Connections, SpeedLimit, SavePlace, Quality }

/** Settings (D5): grouped white cards, each row with its icon, and the green note at the end. */
@Composable
fun SettingsScreen(bottomPadding: PaddingValues, browser: BrowserController, onSignIns: () -> Unit, onProtection: () -> Unit) {
    val context = LocalContext.current
    val center = context.downloads
    val prefs by center.prefs.prefs.collectAsStateWithLifecycle()
    val queues by center.settings.queues.collectAsStateWithLifecycle()
    val protection by context.protection.settings.collectAsStateWithLifecycle()
    val c = Z.colors
    var asking by rememberSaveable { mutableStateOf<Asking?>(null) }
    val units = sizeUnits()
    // Android's own folder picker: the folder's permission is kept, the old one's given back.
    val pickFolder = rememberLauncherForActivityResult(ActivityResultContracts.OpenDocumentTree()) { tree ->
        if (tree == null) return@rememberLauncherForActivityResult
        val flags = Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_GRANT_WRITE_URI_PERMISSION
        if (runCatching { context.contentResolver.takePersistableUriPermission(tree, flags) }.isFailure) return@rememberLauncherForActivityResult
        center.prefs.prefs.value.saveTree?.takeIf { it != tree.toString() }?.let { old -> releaseFolder(context, old) }
        center.prefs.update { it.copy(saveTree = tree.toString()) }
    }

    Column(
        Modifier
            .fillMaxSize()
            .verticalScroll(rememberScrollState())
            .padding(bottom = bottomPadding.calculateBottomPadding()),
    ) {
        TabHeader(stringResource(R.string.settings))
        Group(stringResource(R.string.group_download)) {
            SettingRow(
                stringResource(R.string.save_place),
                value = prefs.saveTree?.let(Publisher::shownTree) ?: "Download/Zajil",
                icon = LineIcon.Folder,
                iconBackground = c.sel,
                iconTint = c.blue,
                onClick = { asking = Asking.SavePlace },
            )
            RowLine()
            SettingRow(
                stringResource(R.string.folder_per_type),
                hint = stringResource(R.string.folder_per_type_hint),
                icon = LineIcon.Archive,
                onClick = { center.prefs.update { it.copy(folderPerType = !it.folderPerType) } },
            ) {
                ZSwitch(prefs.folderPerType, { on -> center.prefs.update { it.copy(folderPerType = on) } })
            }
            RowLine()
            SettingRow(
                stringResource(R.string.connections_per_file),
                value = "${prefs.connections}",
                icon = LineIcon.Plug,
                iconBackground = c.greenBg,
                iconTint = c.green,
                onClick = { asking = Asking.Connections },
            )
            RowLine()
            SettingRow(
                stringResource(R.string.speed_limit),
                value = prefs.speedLimit?.let { stringResource(R.string.per_second, Units.size(it, units)) }
                    ?: stringResource(R.string.no_limit),
                icon = LineIcon.Bolt,
                iconBackground = c.amberBg,
                iconTint = c.amber,
                onClick = { asking = Asking.SpeedLimit },
            )
            RowLine()
            // The main queue's rule, as on its card in «Queues».
            val main = queues.first()
            SettingRow(
                stringResource(R.string.wifi_only),
                icon = LineIcon.Wifi,
                iconBackground = c.purpleBg,
                iconTint = c.purple,
                onClick = { center.settings.update(main.id) { it.copy(wifiOnly = !it.wifiOnly) } },
            ) {
                ZSwitch(main.rules.wifiOnly, { on -> center.settings.update(main.id) { it.copy(wifiOnly = on) } })
            }
        }
        Group(stringResource(R.string.group_video)) {
            SettingRow(
                stringResource(R.string.preferred_quality),
                value = "${prefs.videoHeight}p",
                icon = LineIcon.Video,
                iconBackground = c.redBg,
                iconTint = c.red,
                onClick = { asking = Asking.Quality },
            )
            RowLine()
            VideoTools()
        }
        Group(stringResource(R.string.group_look)) {
            // Android keeps a language for each app from Android 13.
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
                SettingRow(
                    stringResource(R.string.language),
                    value = stringResource(languageName(appLanguage(context))),
                    icon = LineIcon.Languages,
                    onClick = { asking = Asking.Language },
                )
                RowLine()
            }
            SettingRow(stringResource(R.string.look), value = stringResource(lookName(prefs.look)), icon = LineIcon.Moon, onClick = { asking = Asking.Look })
        }
        Group(stringResource(R.string.group_browser)) {
            val sites by browser.store.sites.collectAsStateWithLifecycle()
            val recent by browser.store.recent.collectAsStateWithLifecycle()
            val history by browser.store.history.collectAsStateWithLifecycle()
            SettingRow(
                stringResource(R.string.group_protection),
                hint = stringResource(R.string.protection_short),
                value = stringResource(if (protection.on.isEmpty()) R.string.guards_off else R.string.guards_on),
                icon = LineIcon.Shield,
                iconBackground = c.greenBg,
                iconTint = c.green,
                onClick = onProtection,
            )
            RowLine()
            SettingRow(
                stringResource(R.string.manage_sign_ins),
                hint = stringResource(R.string.sign_ins_short),
                value = pluralStringResource(R.plurals.sites, sites.size, sites.size),
                icon = LineIcon.Lock,
                onClick = onSignIns,
            )
            RowLine()
            val nothing = recent.isEmpty() && history.isEmpty()
            SettingRow(
                stringResource(R.string.clear_visited),
                value = stringResource(if (nothing) R.string.nothing_to_clear else R.string.clear),
                valueColor = if (nothing) c.muted else c.red,
                icon = LineIcon.History,
                onClick = if (nothing) null else ({ browser.store.clearRecent() }),
                chevron = false,
            )
        }
        Group(stringResource(R.string.group_about)) {
            AppUpdateRows()
        }
        Column(Modifier.padding(start = 18.dp, end = 18.dp, top = 16.dp)) { PromiseNote() }
    }

    when (asking) {
        Asking.Language -> LanguageSheet(onDismiss = { asking = null })
        Asking.Look -> ChoiceSheet(
            stringResource(R.string.look),
            Look.entries.map { it to stringResource(lookName(it)) },
            prefs.look,
            onChoose = { look -> center.prefs.update { it.copy(look = look) } },
            onDismiss = { asking = null },
        )
        Asking.Connections -> ChoiceSheet(
            stringResource(R.string.connections_per_file),
            Prefs.CONNECTIONS.map { it to pluralStringResource(R.plurals.connections, it, "$it") },
            prefs.connections,
            onChoose = { n -> center.prefs.update { it.copy(connections = n) } },
            onDismiss = { asking = null },
        )
        Asking.SavePlace -> ChoiceSheet(
            stringResource(R.string.save_place),
            listOf(false to stringResource(R.string.save_default), true to stringResource(R.string.save_choose)),
            chosen = prefs.saveTree != null,
            onChoose = { choose ->
                if (choose) {
                    pickFolder.launch(null)
                } else {
                    prefs.saveTree?.let { old -> releaseFolder(context, old) }
                    center.prefs.update { it.copy(saveTree = null) }
                }
            },
            onDismiss = { asking = null },
        )
        Asking.SpeedLimit -> ChoiceSheet(
            stringResource(R.string.speed_limit),
            Prefs.SPEED_LIMITS.map {
                it to (it?.let { rate -> stringResource(R.string.per_second, Units.size(rate, units)) } ?: stringResource(R.string.no_limit))
            },
            prefs.speedLimit,
            onChoose = { rate -> center.prefs.update { it.copy(speedLimit = rate) } },
            onDismiss = { asking = null },
        )
        Asking.Quality -> ChoiceSheet(
            stringResource(R.string.preferred_quality),
            Prefs.VIDEO_HEIGHTS.map { it to "${it}p" },
            prefs.videoHeight,
            onChoose = { h -> center.prefs.update { it.copy(videoHeight = h) } },
            onDismiss = { asking = null },
        )
        null -> {}
    }
}

/** The app's language: the system's, Arabic or English (Android 13 and later). */
@Composable
fun LanguageSheet(onDismiss: () -> Unit) {
    val context = LocalContext.current
    ChoiceSheet(
        stringResource(R.string.language),
        listOf("" to stringResource(R.string.as_system), "ar" to stringResource(R.string.arabic), "en" to stringResource(R.string.english)),
        appLanguage(context),
        onChoose = { setAppLanguage(context, it) },
        onDismiss = onDismiss,
    )
}

@Composable
private fun Group(title: String, content: @Composable ColumnScope.() -> Unit) {
    SectionLabel(title)
    ZCard(Modifier.padding(horizontal = 18.dp), content = content)
}

/**
 * yt-dlp's version, and a newer one from the signed list when the user asks
 * (ADR-0012, section 8). Nothing is checked unless the row is tapped.
 */
@Composable
private fun VideoTools() {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    val c = Z.colors
    var version by remember { mutableStateOf<String?>(null) }
    var state by remember { mutableStateOf<ToolsState>(ToolsState.Idle) }
    LaunchedEffect(Unit) { version = runCatching { YtDlp.version(context) }.getOrNull() }
    val check = {
        state = ToolsState.Checking
        scope.launch {
            state = ToolsUpdate.check(context).fold(
                onSuccess = { offer -> if (offer == null) ToolsState.Latest else ToolsState.Offered(offer) },
                onFailure = { ToolsState.Failed },
            )
        }
    }
    val hint = listOfNotNull(
        stringResource(R.string.ytdlp_version, version ?: "…"),
        stringResource(
            when (state) {
                ToolsState.Idle -> R.string.tools_signed
                ToolsState.Checking -> R.string.ytdlp_checking
                ToolsState.Latest -> R.string.ytdlp_latest
                ToolsState.Failed -> R.string.ytdlp_failed
                ToolsState.Done -> R.string.ytdlp_done
                is ToolsState.Offered -> R.string.tools_signed
            },
        ),
    ).joinToString(stringResource(R.string.list_sep))
    when (val s = state) {
        is ToolsState.Offered -> SettingRow(
            stringResource(R.string.video_tools),
            hint = hint,
            value = stringResource(R.string.ytdlp_update_to, s.offer.tools.ytDlpVersion),
            valueColor = c.accent,
            icon = LineIcon.Update,
            iconBackground = c.greenBg,
            iconTint = c.green,
            onClick = {
                state = ToolsState.Checking
                scope.launch {
                    state = if (ToolsUpdate.apply(context, s.offer).isSuccess) ToolsState.Done else ToolsState.Failed
                }
            },
        )
        else -> SettingRow(
            stringResource(R.string.video_tools),
            hint = hint,
            icon = LineIcon.Update,
            iconBackground = c.greenBg,
            iconTint = c.green,
            onClick = if (s == ToolsState.Checking) null else ({ check() }),
        ) {
            Pill(stringResource(if (version != null) R.string.tools_ready else R.string.ytdlp_checking), c.greenBg, c.green, height = 28.dp)
        }
    }
}

private sealed interface ToolsState {
    data object Idle : ToolsState
    data object Checking : ToolsState
    data object Latest : ToolsState
    data object Failed : ToolsState
    data object Done : ToolsState
    data class Offered(val offer: ToolsUpdate.Offer) : ToolsState
}

private fun lookName(look: Look) = when (look) {
    Look.System -> R.string.as_system
    Look.Light -> R.string.look_light
    Look.Dark -> R.string.look_dark
}

private fun languageName(tag: String) = when (tag) {
    "ar" -> R.string.arabic
    "en" -> R.string.english
    else -> R.string.as_system
}

/** The app's own language ("": the system's), from Android 13. */
internal fun appLanguage(context: android.content.Context): String {
    if (Build.VERSION.SDK_INT < Build.VERSION_CODES.TIRAMISU) return ""
    val locales = context.getSystemService(LocaleManager::class.java)?.applicationLocales ?: return ""
    return if (locales.isEmpty) "" else locales[0].language
}

internal fun setAppLanguage(context: android.content.Context, tag: String) {
    if (Build.VERSION.SDK_INT < Build.VERSION_CODES.TIRAMISU) return
    context.getSystemService(LocaleManager::class.java)?.applicationLocales =
        if (tag.isEmpty()) LocaleList.getEmptyLocaleList() else LocaleList.forLanguageTags(tag)
}

/** Gives back the permission of a folder no longer used. */
private fun releaseFolder(context: Context, tree: String) {
    runCatching {
        context.contentResolver.releasePersistableUriPermission(
            Uri.parse(tree),
            Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_GRANT_WRITE_URI_PERMISSION,
        )
    }
}

