package io.github.motalim_ai.zajil.ui

import androidx.compose.animation.core.animateDpAsState
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxScope
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.RowScope
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.navigationBars
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBars
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.selection.selectable
import androidx.compose.foundation.selection.toggleable
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.Text
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.Path
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.asAndroidPath
import androidx.compose.ui.graphics.drawscope.DrawScope
import androidx.compose.ui.graphics.drawscope.drawIntoCanvas
import androidx.compose.ui.graphics.nativeCanvas
import androidx.compose.ui.graphics.toArgb
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.painterResource
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
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.TextUnit
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import io.github.motalim_ai.zajil.R
import java.time.LocalTime
import java.time.format.DateTimeFormatter
import java.time.format.DecimalStyle
import kotlinx.coroutines.launch

/** Text in Zajil's font and colours: the design's sizes are in sp. */
@Composable
fun ZText(
    text: String,
    modifier: Modifier = Modifier,
    size: TextUnit = 14.5.sp,
    weight: FontWeight = FontWeight.Normal,
    color: Color = Z.colors.ink,
    maxLines: Int = Int.MAX_VALUE,
    align: TextAlign? = null,
    style: TextStyle = TextStyle.Default,
) {
    Text(
        text,
        modifier = modifier,
        style = style.merge(TextStyle(fontFamily = Plex, fontSize = size, fontWeight = weight, color = color)),
        maxLines = maxLines,
        overflow = TextOverflow.Ellipsis,
        textAlign = align,
    )
}

/**
 * A CSS box shadow under a rounded box: [blur], [dy] down, and [spread]
 * (negative: smaller than the box). Drawn by Android's shadow layer, which
 * hardware drawing supports from Android 9; before it, no shadow.
 */
fun Modifier.dropShadow(color: Color, blur: Dp, dy: Dp, spread: Dp = 0.dp, radius: Dp = 0.dp): Modifier = drawBehind {
    val s = spread.toPx()
    shadowed(color, blur, dy) { canvas, paint ->
        val r = (radius.toPx() + s).coerceAtLeast(0f)
        canvas.drawRoundRect(-s, -s, size.width + s, size.height + s, r, r, paint)
    }
}

/** [path]'s shadow, as [dropShadow]'s. */
fun DrawScope.drawPathShadow(path: Path, color: Color, blur: Dp, dy: Dp) =
    shadowed(color, blur, dy) { canvas, paint -> canvas.drawPath(path.asAndroidPath(), paint) }

private fun DrawScope.shadowed(color: Color, blur: Dp, dy: Dp, draw: (android.graphics.Canvas, android.graphics.Paint) -> Unit) {
    drawIntoCanvas { canvas ->
        val paint = android.graphics.Paint().apply {
            isAntiAlias = true
            this.color = android.graphics.Color.TRANSPARENT
            // CSS blurs by twice the Gaussian's sigma; Android's radius is about sigma ÷ 0.58.
            setShadowLayer((blur.toPx() * 0.85f).coerceAtLeast(0.1f), 0f, dy.toPx(), color.toArgb())
        }
        draw(canvas.nativeCanvas, paint)
    }
}

/** The cards' shadow: `0 6 14 -10`. */
@Composable
fun Modifier.cardShadow(radius: Dp = 24.dp): Modifier = dropShadow(Z.colors.cardShadow, 14.dp, 6.dp, (-10).dp, radius)

/** A white card: [radius] round, with its shadow. */
@Composable
fun Modifier.card(radius: Dp = 24.dp, color: Color = Z.colors.surface): Modifier =
    cardShadow(radius).clip(RoundedCornerShape(radius)).background(color)

/** The screens' background: the design's soft vertical gradient. */
@Composable
fun ZBackground(modifier: Modifier = Modifier, content: @Composable BoxScope.() -> Unit) {
    val c = Z.colors
    Box(modifier.fillMaxSize().background(Brush.verticalGradient(listOf(c.bg, c.bgBottom))), content = content)
}

/** Zajil's mark on the icon's gradient tile (D5: 52 dp, 16 round). */
@Composable
fun ZajilLogo(size: Dp = 52.dp) {
    Box(
        Modifier
            .size(size)
            .dropShadow(Color(0xD91747C9), 18.dp, 8.dp, (-8).dp, size * 16 / 52)
            .clip(RoundedCornerShape(size * 16 / 52))
            .background(Brush.linearGradient(listOf(Color(0xFF3FA5FF), Color(0xFF1747C9)))),
        contentAlignment = Alignment.Center,
    ) {
        Icon(
            painterResource(R.drawable.ic_stat_zajil),
            contentDescription = stringResource(R.string.logo),
            tint = Color.White,
            modifier = Modifier.size(size * 36 / 52),
        )
    }
}

/** A line icon, tinted. */
@Composable
fun ZIcon(icon: LineIcon, tint: Color = Z.colors.ink, size: Dp = 22.dp, modifier: Modifier = Modifier, label: String? = null) {
    Icon(lineIcon(icon), contentDescription = label, tint = tint, modifier = modifier.size(size))
}

/** A row's icon in its tinted circle. */
@Composable
fun IconCircle(icon: LineIcon, background: Color, tint: Color, size: Dp = 40.dp, iconSize: Dp = 20.dp) {
    Box(Modifier.size(size).clip(CircleShape).background(background), contentAlignment = Alignment.Center) {
        ZIcon(icon, tint, iconSize)
    }
}

/** The header's round button: 48 dp, white, with its own soft shadow. [label] is what TalkBack says. */
@Composable
fun HeaderButton(
    label: String,
    onClick: () -> Unit,
    icon: LineIcon? = null,
    background: Color = Z.colors.surface,
    tint: Color = Z.colors.ink,
    content: (@Composable () -> Unit)? = null,
) {
    Box(
        Modifier
            .size(48.dp)
            .dropShadow(Color(0x4D121722), 12.dp, 4.dp, (-6).dp, 24.dp)
            .clip(CircleShape)
            .background(background)
            .clickable(onClickLabel = label, role = Role.Button, onClick = onClick)
            .semantics { contentDescription = label },
        contentAlignment = Alignment.Center,
    ) {
        if (content != null) content() else if (icon != null) ZIcon(icon, tint)
    }
}

/** A tab's title: «المتصفح», «الطوابير», «الإعدادات», with its buttons at the end. */
@Composable
fun TabHeader(title: String, actions: @Composable RowScope.() -> Unit = {}) {
    Row(
        Modifier
            .fillMaxWidth()
            .windowInsetsPadding(WindowInsets.statusBars)
            .padding(start = 18.dp, end = 18.dp, top = 22.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(10.dp),
    ) {
        ZText(title, Modifier.weight(1f), size = 28.sp, weight = FontWeight.Bold, maxLines = 1)
        actions()
    }
}

/** A screen inside a tab: «back», its title, and one button at the end. */
@Composable
fun SubHeader(title: String, onBack: () -> Unit, end: (@Composable () -> Unit)? = null) {
    Row(
        Modifier
            .fillMaxWidth()
            .windowInsetsPadding(WindowInsets.statusBars)
            .padding(start = 18.dp, end = 18.dp, top = 18.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        HeaderButton(stringResource(R.string.back), onBack, LineIcon.Back)
        ZText(title, Modifier.weight(1f), size = 19.sp, weight = FontWeight.Bold, maxLines = 1)
        if (end != null) end() else Spacer(Modifier.width(48.dp))
    }
}

/** A group's name above its card. */
@Composable
fun SectionLabel(text: String, modifier: Modifier = Modifier) {
    ZText(
        text,
        modifier.padding(start = 22.dp, end = 22.dp, top = 20.dp, bottom = 8.dp),
        size = 13.5.sp,
        weight = FontWeight.SemiBold,
        color = Z.colors.muted,
    )
}

/** A white card of rows: 24 round, with the design's shadow. */
@Composable
fun ZCard(
    modifier: Modifier = Modifier,
    radius: Dp = 24.dp,
    padding: PaddingValues = PaddingValues(horizontal = 16.dp, vertical = 2.dp),
    content: @Composable ColumnScope.() -> Unit,
) {
    Column(modifier.fillMaxWidth().card(radius).padding(padding), content = content)
}

/** The line between two rows of one card. */
@Composable
fun RowLine(color: Color = Z.colors.lineSoft) {
    Box(Modifier.fillMaxWidth().height(1.dp).background(color))
}

/**
 * One row of a card: its icon in a tinted circle, a label and a hint under
 * it, then its value and «›», or its control.
 */
@Composable
fun SettingRow(
    label: String,
    modifier: Modifier = Modifier,
    hint: String? = null,
    value: String? = null,
    icon: LineIcon? = null,
    iconBackground: Color = Z.colors.sunken,
    iconTint: Color = Z.colors.softInk,
    valueColor: Color = Z.colors.muted,
    labelColor: Color = Z.colors.ink,
    onClick: (() -> Unit)? = null,
    chevron: Boolean = onClick != null,
    trailing: (@Composable RowScope.() -> Unit)? = null,
) {
    Row(
        modifier
            .fillMaxWidth()
            .heightIn(min = 58.dp)
            .then(if (onClick != null) Modifier.clickable(onClick = onClick) else Modifier)
            .padding(vertical = 11.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        if (icon != null) IconCircle(icon, iconBackground, iconTint, size = 36.dp)
        Column(Modifier.weight(1f)) {
            ZText(label, size = 15.sp, weight = FontWeight.Medium, color = labelColor)
            if (hint != null) ZText(hint, size = 12.5.sp, color = Z.colors.faint)
        }
        if (value != null) ZText(value, Modifier.widthIn(max = 180.dp), size = 14.sp, color = valueColor, maxLines = 1)
        trailing?.invoke(this)
        if (chevron && trailing == null) ZIcon(LineIcon.Chevron, Z.colors.chevron, 20.dp)
    }
}

/**
 * The design's switch: 46 × 28, on when its knob is at the line's end
 * (left in Arabic). [onBlue]: white on the blue cards.
 */
@Composable
fun ZSwitch(checked: Boolean, onCheckedChange: ((Boolean) -> Unit)?, enabled: Boolean = true, onBlue: Boolean = false) {
    val c = Z.colors
    val track = when {
        onBlue -> if (checked) Color.White else Color(0x4DFFFFFF)
        checked -> c.accent
        else -> c.outline
    }
    val knob = if (onBlue && checked) c.accent else Color.White
    val at by animateDpAsState(if (checked) 21.dp else 3.dp, label = "knob")
    Box(
        Modifier
            .size(width = 46.dp, height = 28.dp)
            .clip(CircleShape)
            .background(if (enabled) track else track.copy(alpha = 0.45f))
            .then(
                if (onCheckedChange != null) {
                    Modifier.toggleable(checked, enabled = enabled, role = Role.Switch, onValueChange = onCheckedChange)
                } else {
                    Modifier
                },
            ),
    ) {
        Box(
            Modifier
                .padding(start = at, top = 3.dp)
                .size(22.dp)
                .dropShadow(Color(0x40000000), 3.dp, 1.dp, radius = 11.dp)
                .clip(CircleShape)
                .background(knob),
        )
    }
}

/** A small rounded label: a status, a time, «مفعّلة». */
@Composable
fun Pill(
    text: String,
    background: Color,
    color: Color,
    modifier: Modifier = Modifier,
    icon: LineIcon? = null,
    height: Dp = 30.dp,
    radius: Dp = 10.dp,
    size: TextUnit = 12.5.sp,
    weight: FontWeight = FontWeight.SemiBold,
) {
    Row(
        modifier
            .height(height)
            .clip(RoundedCornerShape(radius))
            .background(background)
            .padding(horizontal = if (height >= 30.dp) 10.dp else 8.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(6.dp),
    ) {
        if (icon != null) ZIcon(icon, color, 15.dp)
        ZText(text, size = size, weight = weight, color = color, maxLines = 1)
    }
}

/** A filter chip: its count in bold, then its name; blue when chosen. [raised]: white with a shadow, on the background. */
@Composable
fun FilterChip(count: Int?, label: String, on: Boolean, onClick: () -> Unit, raised: Boolean = false, height: Dp = 32.dp) {
    val c = Z.colors
    val shape = CircleShape
    Row(
        Modifier
            .heightIn(min = 48.dp)
            .selectable(selected = on, role = Role.Tab, onClick = onClick),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Row(
            Modifier
                .height(height)
                .then(if (raised && !on) Modifier.dropShadow(Color(0x66121722), 10.dp, 4.dp, (-8).dp, height / 2) else Modifier)
                .clip(shape)
                .background(if (on) c.accent else if (raised) c.surface else c.sunken)
                .padding(horizontal = if (raised) 13.dp else 11.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(5.dp),
        ) {
            val color = if (on) Color.White else c.softInk
            if (count != null) ZText("$count", size = if (raised) 13.sp else 12.5.sp, weight = FontWeight.Bold, color = color)
            ZText(label, size = if (raised) 13.sp else 12.5.sp, weight = FontWeight.Medium, color = color, maxLines = 1)
        }
    }
}

/** A few choices side by side on a soft grey ground: «الآن / الطابور الرئيسي / طابور الليل». */
@Composable
fun <T> Segmented(options: List<Pair<T, String>>, chosen: T, onChoose: (T) -> Unit, modifier: Modifier = Modifier) {
    val c = Z.colors
    Row(
        modifier.fillMaxWidth().clip(RoundedCornerShape(18.dp)).background(c.sunken).padding(4.dp),
        horizontalArrangement = Arrangement.spacedBy(4.dp),
    ) {
        options.forEach { (value, label) ->
            val on = value == chosen
            Box(
                Modifier
                    .weight(1f)
                    .height(42.dp)
                    .then(if (on) Modifier.dropShadow(Color(0x59121722), 8.dp, 2.dp, (-4).dp, 14.dp) else Modifier)
                    .clip(RoundedCornerShape(14.dp))
                    .background(if (on) c.surface else Color.Transparent)
                    .selectable(selected = on, role = Role.RadioButton, onClick = { onChoose(value) }),
                contentAlignment = Alignment.Center,
            ) {
                ZText(
                    label,
                    size = 14.sp,
                    weight = if (on) FontWeight.SemiBold else FontWeight.Medium,
                    color = if (on) c.ink else c.muted,
                    maxLines = 1,
                    align = TextAlign.Center,
                )
            }
        }
    }
}

/** «−  2  +»: how many at once. [onBlue]: white circles on the blue card. */
@Composable
fun Stepper(value: Int, min: Int, max: Int, onChange: (Int) -> Unit, onBlue: Boolean = false) {
    Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(10.dp)) {
        StepButton(LineIcon.Minus, stringResource(R.string.fewer), value > min, onBlue) { onChange(value - 1) }
        ZText(
            "$value",
            Modifier.widthIn(min = 22.dp),
            size = 18.sp,
            weight = FontWeight.Bold,
            color = if (onBlue) Color.White else Z.colors.ink,
            align = TextAlign.Center,
        )
        StepButton(LineIcon.Plus, stringResource(R.string.more), value < max, onBlue) { onChange(value + 1) }
    }
}

@Composable
private fun StepButton(icon: LineIcon, label: String, enabled: Boolean, onBlue: Boolean, onClick: () -> Unit) {
    val c = Z.colors
    Box(
        Modifier
            .size(48.dp)
            .clip(CircleShape)
            .clickable(enabled = enabled, onClickLabel = label, role = Role.Button, onClick = onClick)
            .semantics { contentDescription = label },
        contentAlignment = Alignment.Center,
    ) {
        Box(
            Modifier.size(36.dp).clip(CircleShape).background(if (onBlue) Color.White else c.sunken),
            contentAlignment = Alignment.Center,
        ) {
            val tint = if (onBlue) c.accent else c.ink
            ZIcon(icon, if (enabled) tint else tint.copy(alpha = 0.35f), 20.dp)
        }
    }
}

/** The main button: blue, 58 high, fully round. */
@Composable
fun MainButton(
    text: String,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
    enabled: Boolean = true,
    icon: LineIcon? = null,
    background: Color = Z.colors.accent,
) {
    Row(
        modifier
            .height(58.dp)
            .clip(CircleShape)
            .background(if (enabled) background else Z.colors.outline)
            .clickable(enabled = enabled, role = Role.Button, onClick = onClick)
            .padding(horizontal = 16.dp),
        horizontalArrangement = Arrangement.Center,
        verticalAlignment = Alignment.CenterVertically,
    ) {
        if (icon != null) {
            ZIcon(icon, Color.White, 22.dp)
            Spacer(Modifier.width(8.dp))
        }
        ZText(text, size = 16.5.sp, weight = FontWeight.SemiBold, color = Color.White, maxLines = 1)
    }
}

/** The quiet button beside it: soft grey in a sheet; [raised], white with a shadow on the background. */
@Composable
fun QuietButton(
    text: String,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
    height: Dp = 58.dp,
    color: Color = Z.colors.ink,
    raised: Boolean = false,
) {
    val c = Z.colors
    Box(
        modifier
            .heightIn(min = height)
            .then(if (raised) Modifier.cardShadow(height / 2) else Modifier)
            .clip(CircleShape)
            .background(if (raised) c.surface else c.sunken)
            .clickable(role = Role.Button, onClick = onClick)
            .padding(horizontal = 16.dp),
        contentAlignment = Alignment.Center,
    ) {
        ZText(text, size = if (height < 50.dp) 14.sp else 16.sp, weight = FontWeight.SemiBold, color = color, maxLines = 1)
    }
}

/** A small soft button inside a row: «حدّث الآن», «تشغيل». */
@Composable
fun SoftButton(text: String, onClick: (() -> Unit)?, color: Color = Z.colors.blue, background: Color = Z.colors.sel, height: Dp = 34.dp) {
    Box(
        Modifier
            .heightIn(min = 48.dp)
            .clip(CircleShape)
            .then(if (onClick != null) Modifier.clickable(role = Role.Button, onClick = onClick) else Modifier),
        contentAlignment = Alignment.Center,
    ) {
        Box(
            Modifier.height(height).clip(CircleShape).background(background).padding(horizontal = 14.dp),
            contentAlignment = Alignment.Center,
        ) {
            ZText(text, size = 13.5.sp, weight = FontWeight.SemiBold, color = color, maxLines = 1)
        }
    }
}

/** A bottom sheet in the design's shape: 30 round at the top, with a handle. */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun ZSheet(onDismiss: () -> Unit, label: String, content: @Composable ColumnScope.() -> Unit) {
    val state = rememberModalBottomSheetState(skipPartiallyExpanded = true)
    val scope = rememberCoroutineScope()
    ModalBottomSheet(
        onDismissRequest = onDismiss,
        sheetState = state,
        shape = RoundedCornerShape(topStart = 30.dp, topEnd = 30.dp),
        containerColor = Z.colors.surface,
        contentColor = Z.colors.ink,
        scrimColor = Z.colors.scrim,
        dragHandle = {
            Box(
                Modifier
                    .padding(top = 10.dp, bottom = 6.dp)
                    .size(width = 40.dp, height = 5.dp)
                    .clip(CircleShape)
                    .background(Z.colors.outline),
            )
        },
        modifier = Modifier.semantics { contentDescription = label },
    ) {
        Column(
            Modifier
                .padding(horizontal = 20.dp)
                .padding(top = 4.dp, bottom = 26.dp)
                .windowInsetsPadding(WindowInsets.navigationBars)
                // What the sheet learns after it opened (a file's size, a
                // typed link's details) makes it taller: it opens up to
                // show all of it, its buttons included.
                .onSizeChanged { if (state.isVisible) scope.launch { state.expand() } },
            content = content,
        )
    }
}

/** A sheet's title, 21 bold, and what goes at its end. */
@Composable
fun SheetTitle(title: String, modifier: Modifier = Modifier, end: (@Composable () -> Unit)? = null) {
    Row(modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(10.dp)) {
        ZText(title, Modifier.weight(1f), size = 21.sp, weight = FontWeight.Bold)
        end?.invoke()
    }
}

/** A sheet's «×»: a soft grey circle. */
@Composable
fun SheetClose(onClick: () -> Unit) {
    val label = stringResource(R.string.close)
    Box(
        Modifier.size(48.dp).clip(CircleShape).clickable(onClickLabel = label, role = Role.Button, onClick = onClick).semantics { contentDescription = label },
        contentAlignment = Alignment.Center,
    ) {
        Box(Modifier.size(40.dp).clip(CircleShape).background(Z.colors.sunken), contentAlignment = Alignment.Center) {
            ZIcon(LineIcon.Close, Z.colors.ink, 20.dp)
        }
    }
}

/** A choice among a few, in a sheet: the language, the look, the connections. */
@Composable
fun <T> ChoiceSheet(title: String, choices: List<Pair<T, String>>, chosen: T, onChoose: (T) -> Unit, onDismiss: () -> Unit) {
    ZSheet(onDismiss = onDismiss, label = title) {
        SheetTitle(title, Modifier.padding(bottom = 14.dp))
        Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
            choices.forEach { (value, label) ->
                ChoiceRow(label, selected = value == chosen, onClick = {
                    onChoose(value)
                    onDismiss()
                })
            }
        }
    }
}

/** One option of a list of choices: a radio, a label, a note and a size at the end. */
@Composable
fun ChoiceRow(
    label: String,
    selected: Boolean,
    onClick: () -> Unit,
    note: String? = null,
    end: String? = null,
    enabled: Boolean = true,
    noteColor: Color = Z.colors.green,
) {
    val c = Z.colors
    val shape = RoundedCornerShape(18.dp)
    Row(
        Modifier
            .fillMaxWidth()
            .heightIn(min = 54.dp)
            .clip(shape)
            .background(if (selected) c.picked else c.surface)
            .border(1.5.dp, if (selected) c.accent else c.line, shape)
            .selectable(selected = selected, enabled = enabled, role = Role.RadioButton, onClick = onClick)
            .padding(horizontal = 14.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        Radio(selected)
        ZText(label, size = 15.5.sp, weight = FontWeight.SemiBold, color = if (enabled) c.ink else c.muted, maxLines = 1)
        if (note != null) ZText(note, size = 12.5.sp, weight = FontWeight.Medium, color = noteColor, maxLines = 1)
        Spacer(Modifier.weight(1f))
        if (end != null) ZText(end, size = 14.sp, color = c.muted, align = TextAlign.End, maxLines = 1)
    }
}

@Composable
private fun Radio(selected: Boolean) {
    val c = Z.colors
    Box(
        Modifier
            .size(20.dp)
            .clip(CircleShape)
            .border(if (selected) 6.dp else 2.dp, if (selected) c.accent else Color(0xFFC4C9D3), CircleShape),
    )
}

/** A text field on a soft grey ground, 54 high, with its icon. */
@Composable
fun ZField(
    value: String,
    onValue: (String) -> Unit,
    hint: String,
    modifier: Modifier = Modifier,
    icon: LineIcon? = null,
    type: KeyboardType = KeyboardType.Text,
    direction: TextDirection = TextDirection.Content,
    imeAction: ImeAction = ImeAction.Default,
    onAction: (() -> Unit)? = null,
    background: Color = Z.colors.sunken,
) {
    val c = Z.colors
    Row(
        modifier.fillMaxWidth().heightIn(min = 54.dp).clip(RoundedCornerShape(18.dp)).background(background).padding(horizontal = 16.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(10.dp),
    ) {
        if (icon != null) ZIcon(icon, c.muted, 22.dp)
        Box(Modifier.weight(1f), contentAlignment = Alignment.CenterStart) {
            if (value.isEmpty()) ZText(hint, color = c.muted, size = 15.sp, maxLines = 1)
            BasicTextField(
                value = value,
                onValueChange = onValue,
                singleLine = true,
                textStyle = TextStyle(fontFamily = Plex, fontSize = 15.sp, color = c.ink, textDirection = direction),
                keyboardOptions = KeyboardOptions(keyboardType = type, imeAction = imeAction),
                keyboardActions = KeyboardActions(onAny = { onAction?.invoke() }),
                cursorBrush = SolidColor(c.accent),
                modifier = Modifier.fillMaxWidth().semantics { contentDescription = hint },
            )
        }
    }
}

/** A thin progress bar; [fraction] null: moving, its end unknown. */
@Composable
fun ThinBar(fraction: Float?, modifier: Modifier = Modifier, color: Color = Z.colors.accent, track: Color = Z.colors.track, height: Dp = 4.dp) {
    Box(modifier.fillMaxWidth().height(height).clip(CircleShape).background(track)) {
        if (fraction == null) {
            androidx.compose.material3.LinearProgressIndicator(
                Modifier.fillMaxSize(),
                color = color,
                trackColor = track,
                gapSize = 0.dp,
            )
        } else {
            Box(Modifier.fillMaxWidth(fraction.coerceIn(0f, 1f)).fillMaxHeight().clip(CircleShape).background(color))
        }
    }
}

/** A short note on a coloured ground. */
@Composable
fun Note(text: String, background: Color, color: Color, icon: LineIcon) {
    Row(
        Modifier.fillMaxWidth().clip(RoundedCornerShape(18.dp)).background(background).padding(horizontal = 14.dp, vertical = 12.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(10.dp),
    ) {
        ZIcon(icon, color, 20.dp)
        ZText(text, size = 13.sp, color = color)
    }
}

/** The design's quiet line under a sheet: a small icon and a muted sentence. */
@Composable
fun QuietNote(text: String, icon: LineIcon) {
    Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
        ZIcon(icon, Z.colors.muted, 18.dp, Modifier.offset(y = 2.dp))
        ZText(text, size = 13.sp, color = Z.colors.muted)
    }
}

/** A time of day as the user reads it, with Western digits as on the desktop: «2:00 ص», «2:00 AM». */
@Composable
fun timeText(time: LocalTime): String {
    val locale = LocalConfiguration.current.locales[0]
    return time.format(DateTimeFormatter.ofPattern("h:mm a", locale).withDecimalStyle(DecimalStyle.STANDARD))
}

/** The units of [Units.size] in the user's language. */
@Composable
fun sizeUnits() = SizeUnits(
    stringResource(R.string.unit_bytes),
    stringResource(R.string.unit_kib),
    stringResource(R.string.unit_mib),
    stringResource(R.string.unit_gib),
)

/**
 * Whether the phone asks for no animations (its "Remove animations", or an
 * animator scale of 0): what moves then snaps to its place instead.
 */
@Composable
fun reducedMotion(): Boolean {
    val resolver = LocalContext.current.contentResolver
    return remember(resolver) { android.provider.Settings.Global.getFloat(resolver, android.provider.Settings.Global.ANIMATOR_DURATION_SCALE, 1f) == 0f }
}
