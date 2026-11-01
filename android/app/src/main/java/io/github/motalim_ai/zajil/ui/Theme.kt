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
 * The desktop's colours (apps/desktop/src/styles/app.css), the approved
 * Android design's (A5 design, 2026-11), light and dark.
 */
@Immutable
data class ZajilColors(
    val dark: Boolean,
    val bg: Color,
    val surface: Color,
    val sunken: Color,
    val ink: Color,
    val muted: Color,
    val line: Color,
    /** Between the rows of one card. */
    val lineSoft: Color,
    /** An outlined button's border, and the sheet's handle. */
    val outline: Color,
    val accent: Color,
    val accentInk: Color,
    val sel: Color,
    /** A chosen row in a list of choices. */
    val picked: Color,
    val track: Color,
    val fab: Color,
    val fabInk: Color,
    val blue: Color,
    val amber: Color,
    val amberBg: Color,
    val amberFill: Color,
    val red: Color,
    val redBg: Color,
    val green: Color,
    val greenBg: Color,
    val grey: Color,
    val greyBg: Color,
    val scrim: Color,
)

private val Light = ZajilColors(
    dark = false,
    bg = Color(0xFFEEF3F8),
    surface = Color(0xFFFFFFFF),
    sunken = Color(0xFFF5F8FB),
    ink = Color(0xFF10263D),
    muted = Color(0xFF4A5D72),
    line = Color(0xFFD8E2EC),
    lineSoft = Color(0xFFE3EAF1),
    outline = Color(0xFFC7D3DF),
    accent = Color(0xFF1C6CD0),
    accentInk = Color(0xFFFFFFFF),
    sel = Color(0xFFDCEBFC),
    picked = Color(0xFFEEF5FE),
    track = Color(0xFFDCE7F2),
    fab = Color(0xFFFFFFFF),
    fabInk = Color(0xFF10263D),
    blue = Color(0xFF1C6CD0),
    amber = Color(0xFF8F5D00),
    amberBg = Color(0xFFFFF1CC),
    amberFill = Color(0xFFB97800),
    red = Color(0xFFB53232),
    redBg = Color(0xFFFBE4E4),
    green = Color(0xFF136E4B),
    greenBg = Color(0xFFDDF3EA),
    grey = Color(0xFF4A5D72),
    greyBg = Color(0xFFE8EDF3),
    scrim = Color(0x6B0A1828),
)

private val Dark = ZajilColors(
    dark = true,
    bg = Color(0xFF07111D),
    surface = Color(0xFF0E2034),
    sunken = Color(0xFF0B1A2B),
    ink = Color(0xFFE3EEF9),
    muted = Color(0xFFA3B8CF),
    line = Color(0xFF1C3550),
    lineSoft = Color(0xFF16304A),
    outline = Color(0xFF2A4866),
    accent = Color(0xFF8ADCFF),
    accentInk = Color(0xFF0A1828),
    sel = Color(0xFF173A5C),
    picked = Color(0xFF12304D),
    track = Color(0xFF183453),
    fab = Color(0xFF173A5C),
    fabInk = Color(0xFFE3EEF9),
    blue = Color(0xFF2E9BFF),
    amber = Color(0xFFFFC94D),
    amberBg = Color(0xFF3A2C08),
    amberFill = Color(0xFFFFC94D),
    red = Color(0xFFFF8A80),
    redBg = Color(0xFF3B1518),
    green = Color(0xFF6FD9A8),
    greenBg = Color(0xFF0F3426),
    grey = Color(0xFFA3B8CF),
    greyBg = Color(0xFF1C3550),
    scrim = Color(0x9902080E),
)

val LocalZajilColors = staticCompositionLocalOf { Light }

/** The current colours: `Z.colors.accent`. */
object Z {
    val colors: ZajilColors
        @Composable get() = LocalZajilColors.current
}

/** IBM Plex Sans Arabic, as on the desktop: in the APK, never fetched (OFL, assets/licenses). */
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
