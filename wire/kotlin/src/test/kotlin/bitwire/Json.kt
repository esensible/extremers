package bitwire

import java.math.BigInteger

/**
 * Minimal JSON reader for the test vectors. Objects keep key order
 * (LinkedHashMap); integers (no fraction or exponent) become Long, or
 * BigInteger beyond 64 bits; other numbers become Double.
 */
internal class Json private constructor(private val s: String) {
    private var i = 0

    companion object {
        fun parse(text: String): Any? {
            val j = Json(text)
            val v = j.value()
            j.ws()
            require(j.i == text.length) { "trailing JSON at ${j.i}" }
            return v
        }
    }

    private fun ws() {
        while (i < s.length && s[i].isWhitespace()) i++
    }

    private fun value(): Any? {
        ws()
        return when (val c = s[i]) {
            '{' -> obj()
            '[' -> arr()
            '"' -> str()
            't' -> lit("true", true)
            'f' -> lit("false", false)
            'n' -> lit("null", null)
            else -> if (c == '-' || c.isDigit()) num() else error("unexpected '$c' at $i")
        }
    }

    private fun lit(word: String, v: Any?): Any? {
        require(s.startsWith(word, i)) { "bad literal at $i" }
        i += word.length
        return v
    }

    private fun obj(): Map<String, Any?> {
        val m = LinkedHashMap<String, Any?>()
        i++
        ws()
        if (s[i] == '}') { i++; return m }
        while (true) {
            ws()
            val k = str()
            ws()
            require(s[i] == ':') { "expected ':' at $i" }
            i++
            m[k] = value()
            ws()
            when (s[i++]) {
                ',' -> continue
                '}' -> return m
                else -> error("expected ',' or '}' at ${i - 1}")
            }
        }
    }

    private fun arr(): List<Any?> {
        val a = ArrayList<Any?>()
        i++
        ws()
        if (s[i] == ']') { i++; return a }
        while (true) {
            a.add(value())
            ws()
            when (s[i++]) {
                ',' -> continue
                ']' -> return a
                else -> error("expected ',' or ']' at ${i - 1}")
            }
        }
    }

    private fun str(): String {
        require(s[i] == '"') { "expected a string at $i" }
        i++
        val sb = StringBuilder()
        while (true) {
            val c = s[i++]
            when (c) {
                '"' -> return sb.toString()
                '\\' -> {
                    when (val e = s[i++]) {
                        '"' -> sb.append('"')
                        '\\' -> sb.append('\\')
                        '/' -> sb.append('/')
                        'b' -> sb.append('\b')
                        'f' -> sb.append('\u000c')
                        'n' -> sb.append('\n')
                        'r' -> sb.append('\r')
                        't' -> sb.append('\t')
                        'u' -> {
                            sb.append(s.substring(i, i + 4).toInt(16).toChar())
                            i += 4
                        }
                        else -> error("bad escape '\\$e' at $i")
                    }
                }
                else -> sb.append(c)
            }
        }
    }

    private fun num(): Any {
        val start = i
        if (s[i] == '-') i++
        var integral = true
        while (i < s.length) {
            val c = s[i]
            if (c.isDigit()) {
                i++
            } else if (c == '.' || c == 'e' || c == 'E' || c == '+' || c == '-') {
                integral = false
                i++
            } else {
                break
            }
        }
        val t = s.substring(start, i)
        if (!integral) return t.toDouble()
        return t.toLongOrNull() ?: BigInteger(t)
    }
}
