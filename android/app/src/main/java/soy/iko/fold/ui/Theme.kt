package soy.iko.fold.ui

import android.os.Build
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.material3.ColorScheme
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.dynamicDarkColorScheme
import androidx.compose.material3.dynamicLightColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.Immutable
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext

// Ink on paper, with the amber of the fold marker.
private val Light = lightColorScheme(
    primary = Color(0xFF2E3A4A),
    onPrimary = Color(0xFFFFFFFF),
    primaryContainer = Color(0xFFD9E2EF),
    onPrimaryContainer = Color(0xFF16202C),
    secondary = Color(0xFF8A5A1C),
    onSecondary = Color(0xFFFFFFFF),
    secondaryContainer = Color(0xFFF6DFBF),
    onSecondaryContainer = Color(0xFF2C1A04),
    tertiary = Color(0xFF3D6B5A),
    background = Color(0xFFFBFAF7),
    onBackground = Color(0xFF1B1C1E),
    surface = Color(0xFFFBFAF7),
    onSurface = Color(0xFF1B1C1E),
    surfaceVariant = Color(0xFFE6E3DC),
    onSurfaceVariant = Color(0xFF4A4740),
    surfaceContainerLowest = Color(0xFFFFFFFF),
    surfaceContainerLow = Color(0xFFF5F3EE),
    surfaceContainer = Color(0xFFEFEDE7),
    surfaceContainerHigh = Color(0xFFE9E7E1),
    surfaceContainerHighest = Color(0xFFE3E1DB),
    outline = Color(0xFF7B776F),
    outlineVariant = Color(0xFFCCC8BF),
    error = Color(0xFFB3261E),
)

private val Dark = darkColorScheme(
    primary = Color(0xFFB7C7DC),
    onPrimary = Color(0xFF1E2A38),
    primaryContainer = Color(0xFF34414F),
    onPrimaryContainer = Color(0xFFD9E2EF),
    secondary = Color(0xFFE0A458),
    onSecondary = Color(0xFF3F2700),
    secondaryContainer = Color(0xFF5B3D10),
    onSecondaryContainer = Color(0xFFF6DFBF),
    tertiary = Color(0xFF9ED1BB),
    background = Color(0xFF15181C),
    onBackground = Color(0xFFE3E2DF),
    surface = Color(0xFF15181C),
    onSurface = Color(0xFFE3E2DF),
    surfaceVariant = Color(0xFF3F4246),
    onSurfaceVariant = Color(0xFFC3C2BD),
    surfaceContainerLowest = Color(0xFF101215),
    surfaceContainerLow = Color(0xFF1B1E22),
    surfaceContainer = Color(0xFF1F2226),
    surfaceContainerHigh = Color(0xFF292C30),
    surfaceContainerHighest = Color(0xFF34373B),
    outline = Color(0xFF8D8C88),
    outlineVariant = Color(0xFF45474B),
    error = Color(0xFFF2B8B5),
)

/** The few colours fold means something by, beyond the scheme's roles. */
@Immutable
data class FoldColors(
    val accent: Color,
    val done: Color,
    val dim: Color,
    val warn: Color,
    val code: Color,
    val codeBackground: Color,
    val quote: Color,
    val link: Color,
    val highlight: Color,
    val guide: Color,
)

val LocalFoldColors = staticCompositionLocalOf {
    FoldColors(
        Color.Unspecified, Color.Unspecified, Color.Unspecified, Color.Unspecified, Color.Unspecified,
        Color.Unspecified, Color.Unspecified, Color.Unspecified, Color.Unspecified, Color.Unspecified,
    )
}

private fun foldColors(scheme: ColorScheme, dark: Boolean) = FoldColors(
    accent = if (dark) Color(0xFFE0A458) else Color(0xFFB0711F),
    done = scheme.onSurface.copy(alpha = 0.45f),
    dim = scheme.onSurfaceVariant.copy(alpha = 0.8f),
    warn = if (dark) Color(0xFFFFB86B) else Color(0xFFC25E00),
    code = if (dark) Color(0xFFB5D3B5) else Color(0xFF2F5D3A),
    codeBackground = if (dark) Color(0xFF22262B) else Color(0xFFF0EEE8),
    quote = if (dark) Color(0xFF9DB7D5) else Color(0xFF4A6688),
    link = if (dark) Color(0xFF8FC1FF) else Color(0xFF1D5FAD),
    highlight = if (dark) Color(0x40E0A458) else Color(0x33E0A458),
    guide = scheme.outlineVariant.copy(alpha = 0.7f),
)

@Composable
fun FoldTheme(content: @Composable () -> Unit) {
    val dark = isSystemInDarkTheme()
    val context = LocalContext.current
    val scheme = when {
        Build.VERSION.SDK_INT >= Build.VERSION_CODES.S && dark -> dynamicDarkColorScheme(context)
        Build.VERSION.SDK_INT >= Build.VERSION_CODES.S -> dynamicLightColorScheme(context)
        dark -> Dark
        else -> Light
    }
    androidx.compose.runtime.CompositionLocalProvider(LocalFoldColors provides foldColors(scheme, dark)) {
        MaterialTheme(colorScheme = scheme, content = content)
    }
}

/** The fixed palette, for previews and screenshots that must not vary. */
@Composable
fun FoldThemeFixed(dark: Boolean = false, content: @Composable () -> Unit) {
    val scheme = if (dark) Dark else Light
    androidx.compose.runtime.CompositionLocalProvider(LocalFoldColors provides foldColors(scheme, dark)) {
        MaterialTheme(colorScheme = scheme, content = content)
    }
}
