package io.github.motalim_ai.zajil.ui

import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.ui.graphics.Color

// The blues of Zajil's icon (assets/icon).
private val Deep = Color(0xFF1747C9)
private val Sky = Color(0xFF3FA5FF)

private val Light = lightColorScheme(
    primary = Deep,
    onPrimary = Color.White,
    primaryContainer = Color(0xFFDCEEFF),
    onPrimaryContainer = Color(0xFF0A2A80),
    secondary = Sky,
)

private val Dark = darkColorScheme(
    primary = Sky,
    onPrimary = Color(0xFF00205F),
    primaryContainer = Deep,
    onPrimaryContainer = Color(0xFFDCEEFF),
    secondary = Color(0xFF9CCBFF),
)

@Composable
fun ZajilTheme(content: @Composable () -> Unit) {
    MaterialTheme(colorScheme = if (isSystemInDarkTheme()) Dark else Light, content = content)
}
