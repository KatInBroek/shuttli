package org.katinbroek.shuttli

import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.material3.*
import androidx.compose.runtime.Composable
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.StrokeJoin
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.graphics.vector.PathParser
import androidx.compose.ui.unit.dp

private val LightColors = lightColorScheme(
    primary = Color(0xFF2B62D9), onPrimary = Color.White,
    primaryContainer = Color(0xFFE3EBFF), onPrimaryContainer = Color(0xFF174DC2),
    secondaryContainer = Color(0xFFD5E1FF), onSecondaryContainer = Color(0xFF162C57),
    background = Color(0xFFF6F7FC), onBackground = Color(0xFF202127),
    surface = Color.White, onSurface = Color(0xFF202127),
    surfaceVariant = Color(0xFFEDF0F7), onSurfaceVariant = Color(0xFF626573),
    outline = Color(0xFF747783), outlineVariant = Color(0xFFE2E5EE)
)
private val DarkColors = darkColorScheme(
    primary = Color(0xFFA9C3FF), onPrimary = Color(0xFF0A2463),
    primaryContainer = Color(0xFF28385E), onPrimaryContainer = Color(0xFFD6E2FF),
    secondaryContainer = Color(0xFF2C4272), onSecondaryContainer = Color(0xFFD6E2FF),
    background = Color(0xFF111217), onBackground = Color(0xFFE4E4EC),
    surface = Color(0xFF1E2027), onSurface = Color(0xFFE4E4EC),
    surfaceVariant = Color(0xFF242733), onSurfaceVariant = Color(0xFFB4B6C1),
    outline = Color(0xFF8F929E), outlineVariant = Color(0xFF363944)
)
@Composable
internal fun ShuttliTheme(content: @Composable () -> Unit) {
    MaterialTheme(colorScheme = if (isSystemInDarkTheme()) DarkColors else LightColors, content = content)
}

/** Native vector icons; no platform-dependent font glyphs. */
internal enum class AppIcon(val path: String) {
    Home("M3 10 L12 3 L21 10 L21 21 L15 21 L15 14 L9 14 L9 21 L3 21 Z"),
    Devices("M4 4 L20 4 L20 17 L4 17 Z M2 21 L22 21"),
    Settings("M3 6 L8 6 M12 6 L21 6 M3 12 L14 12 M18 12 L21 12 M3 18 L6 18 M10 18 L21 18 M8 4 L12 4 L12 8 L8 8 Z M14 10 L18 10 L18 14 L14 14 Z M6 16 L10 16 L10 20 L6 20 Z"),
    Copy("M8 8 L20 8 L20 21 L8 21 Z M16 4 L4 4 L4 17"),
    Paste("M9 3 L15 3 L15 6 L9 6 Z M9 5 L5 5 L5 21 L19 21 L19 5 L15 5"),
    Refresh("M20 5 L20 11 L14 11 M20 11 C19 5 12 2 7 5 C2 8 2 15 7 18 C12 21 18 19 20 15"),
    Back("M15 5 L8 12 L15 19"),
    Chevron("M9 5 L16 12 L9 19"),
    Down("M6 9 L12 15 L18 9"),
    Close("M5 5 L19 19 M5 19 L19 5"),
    Check("M4 12 L9 17 L20 6")
    ;
    val vector: ImageVector by lazy {
        ImageVector.Builder(name, 24.dp, 24.dp, 24f, 24f).addPath(
            pathData = PathParser().parsePathString(path).toNodes(),
            stroke = SolidColor(Color.Black), strokeLineWidth = 1.7f,
            strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round
        ).build()
    }
}
