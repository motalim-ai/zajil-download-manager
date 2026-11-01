package io.github.motalim_ai.zajil.ui

import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Typography
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.Immutable
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.Font
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import io.github.motalim_ai.zajil.R
import io.github.motalim_ai.zajil.downloads.Look

/**
 * The approved D5 design's colours (2026-11-01), light and dark: the
 * hand-over's tokens, each named for what it colours.
 */
@Immutable
data class ZajilColors(
    val dark: Boolean,
    /** The screen's background: a vertical gradient from [bg] to [bgBottom]. */
    val bg: Color,
    val bgBottom: Color,
    /** Cards, sheets, the bar and the header's round buttons. */
    val surface: Color,
    /** A soft grey fill: inactive tabs and chips, icon circles, fields. */
    val sunken: Color,
    /** The icons on [sunken]. */
    val softInk: Color,
    val ink: Color,
    val muted: Color,
    /** The second line under a row's title. */
    val faint: Color,
    /** A card's or an option's border. */
    val line: Color,
    /** Between the rows of one card. */
    val lineSoft: Color,
    /** The sheet's handle, a switch's track when off. */
    val outline: Color,
    /** A row's «›» at the end. */
    val chevron: Color,
    /** The primary blue: buttons, the active tab, the hero card. */
    val accent: Color,
    val accentInk: Color,
    /** A text link on the background («تغيير»). */
    val link: Color,
    /** «عرض الكل». */
    val quietLink: Color,
    /** The live total speed under the app's name. */
    val speed: Color,
    /** The hero card's layers behind it, and its slider's knob. */
    val heroInner: Color,
    val heroOuter: Color,
    val heroKnob: Color,
    /** Secondary text on the blue cards. */
    val heroSub: Color,
    /** The status pairs (tint, colour): active, done, waiting, failed. */
    val sel: Color,
    val blue: Color,
    val green: Color,
    val greenBg: Color,
    val amber: Color,
    val amberBg: Color,
    val red: Color,
    val redBg: Color,
    val purple: Color,
    val purpleBg: Color,
    /** A chosen option's fill in a list of choices. */
    val picked: Color,
    /** A thin progress bar's track. */
    val track: Color,
    val scrim: Color,
    val cardShadow: Color,
    val barShadow: Color,
)

private val Light = ZajilColors(
    dark = false,
    bg = Color(0xFFF5F6F9),
    bgBottom = Color(0xFFE9EBF0),
    surface = Color(0xFFFFFFFF),
    sunken = Color(0xFFF1F3F7),
    softInk = Color(0xFF3A3C44),
    ink = Color(0xFF121722),
    muted = Color(0xFF6B6E78),
    faint = Color(0xFF8A93A3),
    line = Color(0xFFE6E8EE),
    lineSoft = Color(0xFFEDF0F4),
    outline = Color(0xFFD5D9E1),
    chevron = Color(0xFFA3A9B4),
    accent = Color(0xFF0A5FE0),
    accentInk = Color(0xFFFFFFFF),
    link = Color(0xFF0A5FE0),
    quietLink = Color(0xFF5F6B7D),
    speed = Color(0xFF0A5FE0),
    heroInner = Color(0xFF4A8BEF),
    heroOuter = Color(0xFFA4C3F5),
    heroKnob = Color(0xFF0A3FA6),
    heroSub = Color(0xFFD2E2FF),
    sel = Color(0xFFE5EFFF),
    blue = Color(0xFF0A5FE0),
    green = Color(0xFF128A4F),
    greenBg = Color(0xFFE2F6EA),
    amber = Color(0xFFA86A00),
    amberBg = Color(0xFFFFF1D6),
    red = Color(0xFFC4302B),
    redBg = Color(0xFFFDE6E4),
    purple = Color(0xFF7B4BD6),
    purpleBg = Color(0xFFEFE7FD),
    picked = Color(0xFFF2F7FF),
    track = Color(0xFFE6E8EE),
    scrim = Color(0x6B0C101A),
    cardShadow = Color(0x66121722),
    barShadow = Color(0x38121722),
)

private val Dark = ZajilColors(
    dark = true,
    bg = Color(0xFF11141B),
    bgBottom = Color(0xFF0B0D12),
    surface = Color(0xFF1B1F28),
    sunken = Color(0xFF262B35),
    softInk = Color(0xFFD5D9E1),
    ink = Color(0xFFEEF1F6),
    muted = Color(0xFF9AA1AD),
    faint = Color(0xFF8B94A4),
    line = Color(0xFF2F3440),
    lineSoft = Color(0xFF2A2F39),
    outline = Color(0xFF3A404C),
    chevron = Color(0xFF6B7280),
    accent = Color(0xFF0A5FE0),
    accentInk = Color(0xFFFFFFFF),
    link = Color(0xFF8AB8FF),
    quietLink = Color(0xFF8AB8FF),
    speed = Color(0xFF7FB0FF),
    heroInner = Color(0xFF2A5FC0),
    heroOuter = Color(0xFF1D3A73),
    heroKnob = Color(0xFF0A3FA6),
    heroSub = Color(0xFFD2E2FF),
    sel = Color(0xFF16294A),
    blue = Color(0xFF7FB0FF),
    green = Color(0xFF5FDCA0),
    greenBg = Color(0xFF123526),
    amber = Color(0xFFF3B94D),
    amberBg = Color(0xFF3A2C10),
    red = Color(0xFFFF8A80),
    redBg = Color(0xFF3B1518),
    purple = Color(0xFFB598F0),
    purpleBg = Color(0xFF2A2140),
    picked = Color(0xFF16294A),
    track = Color(0xFF2A2F39),
    scrim = Color(0x990C101A),
    cardShadow = Color(0x66000000),
    barShadow = Color(0x99000000),
)

val LocalZajilColors = staticCompositionLocalOf { Light }

/** The current colours: `Z.colors.accent`. */
object Z {
    val colors: ZajilColors
        @Composable get() = LocalZajilColors.current
}

/** IBM Plex Sans Arabic, the only family (D5): in the APK, never fetched (OFL, assets/licenses). */
val Plex = FontFamily(
    Font(R.font.plex_arabic_regular, FontWeight.Normal),
    Font(R.font.plex_arabic_medium, FontWeight.Medium),
    Font(R.font.plex_arabic_semibold, FontWeight.SemiBold),
    Font(R.font.plex_arabic_bold, FontWeight.Bold),
)

private fun Typography.inPlex(): Typography {
    fun TextStyle.plex() = copy(fontFamily = Plex)
    return copy(
        displayLarge = displayLarge.plex(), displayMedium = displayMedium.plex(), displaySmall = displaySmall.plex(),
        headlineLarge = headlineLarge.plex(), headlineMedium = headlineMedium.plex(), headlineSmall = headlineSmall.plex(),
        titleLarge = titleLarge.plex(), titleMedium = titleMedium.plex(), titleSmall = titleSmall.plex(),
        bodyLarge = bodyLarge.plex(), bodyMedium = bodyMedium.plex(), bodySmall = bodySmall.plex(),
        labelLarge = labelLarge.plex(), labelMedium = labelMedium.plex(), labelSmall = labelSmall.plex(),
    )
}

private val PlexTypography = Typography().inPlex()

/** Whether the screen is dark: the user's choice, or the system's. */
@Composable
fun isDark(look: Look): Boolean = when (look) {
    Look.System -> isSystemInDarkTheme()
    Look.Light -> false
    Look.Dark -> true
}

@Composable
fun ZajilTheme(look: Look = Look.System, content: @Composable () -> Unit) {
    val c = if (isDark(look)) Dark else Light
    // Material's own parts (switches, menus, the text field) in the same colours.
    val scheme = if (c.dark) {
        darkColorScheme(
            primary = c.accent, onPrimary = c.accentInk, primaryContainer = c.sel, onPrimaryContainer = c.ink,
            secondary = c.accent, background = c.bg, onBackground = c.ink, surface = c.surface, onSurface = c.ink,
            surfaceVariant = c.sunken, onSurfaceVariant = c.muted, surfaceContainer = c.surface,
            surfaceContainerHigh = c.surface, surfaceContainerLow = c.surface, outline = c.outline,
            outlineVariant = c.line, error = c.red,
        )
    } else {
        lightColorScheme(
            primary = c.accent, onPrimary = c.accentInk, primaryContainer = c.sel, onPrimaryContainer = c.ink,
            secondary = c.accent, background = c.bg, onBackground = c.ink, surface = c.surface, onSurface = c.ink,
            surfaceVariant = c.sunken, onSurfaceVariant = c.muted, surfaceContainer = c.surface,
            surfaceContainerHigh = c.surface, surfaceContainerLow = c.surface, outline = c.outline,
            outlineVariant = c.line, error = c.red,
        )
    }
    CompositionLocalProvider(LocalZajilColors provides c) {
        MaterialTheme(colorScheme = scheme, typography = PlexTypography, content = content)
    }
}
