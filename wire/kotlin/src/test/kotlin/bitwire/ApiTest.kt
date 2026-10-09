package bitwire

import org.junit.jupiter.api.Assertions.assertEquals
import org.junit.jupiter.api.Assertions.assertThrows
import org.junit.jupiter.api.Test
import java.math.BigDecimal
import java.math.BigInteger

/** Kotlin-specific behaviour the shared vectors cannot express. */
class ApiTest {
    // descriptors from vectors/codec.json
    private val u64 = Descriptor.load(unhex("0124bf9615c972254e0108"))
    private val i64 = Descriptor.load(unhex("01a9c1e148dbe199620109"))
    private val u8i8 = Descriptor.load(unhex("0178a5f998afc9f939030f0202020301750169"))
    private val str = Descriptor.load(unhex("01c395031329e7d1a4020f01010c0173"))
    private val floats = Descriptor.load(unhex("0148654624d4fdfa96030f02020a0b01610162"))

    private fun code(block: () -> Unit): String = assertThrows(WireException::class.java) { block() }.code.wire

    @Test
    fun uint64DecodesToULongOverTheFullRange() {
        assertEquals(0uL, Wire.decode(u64, unhex("00")))
        assertEquals(ULong.MAX_VALUE, Wire.decode(u64, unhex("ffffffffffffffffff01")))
        assertEquals(9223372036854775808uL, Wire.decode(u64, unhex("80808080808080808001")))
    }

    @Test
    fun uint64EncodesEveryIntegerType() {
        val max = "ffffffffffffffffff01"
        assertEquals(max, hex(Wire.encode(u64, ULong.MAX_VALUE)))
        assertEquals(max, hex(Wire.encode(u64, BigInteger("18446744073709551615"))))
        assertEquals(max, hex(Wire.encode(u64, BigDecimal("18446744073709551615.000"))))
        assertEquals("80808080808080808001", hex(Wire.encode(u64, 9223372036854775808.0)))
        assertEquals("ac02", hex(Wire.encode(u64, 300)))
        assertEquals("ac02", hex(Wire.encode(u64, 300u)))
        assertEquals("ac02", hex(Wire.encode(u64, 300.0)))
        assertEquals("range", code { Wire.encode(u64, -1L) })
        assertEquals("range", code { Wire.encode(u64, BigInteger.ONE.shiftLeft(64)) })
        assertEquals("range", code { Wire.encode(u64, 18446744073709551616.0) })
        assertEquals("range", code { Wire.encode(u64, Double.NaN) })
        assertEquals("range", code { Wire.encode(u64, BigDecimal("1.5")) })
        assertEquals("shape", code { Wire.encode(u64, "1") })
        assertEquals("shape", code { Wire.encode(u64, null) })
    }

    @Test
    fun int64Limits() {
        assertEquals(Long.MIN_VALUE, Wire.decode(i64, Wire.encode(i64, Long.MIN_VALUE)))
        assertEquals(Long.MAX_VALUE, Wire.decode(i64, Wire.encode(i64, Long.MAX_VALUE)))
        assertEquals("range", code { Wire.encode(i64, 9223372036854775808uL) })
        assertEquals("range", code { Wire.encode(i64, 9.223372036854775808E18) })
    }

    @Test
    fun smallIntegersDecodeToLong() {
        val v = Wire.decode(u8i8, unhex("ff80")) as Map<*, *>
        assertEquals(255L, v["u"])
        assertEquals(-128L, v["i"])
        assertEquals("ff80", hex(Wire.encode(u8i8, mapOf("u" to 255.toUByte(), "i" to (-128).toByte()))))
    }

    @Test
    fun loneSurrogateIsWrittenAsReplacementCharacter() {
        assertEquals("03efbfbd", hex(Wire.encode(str, mapOf("s" to "\uD800"))))
        assertEquals("0561efbfbd62", hex(Wire.encode(str, mapOf("s" to "a\uDC00b"))))
        assertEquals("04f09f9880", hex(Wire.encode(str, mapOf("s" to "😀"))))
    }

    @Test
    fun floatsAcceptFloatAndDouble() {
        val bytes = Wire.encode(floats, mapOf("a" to 6.4f, "b" to 6.4))
        assertEquals("cdcccc409a99999999991940", hex(bytes))
        val v = Wire.decode(floats, bytes) as Map<*, *>
        assertEquals(6.4f.toDouble(), v["a"])
        assertEquals(6.4, v["b"])
        assertEquals("0000c07f000000000000f87f", hex(Wire.encode(floats, mapOf("a" to Float.NaN, "b" to Double.NaN))))
    }

    @Test
    fun decodeWithOffsetReadsOnlyTheSlice() {
        val buf = unhex("ff01ff")
        assertEquals(1uL, Wire.decode(u64, buf, 1, 1))
        assertEquals("truncated", code { Wire.decode(u64, buf, 0, 1) })
    }
}
