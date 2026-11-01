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
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.AlertDialog
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
import io.github.motalim_ai.zajil.downloads.Publisher
import io.github.motalim_ai.zajil.downloads.Queue
import io.github.motalim_ai.zajil.downloads.SaveCategory
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
 * The new download sheet (D5): the link, the file's name, which the user
 * may change, its size and kind, whether it resumes, where it goes and
 * when it starts. A shared or typed link is asked about at once (only the
 * first answer's headers); a link from the clipboard, which the user did
 * not hand over, is not, until they start it.
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
    val queues by context.downloads.settings.queues.collectAsStateWithLifecycle()
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

    val title = stringResource(R.string.new_download)
    // The name the user saw is the name the file gets: the server cannot
    // change it after this OK.
    val chosen = { name.trim().takeIf { it.isNotEmpty() && (nameEdited || found is Found.Info) } }
    ZSheet(onDismiss = onDismiss, label = title) {
        Column(verticalArrangement = Arrangement.spacedBy(16.dp)) {
            SheetTitle(title) {
                when (offer.source) {
                    Source.Clipboard, Source.Tile -> Pill(stringResource(R.string.source_clipboard), c.sel, c.blue, icon = LineIcon.Clipboard, height = 32.dp, radius = 16.dp, size = 13.sp, weight = FontWeight.Medium)
                    Source.Shared -> Pill(stringResource(R.string.source_shared), c.sel, c.blue, icon = LineIcon.Share, height = 32.dp, radius = 16.dp, size = 13.sp, weight = FontWeight.Medium)
                    Source.Browser -> Pill(stringResource(R.string.source_browser), c.sel, c.blue, icon = LineIcon.Globe, height = 32.dp, radius = 16.dp, size = 13.sp, weight = FontWeight.Medium)
                    Source.Typed -> {}
                }
            }

            LinkField(link, typed = offer.source == Source.Typed, onLink = { link = it })

            if (url != null && video) {
                FileBox(Kind.Video, stringResource(R.string.video_link), null, editable = false, onName = {}) {
                    ZText(stringResource(R.string.video_link_hint, url.toUri().host.orEmpty()), size = 12.5.sp, color = c.faint)
                }
            } else if (url != null) {
                FileBox(Kind.ofName(name), stringResource(R.string.file_name), name, editable = true, onName = {
                    name = it
                    nameEdited = true
                }) {
                    ZText(infoLine(found, name, url), size = 12.5.sp, color = c.faint, maxLines = 2)
                }
                Chips(found, prefs.connections)
                SavedIn(name)
                When(queues, start, queue, onStart = { when_, inQueue -> start = when_; queue = inQueue })
            }

            val inQueue = queues.firstOrNull { it.id == queue } ?: queues.first()
            Row(horizontalArrangement = Arrangement.spacedBy(10.dp)) {
                MainButton(
                    when {
                        video -> stringResource(R.string.choose_quality)
                        start == StartWhen.Now -> stringResource(R.string.start_download)
                        else -> stringResource(R.string.add_to_queue, queueName(inQueue))
                    },
                    onClick = { if (url != null) onStart(url, chosen(), start, queue) },
                    modifier = Modifier.weight(1f),
                    enabled = url != null,
                )
                if (!video) {
                    QuietButton(stringResource(R.string.later), onClick = {
                        if (url != null) onStart(url, chosen(), StartWhen.Later, queue) else onDismiss()
                    }, modifier = Modifier.widthIn(min = 100.dp))
                }
            }
        }
    }
}

/** The link, left to right in any language; «Paste» when it was typed. */
@Composable
private fun LinkField(link: String, typed: Boolean, onLink: (String) -> Unit) {
    val context = LocalContext.current
    val c = Z.colors
    Row(
        Modifier.fillMaxWidth().heightIn(min = 54.dp).clip(RoundedCornerShape(18.dp)).background(c.sunken).padding(start = 16.dp, end = 6.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(10.dp),
    ) {
        ZIcon(LineIcon.Link, c.muted, 22.dp)
        Box(Modifier.weight(1f), contentAlignment = Alignment.CenterStart) {
            if (link.isEmpty()) ZText(stringResource(R.string.link_hint), color = c.muted, size = 14.5.sp, style = TextStyle(textDirection = TextDirection.Ltr))
            BasicTextField(
                value = link,
                onValueChange = onLink,
                singleLine = true,
                // Links read left to right, in any language.
                textStyle = TextStyle(fontFamily = Plex, fontSize = 14.5.sp, color = c.ink, textDirection = TextDirection.Ltr),
                keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Uri, imeAction = ImeAction.Done),
                cursorBrush = SolidColor(c.accent),
                modifier = Modifier.fillMaxWidth().semantics { contentDescription = context.getString(R.string.link) },
            )
        }
        if (typed) {
            ZText(
                stringResource(R.string.paste),
                Modifier.clip(RoundedCornerShape(14.dp)).clickable(role = Role.Button) { pasted(context)?.let(onLink) }.padding(horizontal = 10.dp, vertical = 12.dp),
                size = 14.sp,
                weight = FontWeight.SemiBold,
                color = c.link,
            )
        }
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
    val shape = RoundedCornerShape(22.dp)
    Row(
        Modifier
            .fillMaxWidth()
            .clip(shape)
            .border(1.dp, c.line, shape)
            .padding(14.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        IconCircle(kind.icon, c.sel, c.blue, size = 48.dp, iconSize = 22.dp)
        Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(2.dp)) {
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
                    // Shown as one unit, as in the lists; the name typed has no marks.
                    visualTransformation = IsolatedName,
                    cursorBrush = SolidColor(c.accent),
                    modifier = Modifier.fillMaxWidth().semantics { contentDescription = label },
                )
            } else {
                ZText(label, size = 15.5.sp, weight = FontWeight.SemiBold)
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

/** «مستند PDF، 2.4 م.ب، من example.com». */
@Composable
private fun infoLine(found: Found, name: String, url: String): String {
    val host = url.toUri().host.orEmpty()
    val kind = stringResource(kindName(Kind.ofName(name)))
    val from = stringResource(R.string.from_host, host)
    val sep = stringResource(R.string.list_sep)
    return when (found) {
        Found.Looking -> stringResource(R.string.probing)
        is Found.Failed -> listOf(stringResource(found.message), from).joinToString(sep)
        is Found.Info -> listOfNotNull(
            kind,
            found.info.size?.let { Units.size(it.toLong(), sizeUnits()) } ?: stringResource(R.string.size_unknown),
            from,
        ).joinToString(sep)
        Found.Nothing -> listOf(stringResource(R.string.size_when_started), from).joinToString(sep)
    }
}

@Composable
private fun Chips(found: Found, connections: Int) {
    val c = Z.colors
    val info = (found as? Found.Info)?.info ?: return
    FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
        if (info.isPage) {
            Pill(stringResource(R.string.is_a_page), c.amberBg, c.amber)
        }
        if (info.resumable) {
            Pill(stringResource(R.string.resumes), c.greenBg, c.green, icon = LineIcon.Check)
            Pill(pluralStringResource(R.plurals.connections, connections, "$connections"), c.sel, c.blue)
        } else {
            Pill(stringResource(R.string.does_not_resume), c.amberBg, c.amber)
        }
    }
}

/** «يُحفظ في»: the chosen folder or Download/Zajil, and its type's folder there. */
@Composable
private fun SavedIn(name: String) {
    val c = Z.colors
    val context = LocalContext.current
    val prefs by context.downloads.prefs.prefs.collectAsStateWithLifecycle()
    val folder = listOfNotNull(
        prefs.saveTree?.let(Publisher::shownTree) ?: "Download/Zajil",
        SaveCategory.of(name)?.takeIf { prefs.folderPerType }?.let { Publisher.folderName(context, it) },
    ).joinToString("/")
    Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
        IconCircle(LineIcon.Folder, c.sunken, c.softInk)
        Column(Modifier.weight(1f)) {
            ZText(stringResource(R.string.saved_in), size = 12.5.sp, color = c.faint)
            // A path reads left to right, in any language.
            ZText(folder, size = 14.5.sp, weight = FontWeight.Medium, maxLines = 1, style = TextStyle(textDirection = TextDirection.Ltr))
        }
    }
}

/**
 * «متى يبدأ»: now, or with a queue. Up to two queues fit beside «الآن»;
 * with more, the third place picks among the others.
 */
@Composable
private fun When(queues: List<Queue>, start: StartWhen, queue: Long, onStart: (StartWhen, Long) -> Unit) {
    val c = Z.colors
    var choosing by remember { mutableStateOf(false) }
    val now = start == StartWhen.Now
    // null: «Now»; a queue's id; or -1: «another queue».
    val other = -1L
    val shown = if (queues.size <= 2) queues.map { it.id } else listOf(queues.first().id, other)
    val chosenOther = queues.size > 2 && !now && queue != queues.first().id
    val options = buildList<Pair<Long?, String>> {
        add(null to stringResource(R.string.now_word))
        shown.forEach { id ->
            val q = queues.firstOrNull { it.id == id }
            add(id to if (q != null) queueName(q) else if (chosenOther) queueName(queues.first { it.id == queue }) else stringResource(R.string.other_queue))
        }
    }
    val chosen: Long? = when {
        now -> null
        chosenOther -> other
        else -> queue
    }
    Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
        ZText(stringResource(R.string.when_starts), size = 13.5.sp, weight = FontWeight.SemiBold, color = c.muted)
        Segmented(options, chosen, onChoose = { id ->
            when (id) {
                null -> onStart(StartWhen.Now, queue)
                other -> choosing = true
                else -> onStart(StartWhen.Queue, id)
            }
        })
    }
    if (choosing) {
        ChoiceSheet(
            stringResource(R.string.choose_queue),
            queues.drop(1).map { it.id to queueName(it) },
            chosen = queue,
            onChoose = { id -> onStart(StartWhen.Queue, id) },
            onDismiss = { choosing = false },
        )
    }
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
