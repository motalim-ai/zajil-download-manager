package io.github.motalim_ai.zajil.ui

import android.app.LocaleManager
import android.content.Context
import android.content.Intent
import android.net.Uri
import android.os.Build
import android.os.LocaleList
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Switch
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
import io.github.motalim_ai.zajil.video.ToolsUpdate
import io.github.motalim_ai.zajil.video.YtDlp
import kotlinx.coroutines.launch

private enum class Asking { Language, Look, Connections, SpeedLimit, SavePlace }

/** Settings, grouped as the approved design has them, with the green note at the end. */
@Composable
fun SettingsScreen(bottomPadding: PaddingValues, browser: BrowserController, onSignIns: () -> Unit) {
    val context = LocalContext.current
    val center = context.downloads
    val prefs by center.prefs.prefs.collectAsStateWithLifecycle()
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

    Column(Modifier.fillMaxSize().background(c.bg)) {
        TitleBar(stringResource(R.string.settings))
        Column(
            Modifier
                .verticalScroll(rememberScrollState())
                .padding(12.dp)
                .padding(bottom = bottomPadding.calculateBottomPadding()),
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            Group(stringResource(R.string.group_look)) {
                // Android keeps a language for each app from Android 13.
                if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
                    SettingRow(
                        stringResource(R.string.language),
                        value = stringResource(languageName(appLanguage(context))),
                        onClick = { asking = Asking.Language },
                    )
                    RowLine()
                }
                SettingRow(stringResource(R.string.look), value = stringResource(lookName(prefs.look)), onClick = { asking = Asking.Look })
            }
            Group(stringResource(R.string.group_download)) {
                SettingRow(
                    stringResource(R.string.save_place),
                    value = prefs.saveTree?.let(Publisher::shownTree) ?: "Download/Zajil",
                    valueColor = if (prefs.saveTree == null) c.muted else c.accent,
                    onClick = { asking = Asking.SavePlace },
                )
                RowLine()
                SettingRow(
                    stringResource(R.string.folder_per_type),
                    hint = stringResource(R.string.folder_per_type_hint),
                    onClick = { center.prefs.update { it.copy(folderPerType = !it.folderPerType) } },
                ) {
                    Switch(
                        checked = prefs.folderPerType,
                        onCheckedChange = { on -> center.prefs.update { it.copy(folderPerType = on) } },
                        colors = switchColors(),
                    )
                }
                RowLine()
                SettingRow(
                    stringResource(R.string.connections_per_file),
                    value = "${prefs.connections}",
                    onClick = { asking = Asking.Connections },
                )
                RowLine()
                SettingRow(
                    stringResource(R.string.speed_limit),
                    hint = stringResource(R.string.speed_limit_hint),
                    value = prefs.speedLimit?.let { stringResource(R.string.per_second, Units.size(it, units)) }
                        ?: stringResource(R.string.no_limit),
                    valueColor = if (prefs.speedLimit == null) c.muted else c.accent,
                    onClick = { asking = Asking.SpeedLimit },
                )
            }
            Group(stringResource(R.string.group_video)) { VideoTools() }
            Group(stringResource(R.string.group_browser)) {
                val sites by browser.store.sites.collectAsStateWithLifecycle()
                val recent by browser.store.recent.collectAsStateWithLifecycle()
                val history by browser.store.history.collectAsStateWithLifecycle()
                SettingRow(
                    stringResource(R.string.manage_sign_ins),
                    hint = stringResource(R.string.sign_ins_short),
                    value = pluralStringResource(R.plurals.sites, sites.size, sites.size),
                    onClick = onSignIns,
                )
                RowLine()
                SettingRow(
                    stringResource(R.string.clear_visited),
                    value = if (recent.isEmpty() && history.isEmpty()) stringResource(R.string.nothing_to_clear) else stringResource(R.string.clear),
                    valueColor = if (recent.isEmpty() && history.isEmpty()) c.muted else c.accent,
                    onClick = if (recent.isEmpty() && history.isEmpty()) null else ({ browser.store.clearRecent() }),
                )
            }
            Group(stringResource(R.string.group_protection)) { ProtectionGroup() }
            Group(stringResource(R.string.group_update)) { AppUpdateRows() }
            PromiseNote()
        }
    }

    when (asking) {
        Asking.Language -> ChoiceSheet(
            stringResource(R.string.language),
            listOf("" to stringResource(R.string.as_system), "ar" to stringResource(R.string.arabic), "en" to stringResource(R.string.english)),
            appLanguage(context),
            onChoose = { setAppLanguage(context, it) },
            onDismiss = { asking = null },
        )
        Asking.Look -> ChoiceSheet(
            stringResource(R.string.look),
            Look.entries.map { it to stringResource(lookName(it)) },
            prefs.look,
            onChoose = { look -> center.prefs.update { it.copy(look = look) } },
            onDismiss = { asking = null },
        )
        Asking.Connections -> ChoiceSheet(
            stringResource(R.string.connections_per_file),
            Prefs.CONNECTIONS.map { it to pluralStringResource(R.plurals.connections, it, it) },
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
        null -> {}
    }
}

@Composable
private fun Group(title: String, content: @Composable () -> Unit) {
    Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
        ZText(title, Modifier.padding(horizontal = 6.dp), size = 12.5.sp, weight = FontWeight.Bold, color = Z.colors.muted)
        ZCard { content() }
    }
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
    ).joinToString(" · ")
    when (val s = state) {
        is ToolsState.Offered -> SettingRow(
            stringResource(R.string.video_tools),
            hint = hint,
            value = stringResource(R.string.ytdlp_update_to, s.offer.tools.ytDlpVersion),
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
            value = stringResource(if (version != null) R.string.tools_ready else R.string.ytdlp_checking),
            valueColor = c.green,
            onClick = if (s == ToolsState.Checking) null else ({ check() }),
        )
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
private fun appLanguage(context: android.content.Context): String {
    if (Build.VERSION.SDK_INT < Build.VERSION_CODES.TIRAMISU) return ""
    val locales = context.getSystemService(LocaleManager::class.java)?.applicationLocales ?: return ""
    return if (locales.isEmpty) "" else locales[0].language
}

private fun setAppLanguage(context: android.content.Context, tag: String) {
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

