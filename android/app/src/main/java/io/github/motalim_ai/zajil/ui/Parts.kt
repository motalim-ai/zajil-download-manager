package io.github.motalim_ai.zajil.ui

import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.RowScope
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.navigationBars
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.selection.selectable
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.Text
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.TextUnit
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import io.github.motalim_ai.zajil.R
import java.time.LocalTime
import java.time.format.DateTimeFormatter
import java.time.format.DecimalStyle

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
        overflow = androidx.compose.ui.text.style.TextOverflow.Ellipsis,
        textAlign = align,
    )
}

/** Zajil's mark on the icon's gradient tile. */
@Composable
fun ZajilLogo(size: Dp = 34.dp) {
    Box(
        Modifier
            .size(size)
            .clip(RoundedCornerShape(size * 9 / 34))
            .background(Brush.linearGradient(listOf(Color(0xFF3FA5FF), Color(0xFF1747C9)))),
        contentAlignment = Alignment.Center,
    ) {
        Icon(
            painterResource(R.drawable.ic_stat_zajil),
            contentDescription = stringResource(R.string.logo),
            tint = Color.White,
            modifier = Modifier.size(size * 24 / 34),
        )
    }
}

@Composable
fun ColorImage(icon: ColorIcon, size: Dp, modifier: Modifier = Modifier) {
    Image(colorIcon(icon, Z.colors.surface), contentDescription = null, modifier = modifier.size(size))
}

/** A small rounded label: a status, or what a file can do. */
@Composable
fun Pill(
    text: String,
    background: Color,
    color: Color,
    modifier: Modifier = Modifier,
    icon: ImageVector? = null,
    height: Dp = 22.dp,
    size: TextUnit = 12.5.sp,
    weight: FontWeight = FontWeight.SemiBold,
) {
    Row(
        modifier
            .height(height)
            .clip(CircleShape)
            .background(background)
            .padding(horizontal = if (height > 26.dp) 10.dp else 8.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(6.dp),
    ) {
        if (icon != null) Icon(icon, contentDescription = null, tint = color, modifier = Modifier.size(14.dp))
        ZText(text, size = size, weight = weight, color = color, maxLines = 1)
    }
}

/** A round button with one of the coloured icons; [label] is what TalkBack says. */
@Composable
fun RoundIconButton(icon: ColorIcon, label: String, onClick: () -> Unit, modifier: Modifier = Modifier) {
    Box(
        modifier
            .size(48.dp)
            .clip(CircleShape)
            .clickable(onClickLabel = label, role = Role.Button, onClick = onClick)
            .semantics { contentDescription = label },
        contentAlignment = Alignment.Center,
    ) {
        Box(
            Modifier
                .size(44.dp)
                .clip(CircleShape)
                .background(Z.colors.sunken)
                .border(1.dp, Z.colors.line, CircleShape),
            contentAlignment = Alignment.Center,
        ) {
            ColorImage(icon, 22.dp)
        }
    }
}

/** A plain icon in the bar: search, more. */
@Composable
fun BarIconButton(icon: LineIcon, label: String, onClick: () -> Unit) {
    Box(
        Modifier
            .size(48.dp)
            .clip(CircleShape)
            .clickable(onClickLabel = label, role = Role.Button, onClick = onClick)
            .semantics { contentDescription = label },
        contentAlignment = Alignment.Center,
    ) {
        Icon(lineIcon(icon), contentDescription = null, tint = Z.colors.muted, modifier = Modifier.size(22.dp))
    }
}

/** A white card with a thin border: a queue, a group of settings. */
@Composable
fun ZCard(modifier: Modifier = Modifier, radius: Dp = 16.dp, content: @Composable ColumnScope.() -> Unit) {
    val shape = RoundedCornerShape(radius)
    Column(
        modifier
            .fillMaxWidth()
            .clip(shape)
            .background(Z.colors.surface)
            .border(1.dp, Z.colors.line, shape),
        content = content,
    )
}

/** The line between two rows of one card. */
@Composable
fun RowLine() {
    Box(Modifier.fillMaxWidth().height(1.dp).background(Z.colors.lineSoft))
}

/** One row of a card: a label, a hint under it, and its value or control at the end. */
@Composable
fun SettingRow(
    label: String,
    modifier: Modifier = Modifier,
    hint: String? = null,
    value: String? = null,
    valueColor: Color = Z.colors.accent,
    leading: (@Composable () -> Unit)? = null,
    onClick: (() -> Unit)? = null,
    minHeight: Dp = 56.dp,
    trailing: (@Composable RowScope.() -> Unit)? = null,
) {
    Row(
        modifier
            .fillMaxWidth()
            .heightIn(min = minHeight)
            .then(if (onClick != null) Modifier.clickable(onClick = onClick) else Modifier)
            .padding(horizontal = 14.dp, vertical = 6.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        leading?.invoke()
        Column(Modifier.weight(1f)) {
            ZText(label)
            if (hint != null) ZText(hint, size = 12.sp, color = Z.colors.muted)
        }
        if (value != null) {
            ZText(value, size = 13.5.sp, weight = FontWeight.SemiBold, color = valueColor, maxLines = 2)
        }
        trailing?.invoke(this)
    }
}

/** «−  2  +»: how many at once. */
@Composable
fun Stepper(value: Int, min: Int, max: Int, onChange: (Int) -> Unit) {
    val shape = RoundedCornerShape(12.dp)
    Row(
        Modifier.clip(shape).border(1.dp, Z.colors.outline, shape),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        StepButton("−", stringResource(R.string.fewer), value > min) { onChange(value - 1) }
        ZText("$value", Modifier.width(36.dp), size = 15.sp, weight = FontWeight.Bold, align = TextAlign.Center)
        StepButton("+", stringResource(R.string.more), value < max) { onChange(value + 1) }
    }
}

@Composable
private fun StepButton(sign: String, label: String, enabled: Boolean, onClick: () -> Unit) {
    Box(
        Modifier
            .size(width = 48.dp, height = 48.dp)
            .background(Z.colors.sunken)
            .clickable(enabled = enabled, onClickLabel = label, role = Role.Button, onClick = onClick)
            .semantics { contentDescription = label },
        contentAlignment = Alignment.Center,
    ) {
        ZText(sign, size = 18.sp, weight = FontWeight.SemiBold, color = if (enabled) Z.colors.ink else Z.colors.outline)
    }
}

/** The sheet's main button: filled, 52 high. */
@Composable
fun MainButton(
    text: String,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
    enabled: Boolean = true,
    icon: ImageVector? = null,
) {
    val shape = RoundedCornerShape(16.dp)
    Row(
        modifier
            .height(52.dp)
            .clip(shape)
            .background(if (enabled) Z.colors.accent else Z.colors.outline)
            .clickable(enabled = enabled, role = Role.Button, onClick = onClick)
            .padding(horizontal = 12.dp),
        horizontalArrangement = Arrangement.Center,
        verticalAlignment = Alignment.CenterVertically,
    ) {
        if (icon != null) {
            Icon(icon, contentDescription = null, tint = Z.colors.accentInk, modifier = Modifier.size(20.dp))
            Spacer(Modifier.width(8.dp))
        }
        ZText(text, size = 16.sp, weight = FontWeight.SemiBold, color = Z.colors.accentInk, maxLines = 1)
    }
}

/** The sheet's quiet button: outlined. */
@Composable
fun QuietButton(text: String, onClick: () -> Unit, modifier: Modifier = Modifier, height: Dp = 52.dp, color: Color = Z.colors.ink) {
    val shape = RoundedCornerShape(if (height < 52.dp) 12.dp else 16.dp)
    Box(
        modifier
            .heightIn(min = height)
            .clip(shape)
            .background(Z.colors.surface)
            .border(1.dp, Z.colors.outline, shape)
            .clickable(role = Role.Button, onClick = onClick)
            .padding(horizontal = 14.dp),
        contentAlignment = Alignment.Center,
    ) {
        ZText(text, size = if (height < 52.dp) 13.5.sp else 15.sp, weight = FontWeight.SemiBold, color = color, maxLines = 1)
    }
}

/** A bottom sheet in the design's shape: 26 round at the top, with a handle. */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun ZSheet(onDismiss: () -> Unit, label: String, content: @Composable ColumnScope.() -> Unit) {
    ModalBottomSheet(
        onDismissRequest = onDismiss,
        sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true),
        shape = RoundedCornerShape(topStart = 26.dp, topEnd = 26.dp),
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
                .padding(bottom = 22.dp)
                .windowInsetsPadding(WindowInsets.navigationBars),
            content = content,
        )
    }
}

/** A choice among a few, in a sheet: the language, the look, the connections. */
@Composable
fun <T> ChoiceSheet(title: String, choices: List<Pair<T, String>>, chosen: T, onChoose: (T) -> Unit, onDismiss: () -> Unit) {
    ZSheet(onDismiss = onDismiss, label = title) {
        ZText(title, size = 19.sp, weight = FontWeight.Bold, modifier = Modifier.padding(bottom = 12.dp))
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

/** One row of a list of choices: a radio, a label, a note and a size at the end. */
@Composable
fun ChoiceRow(
    label: String,
    selected: Boolean,
    onClick: () -> Unit,
    note: String? = null,
    end: String? = null,
    enabled: Boolean = true,
) {
    val shape = RoundedCornerShape(14.dp)
    Row(
        Modifier
            .fillMaxWidth()
            .heightIn(min = 48.dp)
            .clip(shape)
            .background(if (selected) Z.colors.picked else Z.colors.surface)
            .border(1.5.dp, if (selected) Z.colors.accent else Z.colors.line, shape)
            .selectable(selected = selected, enabled = enabled, role = Role.RadioButton, onClick = onClick)
            .padding(horizontal = 14.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        Radio(selected)
        ZText(
            label,
            Modifier.weight(1f),
            size = 15.sp,
            weight = FontWeight.SemiBold,
            color = if (enabled) Z.colors.ink else Z.colors.muted,
            maxLines = 1,
        )
        if (note != null) ZText(note, size = 12.sp, color = Z.colors.muted, maxLines = 1)
        if (end != null) {
            ZText(end, Modifier.widthIn(min = 64.dp), size = 13.sp, color = Z.colors.muted, align = TextAlign.End, maxLines = 1)
        }
    }
}

@Composable
private fun Radio(selected: Boolean) {
    Box(
        Modifier
            .size(20.dp)
            .clip(CircleShape)
            .border(2.dp, if (selected) Z.colors.accent else Z.colors.muted, CircleShape),
        contentAlignment = Alignment.Center,
    ) {
        if (selected) Box(Modifier.size(10.dp).clip(CircleShape).background(Z.colors.accent))
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
