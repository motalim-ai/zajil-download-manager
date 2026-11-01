package io.github.motalim_ai.zajil.ui

import android.content.ClipboardManager
import android.content.Context
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableLongStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.pluralStringResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.OffsetMapping
import androidx.compose.ui.text.input.TransformedText
import androidx.compose.ui.text.input.VisualTransformation
import androidx.compose.ui.text.style.TextDirection
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.core.net.toUri
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import io.github.motalim_ai.zajil.R
import io.github.motalim_ai.zajil.downloads
import io.github.motalim_ai.zajil.downloads.BrowserLink
import io.github.motalim_ai.zajil.downloads.FileNames
import io.github.motalim_ai.zajil.downloads.LocalNetwork
import io.github.motalim_ai.zajil.downloads.Queue
import io.github.motalim_ai.zajil.engine.EngineException
import io.github.motalim_ai.zajil.engine.LinkInfo
import io.github.motalim_ai.zajil.video.VideoPages
import kotlinx.coroutines.delay

/** Where a link came from. */
enum class Source {
    Shared,

    /** Found on the clipboard when Zajil opened: the user did not hand it over. */
    Clipboard,

    Typed,

    /** The Quick Settings tile: the user asked for the copied link. */
    Tile,

    /** Found in Zajil's browser (A6): asked about with that page's cookies and address. */
    Browser,
}

/**
 * A link to ask about: shared by another app, found on the clipboard when
 * Zajil opened or asked for with the Quick Settings tile, or typed after
 * «New link» ([url] null until then). Nothing
 * downloads until the user taps «Start»; a shared text with no link has a
 * null [url] and says so.
 */
data class Offer(val url: String?, val source: Source, val link: BrowserLink? = null)

/** What the sheet knows of the file before it is downloaded. */
private sealed interface Found {
    data object Nothing : Found
    data object Looking : Found
    data class Info(val info: LinkInfo) : Found
    data class Failed(val message: Int) : Found
}

/**
 * The new download sheet (the approved design): the file's name, which the
 * user may change, its size and kind, whether it resumes, where it goes
 * and when it starts. A shared or typed link is asked about at once (only
 * the first answer's headers); a link from the clipboard, which the user
 * did not hand over, is not, until they start it.
 */
@Composable
fun AddSheet(offer: Offer, onStart: (url: String, name: String?, start: StartWhen, queue: Long) -> Unit, onDismiss: () -> Unit) {
    val context = LocalContext.current
    if (offer.url == null && offer.source != Source.Typed) {
        AlertDialog(
            onDismissRequest = onDismiss,
            title = { Text(stringResource(R.string.no_link_title)) },
            text = { Text(stringResource(R.string.no_link_text)) },
            confirmButton = { TextButton(onClick = onDismiss) { Text(stringResource(R.string.ok)) } },
            containerColor = Z.colors.surface,
        )
        return
    }
    val c = Z.colors
    val prefs by context.downloads.prefs.prefs.collectAsStateWithLifecycle()
    var link by remember { mutableStateOf(offer.url.orEmpty()) }
    val url = link.trim().takeIf(::isLink)
    val video = url != null && VideoPages.isVideoPage(url)
    var found by remember { mutableStateOf<Found>(Found.Nothing) }
    var name by remember { mutableStateOf("") }
    var nameEdited by remember { mutableStateOf(false) }
    var start by remember { mutableStateOf(StartWhen.Queue) }
    var queue by remember { mutableLongStateOf(Queue.MAIN) }

    LaunchedEffect(url) {
        found = Found.Nothing
        if (!nameEdited) name = url?.let(::nameFromLink).orEmpty()
        if (url == null || video) return@LaunchedEffect
        // The clipboard was not handed over: nothing goes to its server yet
        // (the tile is the user asking for it).
        if (offer.source == Source.Clipboard) return@LaunchedEffect
        // Android lets Zajil on the local network only with the permission,
        // asked when the download starts.
        if (LocalNetwork.isLocal(url) && !LocalNetwork.isGranted(context)) return@LaunchedEffect
        if (offer.source == Source.Typed) delay(400)
        found = Found.Looking
        found = context.downloads.probe(url, offer.link).fold(
            onSuccess = { Found.Info(it) },
            onFailure = { e -> Found.Failed(if (e is EngineException.Probe) problemText(e.problem) else R.string.problem_other) },
        )
        val info = (found as? Found.Info)?.info
        if (info != null && !nameEdited) name = FileNames.withType(info.name, info.contentType)
    }

    val title = stringResource(
        if (offer.source == Source.Clipboard || offer.source == Source.Tile) R.string.offer_copied_title else R.string.new_download,
    )
    ZSheet(onDismiss = onDismiss, label = title) {
        Column(verticalArrangement = Arrangement.spacedBy(16.dp)) {
            Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(10.dp)) {
                ZText(title, Modifier.weight(1f), size = 19.sp, weight = FontWeight.Bold)
                if (offer.source == Source.Shared) {
                    Pill(
                        stringResource(R.string.source_shared), c.bg, c.muted,
                        icon = lineIcon(LineIcon.Shared), height = 28.dp, weight = FontWeight.Normal,
                    )
                }
                if (offer.source == Source.Browser) {
                    Pill(
                        stringResource(R.string.source_browser), c.bg, c.muted,
                        icon = lineIcon(LineIcon.Globe), height = 28.dp, weight = FontWeight.Normal,
                    )
                }
            }

            if (offer.source == Source.Typed) LinkField(link, onLink = { link = it })

            if (url != null && video) {
                FileBox(Kind.Video, stringResource(R.string.video_link), null, editable = false, onName = {}) {
                    ZText(stringResource(R.string.video_link_hint, url.toUri().host.orEmpty()), size = 12.5.sp, color = c.muted)
                }
            } else if (url != null) {
                FileBox(Kind.ofName(name), stringResource(R.string.file_name), name, editable = true, onName = {
                    name = it
                    nameEdited = true
                }) {
                    ZText(infoLine(found, name, url), size = 12.5.sp, color = c.muted, maxLines = 2)
                }
                Chips(found, prefs.connections)
                Places(start, queue, onStart = { when_, inQueue -> start = when_; queue = inQueue })
            }

            Row(Modifier.padding(top = 4.dp), horizontalArrangement = Arrangement.spacedBy(10.dp)) {
                MainButton(
                    stringResource(if (video) R.string.choose_quality else R.string.start_download),
                    onClick = {
                        if (url != null) {
                            // The name the user saw is the name the file gets: the
                            // server cannot change it after this OK.
                            val chosen = name.trim().takeIf { it.isNotEmpty() && (nameEdited || found is Found.Info) }
                            onStart(url, chosen, start, queue)
                        }
                    },
                    modifier = Modifier.weight(2f),
                    enabled = url != null,
                    icon = lineIcon(LineIcon.Downloads),
                )
                if (!video) {
                    QuietButton(stringResource(R.string.later), onClick = {
                        if (url != null) {
                            onStart(url, name.trim().takeIf { it.isNotEmpty() && (nameEdited || found is Found.Info) }, StartWhen.Later, queue)
                        } else {
                            onDismiss()
                        }
                    }, modifier = Modifier.weight(1f))
                }
            }
        }
    }
}

@Composable
private fun LinkField(link: String, onLink: (String) -> Unit) {
    val context = LocalContext.current
    val c = Z.colors
    val shape = RoundedCornerShape(14.dp)
    Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
        Box(
            Modifier
                .weight(1f)
                .heightIn(min = 50.dp)
                .clip(shape)
                .background(c.sunken)
                .border(1.5.dp, c.outline, shape)
                .padding(horizontal = 14.dp),
            contentAlignment = Alignment.CenterStart,
        ) {
            if (link.isEmpty()) ZText(stringResource(R.string.link_hint), color = c.muted, size = 15.sp)
            BasicTextField(
                value = link,
                onValueChange = onLink,
                singleLine = true,
                // Links read left to right, in any language.
                textStyle = TextStyle(fontFamily = Plex, fontSize = 15.sp, color = c.ink, textDirection = TextDirection.Ltr),
                keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Uri, imeAction = ImeAction.Done),
                cursorBrush = SolidColor(c.accent),
                modifier = Modifier.fillMaxWidth().semantics { contentDescription = context.getString(R.string.link) },
            )
        }
        QuietButton(stringResource(R.string.paste), onClick = { pasted(context)?.let(onLink) }, height = 50.dp, color = c.accent)
    }
}

@Composable
private fun FileBox(
    kind: Kind,
    label: String,
    name: String?,
    editable: Boolean,
    onName: (String) -> Unit,
    info: @Composable () -> Unit,
) {
    val c = Z.colors
    val shape = RoundedCornerShape(16.dp)
    Row(
        Modifier
            .fillMaxWidth()
            .clip(shape)
            .background(c.sunken)
            .border(1.dp, c.line, shape)
            .padding(12.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        ColorImage(kind.icon, 44.dp)
        Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(2.dp)) {
            ZText(label, size = 12.sp, color = c.muted)
            if (editable && name != null) {
                BasicTextField(
                    value = name,
                    onValueChange = onName,
                    singleLine = true,
                    textStyle = TextStyle(
                        fontFamily = Plex,
                        fontSize = 15.5.sp,
                        fontWeight = FontWeight.SemiBold,
                        color = c.ink,
                    ),
                    // Shown as one unit, as on the cards; the name typed has no marks.
                    visualTransformation = IsolatedName,
                    cursorBrush = SolidColor(c.accent),
                    modifier = Modifier.fillMaxWidth().semantics { contentDescription = label },
                )
            }
            info()
        }
    }
}

/** [FileNames.display] for a text field: the isolates are shown, never typed. */
private object IsolatedName : VisualTransformation {
    override fun filter(text: AnnotatedString): TransformedText {
        val name = text.text
        val marks = FileNames.marks(name)
        val mapping = object : OffsetMapping {
            override fun originalToTransformed(offset: Int): Int =
                offset + marks.filter { it.first <= offset }.sumOf { it.second.length }

            override fun transformedToOriginal(offset: Int): Int {
                var inserted = 0
                for ((index, mark) in marks) {
                    val at = index + inserted
                    if (offset <= at) break
                    val within = minOf(mark.length, offset - at)
                    inserted += within
                    if (within < mark.length) break
                }
                return (offset - inserted).coerceIn(0, name.length)
            }
        }
        return TransformedText(AnnotatedString(FileNames.display(name)), mapping)
    }
}

/** «2.4 MB · Document · from example.com». */
@Composable
private fun infoLine(found: Found, name: String, url: String): String {
    val host = url.toUri().host.orEmpty()
    val kind = stringResource(kindName(Kind.ofName(name)))
    val from = stringResource(R.string.from_host, host)
    return when (found) {
        Found.Looking -> stringResource(R.string.probing)
        is Found.Failed -> listOf(stringResource(found.message), from).joinToString(" · ")
        is Found.Info -> listOfNotNull(
            found.info.size?.let { Units.size(it.toLong(), sizeUnits()) } ?: stringResource(R.string.size_unknown),
            kind,
            from,
        ).joinToString(" · ")
        Found.Nothing -> listOf(stringResource(R.string.size_when_started), from).joinToString(" · ")
    }
}

@Composable
private fun Chips(found: Found, connections: Int) {
    val c = Z.colors
    val info = (found as? Found.Info)?.info ?: return
    FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
        if (info.isPage) {
            Pill(stringResource(R.string.is_a_page), c.amberBg, c.amber, height = 30.dp)
        }
        if (info.resumable) {
            Pill(stringResource(R.string.resumes), c.greenBg, c.green, icon = lineIcon(LineIcon.Check), height = 30.dp)
            Pill(pluralStringResource(R.plurals.connections, connections, connections), c.sel, c.accent, height = 30.dp)
        } else {
            Pill(stringResource(R.string.does_not_resume), c.amberBg, c.amber, height = 30.dp)
        }
    }
}

/** Where it is saved, and when it starts. */
@Composable
private fun Places(start: StartWhen, queue: Long, onStart: (StartWhen, Long) -> Unit) {
    val c = Z.colors
    val queues by LocalContext.current.downloads.settings.queues.collectAsStateWithLifecycle()
    val inQueue = queues.firstOrNull { it.id == queue } ?: queues.first()
    var choosing by remember { mutableStateOf(false) }
    val now = start == StartWhen.Now
    ZCard {
        Row(
            Modifier.fillMaxWidth().heightIn(min = 56.dp).padding(horizontal = 14.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            Icon(lineIcon(LineIcon.Folder), null, tint = c.muted, modifier = Modifier.size(22.dp))
            Column(Modifier.weight(1f)) {
                ZText(stringResource(R.string.saved_in), size = 12.sp, color = c.muted)
                // A path reads left to right, in any language.
                ZText("Download/Zajil", style = TextStyle(textDirection = TextDirection.Ltr))
            }
        }
        RowLine()
        Row(
            Modifier
                .fillMaxWidth()
                .heightIn(min = 56.dp)
                .clickable(role = Role.Button, onClickLabel = stringResource(R.string.change)) {
                    // One queue: «Now» or the queue. Several: the user picks.
                    if (queues.size > 1) choosing = true else onStart(if (now) StartWhen.Queue else StartWhen.Now, Queue.MAIN)
                }
                .padding(horizontal = 14.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            Icon(lineIcon(LineIcon.Queues), null, tint = c.muted, modifier = Modifier.size(22.dp))
            Column(Modifier.weight(1f)) {
                ZText(stringResource(R.string.when_starts), size = 12.sp, color = c.muted)
                ZText(if (now) stringResource(R.string.start_now_word) else stringResource(R.string.start_with_queue, queueName(inQueue)))
            }
            ZText(
                when {
                    queues.size > 1 -> stringResource(R.string.change)
                    now -> stringResource(R.string.with_queue_instead)
                    else -> stringResource(R.string.now_instead)
                },
                size = 13.sp,
                weight = FontWeight.SemiBold,
                color = c.accent,
            )
        }
    }
    if (choosing) {
        // The queues by name, then «Now, before its turn» (null).
        val choices = queues.map<Queue, Pair<Long?, String>> { it.id to queueName(it) } + (null to stringResource(R.string.start_now_word))
        ChoiceSheet(
            stringResource(R.string.choose_queue),
            choices,
            chosen = if (now) null else inQueue.id,
            onChoose = { id -> if (id == null) onStart(StartWhen.Now, inQueue.id) else onStart(StartWhen.Queue, id) },
            onDismiss = { choosing = false },
        )
    }
}

fun kindName(kind: Kind) = when (kind) {
    Kind.Video -> R.string.kind_video
    Kind.Music -> R.string.kind_music
    Kind.Images -> R.string.kind_image
    Kind.Documents -> R.string.kind_document
    Kind.Programs -> R.string.kind_program
    Kind.Archives -> R.string.kind_archive
    Kind.Other -> R.string.kind_file
}

/** An http(s) link with a host: the only kind the engine downloads. */
fun isLink(text: String): Boolean {
    val uri = text.toUri()
    return (uri.scheme == "http" || uri.scheme == "https") && !uri.host.isNullOrEmpty() && text.none(Char::isWhitespace)
}

/** The link's last path part, before the engine is asked: never its query (AGENTS.md, section 10.3). */
private fun nameFromLink(url: String): String =
    url.toUri().lastPathSegment?.takeIf { it.isNotBlank() } ?: url.toUri().host.orEmpty()

/** The clipboard's text, read only when the user taps «Paste». */
private fun pasted(context: Context): String? =
    context.getSystemService(ClipboardManager::class.java)
        ?.primaryClip
        ?.takeIf { it.itemCount > 0 }
        ?.getItemAt(0)
        ?.coerceToText(context)
        ?.toString()
        ?.trim()
