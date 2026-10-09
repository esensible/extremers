package bitwire

import java.math.BigDecimal
import java.math.BigInteger

/**
 * Decodes and encodes bitwire messages by walking a loaded [Descriptor]
 * (see SPEC.md). Both directions are a single pass over the descriptor and
 * the bytes: no intermediate tree, no name lookups beyond the object's own
 * properties. Every call has its own state, so concurrent calls are safe.
 *
 * ## Values
 *
 * | Node | [decode] produces | [encode] accepts |
 * |---|---|---|
 * | `BOOL` | [Boolean] | [Boolean] |
 * | `U8`, `I8`, `U16`, `I16`, `U32`, `I32`, `I64` | [Long] | any integer value, see below |
 * | `U64` | [ULong] (always, also below 2^63) | any integer value, see below |
 * | `F32` | [Double] (the binary32 widened exactly) | any number; rounded to binary32 as `toFloat()` does |
 * | `F64` | [Double] | any number |
 * | `STRING` | [String] | [String] (a lone surrogate is written as U+FFFD) |
 * | `BYTES` | [ByteArray] (a fresh copy) | [ByteArray] |
 * | `ENUM` | [String] | [String], one of the values |
 * | `OBJECT` | `Map<String, Any?>` ([LinkedHashMap], keys in wire order, absent optionals omitted) | `Map<*, *>`: only declared keys are read; a missing key is absent; a key mapped to `null` is refused (`shape`) |
 * | `ARRAY` | `List<Any?>` | `List<*>` |
 * | `UNION` | `Map<String, Any?>`, tag property first | `Map<*, *>` holding the tag property |
 *
 * **uint64.** `U64` always decodes to [ULong], so every value up to 2^64 - 1
 * is exact and the type does not depend on the magnitude; the other integer
 * nodes decode to [Long]. [decode] never raises [WireCode.PRECISION].
 *
 * **Integers on encode** may be [Byte], [Short], [Int], [Long], [UByte],
 * [UShort], [UInt], [ULong], [java.math.BigInteger], [java.math.BigDecimal],
 * or a [Float]/[Double] (or other [Number]) holding a whole value. A
 * non-integer type is refused as `shape`; a non-whole number or a value
 * outside the node's width as `range`. Integers are never rounded or clamped.
 */
object Wire {
    /**
     * Decodes [bytes] (the whole array) with [desc].
     *
     * @throws WireException [WireCode.TRUNCATED], [WireCode.TRAILING] or [WireCode.INVALID].
     */
    @JvmStatic
    fun decode(desc: Descriptor, bytes: ByteArray): Any? = decode(desc, bytes, 0, bytes.size)

    /** Decodes the [length] bytes of [bytes] starting at [offset]; nothing outside that range is read. */
    @JvmStatic
    fun decode(desc: Descriptor, bytes: ByteArray, offset: Int, length: Int): Any? {
        if (offset < 0 || length < 0 || offset > bytes.size - length) {
            throw IndexOutOfBoundsException("offset $offset, length $length, size ${bytes.size}")
        }
        return Reader(bytes, offset, offset + length).message(desc.root)
    }

    /**
     * Encodes [value] with [desc] into a new byte array.
     *
     * @throws WireException [WireCode.SHAPE] or [WireCode.RANGE].
     */
    @JvmStatic
    fun encode(desc: Descriptor, value: Any?): ByteArray = Writer().message(desc.root, value)
}

// ---- decode -----------------------------------------------------------------

private class Reader(private val b: ByteArray, private var p: Int, private val end: Int) {
    private var bv = 0   // current bit byte's value
    private var used = 8 // bits used in it; 8 = no room (also before the first)

    private fun truncated(): Nothing = throw WireException(WireCode.TRUNCATED, "message ends early")
    private fun invalid(detail: String): Nothing = throw WireException(WireCode.INVALID, detail)

    fun message(root: Node): Any? {
        val v = node(root)
        if ((bv ushr used) != 0) invalid("non-zero unused bits in a bit byte")
        if (p != end) throw WireException(WireCode.TRAILING, "bytes after the end of the message")
        return v
    }

    private fun bits(w: Int): Int {
        if (used + w > 8) {
            if ((bv ushr used) != 0) invalid("non-zero unused bits in a bit byte")
            if (p >= end) truncated()
            bv = b[p++].toInt() and 0xff
            used = 0
        }
        val v = (bv ushr used) and ((1 shl w) - 1)
        used += w
        return v
    }

    private fun u8(): Int {
        if (p >= end) truncated()
        return b[p++].toInt() and 0xff
    }

    /** LEB128 of at most [maxBytes] bytes as unsigned 64-bit bits (SPEC §4.1). */
    private fun varint(maxBytes: Int): Long {
        var v = 0L
        var shift = 0
        var i = 0
        var byte: Int
        while (true) {
            if (p >= end) truncated()
            byte = b[p++].toInt() and 0xff
            val g = (byte and 0x7f).toLong()
            if (shift == 63 && g > 1) invalid("varint exceeds 64 bits")
            v = v or (g shl shift)
            i++
            shift += 7
            if (byte < 0x80) break
            if (i == maxBytes) invalid("varint too long")
        }
        if (i > 1 && byte == 0) invalid("varint has a redundant final zero byte")
        return v
    }

    /** A string, byte string or array length: a uint32 varint. */
    private fun length(): Long {
        if (p < end && b[p] >= 0) return b[p++].toLong()
        val v = varint(5)
        if (v > 0xffffffffL) invalid("length exceeds 2^32-1")
        return v
    }

    private fun unsigned(maxBytes: Int, max: Long): Long {
        if (p < end && b[p] >= 0) return b[p++].toLong()
        val v = varint(maxBytes)
        if (max != 0L && v > max) invalid("varint exceeds the type's width")
        return v
    }

    private fun signed(maxBytes: Int, max: Long): Long {
        val z = unsigned(maxBytes, max)
        return (z ushr 1) xor -(z and 1L)
    }

    private fun node(n: Node): Any? {
        when (n.op) {
            OP_BOOL -> return bits(1) == 1
            OP_U8 -> return u8().toLong()
            OP_I8 -> return u8().toByte().toLong()
            OP_U16 -> return unsigned(3, 0xffffL)
            OP_U32 -> return unsigned(5, 0xffffffffL)
            OP_U64 -> return unsigned(10, 0L).toULong()
            OP_I16 -> return signed(3, 0xffffL)
            OP_I32 -> return signed(5, 0xffffffffL)
            OP_I64 -> return signed(10, 0L)
            OP_F32 -> {
                if (end - p < 4) truncated()
                val bits = (b[p].toInt() and 0xff) or
                    ((b[p + 1].toInt() and 0xff) shl 8) or
                    ((b[p + 2].toInt() and 0xff) shl 16) or
                    ((b[p + 3].toInt() and 0xff) shl 24)
                p += 4
                return Float.fromBits(bits).toDouble()
            }
            OP_F64 -> {
                if (end - p < 8) truncated()
                var bits = 0L
                for (i in 7 downTo 0) bits = (bits shl 8) or (b[p + i].toLong() and 0xff)
                p += 8
                return Double.fromBits(bits)
            }
            OP_STRING -> {
                val len = length()
                if (len > end - p) truncated()
                val s = utf8Decode(b, p, len.toInt()) ?: invalid("invalid UTF-8")
                p += len.toInt()
                return s
            }
            OP_BYTES -> {
                val len = length()
                if (len > end - p) truncated()
                val v = b.copyOfRange(p, p + len.toInt())
                p += len.toInt()
                return v
            }
            OP_ENUM -> {
                if (n.bits == 0) return n.values[0]
                val i = bits(n.bits)
                if (i >= n.n) invalid("enum index out of range")
                return n.values[i]
            }
            OP_OBJECT -> return fields(n, LinkedHashMap(mapCapacity(n.fields.size)))
            OP_ARRAY -> {
                val len = length()
                // Every item takes at least one bit, so a length beyond what
                // the remaining bytes can hold ends in `truncated`; do not let
                // it size the list.
                val cap = minOf(len, (end - p).toLong() * 8 + 8, 1L shl 20).toInt()
                val list = ArrayList<Any?>(cap)
                val item = n.item!!
                var i = 0L
                while (i < len) {
                    list.add(node(item))
                    i++
                }
                return list
            }
            OP_UNION -> {
                val i = if (n.bits == 0) 0 else bits(n.bits)
                if (i >= n.n) invalid("union index out of range")
                val br = n.branches[i]
                val obj = LinkedHashMap<String, Any?>(mapCapacity(br.fields.size + 1))
                obj[n.tag] = n.values[i]
                return fields(br, obj)
            }
        }
        throw IllegalStateException("unreachable op ${n.op}")
    }

    private fun fields(n: Node, obj: LinkedHashMap<String, Any?>): Map<String, Any?> {
        val fields = n.fields
        val names = n.fieldNames
        for (i in fields.indices) {
            val f = fields[i]
            if (f.optional && bits(1) == 0) continue
            obj[names[i]] = node(f)
        }
        return obj
    }

    private fun mapCapacity(n: Int): Int = if (n < 3) n + 1 else n + n / 3 + 1
}

// ---- encode -----------------------------------------------------------------

private const val TWO63 = 9.223372036854775808E18
private const val TWO64 = 1.8446744073709551616E19

private class Writer {
    private var buf = ByteArray(64)
    private var p = 0
    private var bb = -1    // position of the current bit byte
    private var used = 8   // bits used in it; 8 = no room

    // The last integer read by integer(): its 64 bits, and whether it is an
    // unsigned value >= 2^63 (then `ival` holds its bits, negative as a Long).
    private var ival = 0L
    private var ibig = false

    // Keys from the root to the value being encoded, for refusal messages.
    private val keys = ArrayList<Any>()

    fun message(root: Node, value: Any?): ByteArray {
        node(root, value)
        return buf.copyOf(p)
    }

    // ---- refusals

    private fun at(key: Any? = null): String {
        if (keys.isEmpty() && key == null) return "the root"
        val sb = StringBuilder()
        for (k in keys) sb.append('/').append(k.toString().replace("~", "~0").replace("/", "~1"))
        if (key != null) sb.append('/').append(key.toString().replace("~", "~0").replace("/", "~1"))
        return sb.toString()
    }

    private fun shape(what: String, key: Any? = null): Nothing =
        throw WireException(WireCode.SHAPE, "${at(key)}: $what")

    private fun range(what: String, key: Any? = null): Nothing =
        throw WireException(WireCode.RANGE, "${at(key)}: $what")

    private fun outOfRange(type: String): Nothing = range("integer out of range for $type")

    // ---- bytes

    private fun ensure(n: Int) {
        if (p + n <= buf.size && p + n >= 0) return
        val need = p.toLong() + n
        if (need > Int.MAX_VALUE - 8) throw OutOfMemoryError("message larger than a byte array can hold")
        var size = buf.size.toLong() * 2
        while (size < need) size *= 2
        buf = buf.copyOf(minOf(size, (Int.MAX_VALUE - 8).toLong()).toInt())
    }

    private fun byte(v: Int) {
        ensure(1)
        buf[p++] = v.toByte()
    }

    private fun bits(v: Int, w: Int) {
        if (used + w > 8) {
            ensure(1)
            bb = p
            buf[p++] = 0
            used = 0
        }
        buf[bb] = (buf[bb].toInt() or (v shl used)).toByte()
        used += w
    }

    /** Unsigned LEB128 of the 64 bits of [value]. */
    private fun varint(value: Long) {
        ensure(10)
        var v = value
        while ((v and 0x7fL.inv()) != 0L) {
            buf[p++] = ((v and 0x7f) or 0x80).toByte()
            v = v ushr 7
        }
        buf[p++] = v.toByte()
    }

    private fun zigzag(v: Long) = varint((v shl 1) xor (v shr 63))

    // ---- integers

    /** Reads an integer value into ival/ibig, or refuses it. */
    private fun integer(v: Any?) {
        ibig = false
        when (v) {
            is Long -> ival = v
            is Int -> ival = v.toLong()
            is Short -> ival = v.toLong()
            is Byte -> ival = v.toLong()
            is ULong -> { ival = v.toLong(); ibig = ival < 0 }
            is UInt -> ival = v.toLong()
            is UShort -> ival = v.toLong()
            is UByte -> ival = v.toLong()
            is Double -> fromDouble(v)
            is Float -> fromDouble(v.toDouble())
            is BigInteger -> fromBig(v)
            is BigDecimal -> {
                val bi = try {
                    v.toBigIntegerExact()
                } catch (e: ArithmeticException) {
                    range("expected a whole number")
                }
                fromBig(bi)
            }
            is Number -> fromDouble(v.toDouble())
            else -> shape("expected a number")
        }
    }

    private fun fromDouble(d: Double) {
        if (d.isNaN() || d.isInfinite() || Math.floor(d) != d) range("expected a whole number")
        if (d >= -TWO63 && d < TWO63) {
            ival = d.toLong()
        } else if (d >= TWO63 && d < TWO64) {
            // exact: doubles in [2^63, 2^64) are multiples of 2^11
            ival = (d - TWO63).toLong() xor Long.MIN_VALUE
            ibig = true
        } else {
            range("integer out of range for 64 bits")
        }
    }

    private fun fromBig(v: BigInteger) {
        val bl = v.bitLength()
        if (bl <= 63) {
            ival = v.toLong()
        } else if (bl == 64 && v.signum() > 0) {
            ival = v.toLong()
            ibig = true
        } else {
            range("integer out of range for 64 bits")
        }
    }

    // ---- floats

    private fun float(v: Any?): Float = when (v) {
        is Float -> v
        is Double -> v.toFloat()
        is Long -> v.toFloat()
        is Int -> v.toFloat()
        is ULong -> v.toFloat()
        is UInt -> v.toFloat()
        is UShort -> v.toFloat()
        is UByte -> v.toFloat()
        is BigDecimal -> v.toFloat()
        is BigInteger -> v.toFloat()
        is Number -> v.toDouble().toFloat()
        else -> shape("expected a number")
    }

    private fun double(v: Any?): Double = when (v) {
        is Double -> v
        is ULong -> v.toDouble()
        is UInt -> v.toDouble()
        is UShort -> v.toDouble()
        is UByte -> v.toDouble()
        is Number -> v.toDouble()
        else -> shape("expected a number")
    }

    // ---- strings

    private fun string(s: String) {
        val n = s.length
        var len = 0L
        var i = 0
        while (i < n) {
            val c = s[i].code
            len += when {
                c < 0x80 -> 1
                c < 0x800 -> 2
                c in 0xd800..0xdbff && i + 1 < n && s[i + 1].code in 0xdc00..0xdfff -> { i++; 4 }
                else -> 3 // BMP, or a lone surrogate written as U+FFFD
            }
            i++
        }
        if (len > Int.MAX_VALUE) throw OutOfMemoryError("string larger than a byte array can hold")
        varint(len)
        ensure(len.toInt())
        val b = buf
        var q = p
        i = 0
        while (i < n) {
            var c = s[i].code
            if (c < 0x80) {
                b[q++] = c.toByte()
            } else if (c < 0x800) {
                b[q++] = (0xc0 or (c shr 6)).toByte()
                b[q++] = (0x80 or (c and 63)).toByte()
            } else if (c in 0xd800..0xdbff && i + 1 < n && s[i + 1].code in 0xdc00..0xdfff) {
                c = 0x10000 + ((c - 0xd800) shl 10) + (s[++i].code - 0xdc00)
                b[q++] = (0xf0 or (c shr 18)).toByte()
                b[q++] = (0x80 or ((c shr 12) and 63)).toByte()
                b[q++] = (0x80 or ((c shr 6) and 63)).toByte()
                b[q++] = (0x80 or (c and 63)).toByte()
            } else {
                if (c in 0xd800..0xdfff) c = 0xfffd // lone surrogate, as TextEncoder does
                b[q++] = (0xe0 or (c shr 12)).toByte()
                b[q++] = (0x80 or ((c shr 6) and 63)).toByte()
                b[q++] = (0x80 or (c and 63)).toByte()
            }
            i++
        }
        p = q
    }

    // ---- the walk

    private fun node(n: Node, v: Any?) {
        when (n.op) {
            OP_BOOL -> when (v) {
                true -> bits(1, 1)
                false -> bits(0, 1)
                else -> shape("expected a boolean")
            }
            OP_U8 -> {
                integer(v)
                if (ibig || ival < 0 || ival > 255) outOfRange("uint8")
                byte(ival.toInt())
            }
            OP_I8 -> {
                integer(v)
                if (ibig || ival < -128 || ival > 127) outOfRange("int8")
                byte(ival.toInt())
            }
            OP_U16 -> {
                integer(v)
                if (ibig || ival < 0 || ival > 0xffff) outOfRange("uint16")
                varint(ival)
            }
            OP_U32 -> {
                integer(v)
                if (ibig || ival < 0 || ival > 0xffffffffL) outOfRange("uint32")
                varint(ival)
            }
            OP_U64 -> {
                integer(v)
                if (!ibig && ival < 0) outOfRange("uint64")
                varint(ival)
            }
            OP_I16 -> {
                integer(v)
                if (ibig || ival < -32768 || ival > 32767) outOfRange("int16")
                zigzag(ival)
            }
            OP_I32 -> {
                integer(v)
                if (ibig || ival < Int.MIN_VALUE || ival > Int.MAX_VALUE) outOfRange("int32")
                zigzag(ival)
            }
            OP_I64 -> {
                integer(v)
                if (ibig) outOfRange("int64")
                zigzag(ival)
            }
            OP_F32 -> {
                val f = float(v)
                // NaN is always the canonical quiet NaN (SPEC §4.4)
                val bits = if (f.isNaN()) 0x7fc00000 else f.toRawBits()
                ensure(4)
                buf[p++] = bits.toByte()
                buf[p++] = (bits ushr 8).toByte()
                buf[p++] = (bits ushr 16).toByte()
                buf[p++] = (bits ushr 24).toByte()
            }
            OP_F64 -> {
                val d = double(v)
                var bits = if (d.isNaN()) 0x7ff8000000000000L else d.toRawBits()
                ensure(8)
                for (i in 0 until 8) {
                    buf[p++] = bits.toByte()
                    bits = bits ushr 8
                }
            }
            OP_STRING -> {
                if (v !is String) shape("expected a string")
                string(v)
            }
            OP_BYTES -> {
                if (v !is ByteArray) shape("expected a ByteArray")
                varint(v.size.toLong())
                ensure(v.size)
                System.arraycopy(v, 0, buf, p, v.size)
                p += v.size
            }
            OP_ENUM -> {
                if (v !is String) shape("expected a string")
                val i = n.index[v] ?: range("\"$v\" is not one of the enum values")
                if (n.bits != 0) bits(i, n.bits)
            }
            OP_OBJECT -> {
                if (v !is Map<*, *>) shape("expected an object (Map)")
                fields(n, v)
            }
            OP_ARRAY -> {
                if (v !is List<*>) shape("expected an array (List)")
                val len = v.size
                varint(len.toLong())
                val item = n.item!!
                val depth = keys.size
                keys.add(0)
                if (v is RandomAccess) {
                    for (i in 0 until len) {
                        keys[depth] = i
                        node(item, v[i])
                    }
                } else {
                    var i = 0
                    for (x in v) {
                        keys[depth] = i++
                        node(item, x)
                    }
                }
                keys.removeAt(depth)
            }
            OP_UNION -> {
                if (v !is Map<*, *>) shape("expected an object (Map)")
                val t = v[n.tag]
                if (t !is String) shape("expected a tag string", n.tag)
                val i = n.index[t] ?: range("unknown tag \"$t\"", n.tag)
                if (n.bits != 0) bits(i, n.bits)
                fields(n.branches[i], v)
            }
            else -> throw IllegalStateException("unreachable op ${n.op}")
        }
    }

    /** Reads only the declared fields; any other entries of [m] are ignored. */
    private fun fields(n: Node, m: Map<*, *>) {
        val fields = n.fields
        val names = n.fieldNames
        for (i in fields.indices) {
            val f = fields[i]
            val name = names[i]
            val x = m[name]
            // A key mapped to null is present (and then refused as `shape`):
            // null is never a value (SPEC §2.4).
            val has = x != null || m.containsKey(name)
            if (f.optional) {
                bits(if (has) 1 else 0, 1)
                if (!has) continue
            } else if (!has) {
                shape("missing required property", name)
            }
            keys.add(name)
            node(f, x)
            keys.removeAt(keys.size - 1)
        }
    }
}
