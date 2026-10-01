package soy.iko.fold.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.IntrinsicSize
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.rememberScrollState
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.LinkAnnotation
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.TextLinkStyles
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextDecoration
import androidx.compose.ui.text.withLink
import androidx.compose.ui.text.withStyle
import androidx.compose.ui.unit.dp

/**
 * Light Markdown styling (§10.9): line-based, not a full renderer. Code,
 * bold, emphasis and links are styled, and their markers stay on screen,
 * dimmed, so what is read is what is in the file.
 */
fun inline(text: String, c: FoldColors): AnnotatedString = buildAnnotatedString {
    var i = 0
    val n = text.length
    val dim = SpanStyle(color = c.dim)
    fun plain(s: String) = append(s)
    while (i < n) {
        val ch = text[i]
        when {
            // `code`
            ch == '`' -> {
                val end = text.indexOf('`', i + 1)
                if (end < 0) {
                    plain(text.substring(i)); i = n
                } else {
                    withStyle(dim) { append('`') }
                    withStyle(SpanStyle(fontFamily = FontFamily.Monospace, color = c.code, background = c.codeBackground)) {
                        append(text.substring(i + 1, end))
                    }
                    withStyle(dim) { append('`') }
                    i = end + 1
                }
            }
            // **bold** or __bold__
            (ch == '*' || ch == '_') && text.startsWith("$ch$ch", i) -> {
                val mark = "$ch$ch"
                val end = text.indexOf(mark, i + 2)
                if (end <= i + 2) {
                    plain(mark); i += 2
                } else {
                    withStyle(dim) { append(mark) }
                    withStyle(SpanStyle(fontWeight = FontWeight.Bold)) { append(inline(text.substring(i + 2, end), c)) }
                    withStyle(dim) { append(mark) }
                    i = end + 2
                }
            }
            // *em* or _em_ (an underscore inside a word is a word)
            (ch == '*' || (ch == '_' && (i == 0 || !text[i - 1].isLetterOrDigit()))) &&
                i + 1 < n && !text[i + 1].isWhitespace() -> {
                val end = text.indexOf(ch, i + 1)
                if (end < 0 || text[end - 1].isWhitespace()) {
                    plain(ch.toString()); i++
                } else {
                    withStyle(dim) { append(ch) }
                    withStyle(SpanStyle(fontStyle = FontStyle.Italic)) { append(text.substring(i + 1, end)) }
                    withStyle(dim) { append(ch) }
                    i = end + 1
                }
            }
            // [text](url)
            ch == '[' -> {
                val close = text.indexOf("](", i + 1)
                val end = if (close > 0) text.indexOf(')', close + 2) else -1
                if (close < 0 || end < 0) {
                    plain("["); i++
                } else {
                    val label = text.substring(i + 1, close)
                    val url = text.substring(close + 2, end)
                    withStyle(dim) { append('[') }
                    withLink(LinkAnnotation.Url(url, TextLinkStyles(SpanStyle(color = c.link, textDecoration = TextDecoration.Underline)))) {
                        append(label)
                    }
                    withStyle(dim) { append("]($url)") }
                    i = end + 1
                }
            }
            // a bare link
            text.startsWith("https://", i) || text.startsWith("http://", i) -> {
                var end = i
                while (end < n && !text[end].isWhitespace() && text[end] != ')' && text[end] != '>') end++
                while (end > i && text[end - 1] in ".,;:!?") end--
                val url = text.substring(i, end)
                withLink(LinkAnnotation.Url(url, TextLinkStyles(SpanStyle(color = c.link, textDecoration = TextDecoration.Underline)))) {
                    append(url)
                }
                i = end
            }
            else -> {
                plain(ch.toString()); i++
            }
        }
    }
}

/** One kind of text line, as the reading view styles it. */
private enum class Look { Text, Fence, Code, Quote, Rule }

/**
 * A run of body text: paragraphs, quotes, fenced code on a tinted ground
 * with its fence dimmed and its info string in the accent colour.
 */
@Composable
fun MarkdownBody(lines: List<String>, modifier: Modifier = Modifier, style: TextStyle = MaterialTheme.typography.bodyLarge) {
    val c = LocalFoldColors.current
    Column(modifier) {
        var fence: String? = null
        for (line in lines) {
            val trimmed = line.trimStart()
            val look = when {
                fence != null && trimmed.startsWith(fence) -> {
                    fence = null
                    Look.Fence
                }
                fence != null -> Look.Code
                trimmed.startsWith("```") || trimmed.startsWith("~~~") -> {
                    fence = trimmed.takeWhile { it == trimmed[0] }
                    Look.Fence
                }
                trimmed.startsWith(">") -> Look.Quote
                trimmed.matches(Regex("([-*_])( ?\\1){2,}")) -> Look.Rule
                else -> Look.Text
            }
            BodyLine(line, look, style, c)
        }
    }
}

@Composable
private fun BodyLine(line: String, look: Look, style: TextStyle, c: FoldColors) {
    when (look) {
        Look.Text -> if (line.isBlank()) {
            Box(Modifier.height(8.dp))
        } else {
            Text(inline(line, c), style = style)
        }
        Look.Fence -> {
            val t = line.trimStart()
            val marks = t.takeWhile { it == '`' || it == '~' }
            Text(
                buildAnnotatedString {
                    withStyle(SpanStyle(color = c.dim)) { append(line.substring(0, line.length - t.length + marks.length)) }
                    withStyle(SpanStyle(color = c.accent)) { append(t.substring(marks.length)) }
                },
                style = style.copy(fontFamily = FontFamily.Monospace),
                modifier = Modifier.fillMaxWidth().background(c.codeBackground).padding(horizontal = 6.dp),
            )
        }
        Look.Code -> Box(Modifier.fillMaxWidth().background(c.codeBackground).horizontalScroll(rememberScrollState())) {
            Text(
                line.ifEmpty { " " },
                style = style.copy(fontFamily = FontFamily.Monospace, color = c.code),
                softWrap = false,
                modifier = Modifier.padding(horizontal = 6.dp),
            )
        }
        Look.Quote -> Row(Modifier.height(IntrinsicSize.Min)) {
            Box(Modifier.width(3.dp).fillMaxHeight().background(c.quote))
            Text(inline(line.trimStart().removePrefix(">").trimStart(), c), style = style.copy(color = c.quote), modifier = Modifier.padding(start = 8.dp))
        }
        Look.Rule -> Box(Modifier.fillMaxWidth().padding(vertical = 8.dp).height(1.dp).background(c.guide))
    }
}
