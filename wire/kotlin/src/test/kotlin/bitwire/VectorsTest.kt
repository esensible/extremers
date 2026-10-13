package bitwire

import org.junit.jupiter.api.Assertions.assertArrayEquals
import org.junit.jupiter.api.Assertions.assertEquals
import org.junit.jupiter.api.Assertions.assertTrue
import org.junit.jupiter.api.Assertions.fail
import org.junit.jupiter.api.DynamicContainer.dynamicContainer
import org.junit.jupiter.api.DynamicNode
import org.junit.jupiter.api.DynamicTest.dynamicTest
import org.junit.jupiter.api.TestFactory
import java.io.File
import java.math.BigInteger

/**
 * Runs the shared golden vectors (../vectors, SPEC §8) directly: every
 * round-trip case, decode refusal, encode refusal and descriptor refusal.
 */
class VectorsTest {
    private val dir = File(System.getProperty("bitwire.vectors") ?: "../vectors")

    @Suppress("UNCHECKED_CAST")
    private fun read(name: String): Map<String, Any?> {
        val doc = Json.parse(File(dir, name).readText(Charsets.UTF_8)) as Map<String, Any?>
        assertEquals(1L, doc["format"], "$name format")
        return doc
    }

    @Suppress("UNCHECKED_CAST")
    @TestFactory
    fun codec(): List<DynamicNode> {
        val schemas = read("codec.json")["schemas"] as List<Map<String, Any?>>
        return schemas.map { s ->
            val name = s["name"] as String
            val dhex = s["descriptor"] as String
            val tests = ArrayList<DynamicNode>()
            tests += dynamicTest("loads") {
                val d = Descriptor.load(unhex(dhex))
                assertEquals(s["hash"], d.hash)
                assertEquals(1, d.version)
                assertEquals(dhex.length / 2, d.size)
            }
            for (c in s["cases"] as List<Map<String, Any?>>) {
                tests += dynamicTest("case: ${c["name"]}") { roundTrip(dhex, c) }
            }
            for (r in s["decode_refusals"] as List<Map<String, Any?>>) {
                tests += dynamicTest("decode refusal: ${r["name"]}") {
                    val d = Descriptor.load(unhex(dhex))
                    val bytes = unhex(r["wire"] as String)
                    refused(r["error"] as String) { Wire.decode(d, bytes) }
                }
            }
            for (r in s["encode_refusals"] as List<Map<String, Any?>>) {
                tests += dynamicTest("encode refusal: ${r["name"]}") {
                    val d = Descriptor.load(unhex(dhex))
                    val value = prepare(d.root, r["value"])
                    refused(r["error"] as String) { Wire.encode(d, value) }
                }
            }
            dynamicContainer(name, tests)
        }
    }

    @Suppress("UNCHECKED_CAST")
    @TestFactory
    fun descriptors(): List<DynamicNode> {
        val refusals = read("descriptors.json")["refusals"] as List<Map<String, Any?>>
        return refusals.map { r ->
            dynamicTest("descriptor refusal: ${r["name"]}") {
                val bytes = unhex(r["descriptor"] as String)
                refused(r["error"] as String) { Descriptor.load(bytes) }
            }
        }
    }

    @Suppress("UNCHECKED_CAST")
    private fun roundTrip(dhex: String, c: Map<String, Any?>) {
        val d = Descriptor.load(unhex(dhex))
        val wire = unhex(c["wire"] as String)
        val value = withF64(c["value"], c["value_f64"] as Map<String, String>?)
        val expected = if (c.containsKey("decoded")) {
            withF64(c["decoded"], c["decoded_f64"] as Map<String, String>?)
        } else {
            value
        }
        val decodeOnly = c["decode_only"] == true
        if (!decodeOnly) {
            val encoded = Wire.encode(d, prepare(d.root, value))
            assertEquals(c["wire"], hex(encoded), "encode(value)")
        }
        val decoded = Wire.decode(d, wire)
        check(d.root, expected, decoded, "")
        if (!decodeOnly) {
            // One encoding per value (SPEC §1): re-encoding the decoded value gives the same bytes.
            assertArrayEquals(wire, Wire.encode(d, decoded), "encode(decode(wire))")
        }
    }

    private fun refused(code: String, block: () -> Unit) {
        try {
            block()
        } catch (e: WireException) {
            assertEquals(code, e.code.wire, "refusal code (${e.message})")
            return
        }
        fail<Unit>("expected a '$code' refusal, got none")
    }

    // ---- value preparation --------------------------------------------------

    /** Replaces the numbers at the JSON pointers of an `*_f64` map by their exact bits. */
    @Suppress("UNCHECKED_CAST")
    private fun withF64(value: Any?, f64: Map<String, String>?): Any? {
        if (f64 == null) return value
        var root = value
        for ((ptr, bits) in f64) {
            val d = Double.fromBits(java.lang.Long.parseUnsignedLong(bits, 16))
            if (ptr == "") {
                root = d
                continue
            }
            val tokens = ptr.substring(1).split("/").map { it.replace("~1", "/").replace("~0", "~") }
            var cur: Any? = root
            for (t in tokens.dropLast(1)) {
                cur = if (cur is List<*>) cur[t.toInt()] else (cur as Map<String, Any?>)[t]
            }
            val last = tokens.last()
            if (cur is MutableList<*>) (cur as MutableList<Any?>)[last.toInt()] = d
            else (cur as MutableMap<String, Any?>)[last] = d
        }
        return root
    }

    /**
     * Vector files write byte strings as hex: turn them into ByteArrays at
     * BYTES nodes. Lenient (leaves anything unexpected as is) so refusal
     * values keep their faults.
     */
    private fun prepare(n: Node, v: Any?): Any? = when (n.op) {
        OP_BYTES -> if (v is String) unhex(v) else v
        OP_OBJECT -> if (v is Map<*, *>) prepareFields(n, v) else v
        OP_ARRAY -> if (v is List<*>) v.map { prepare(n.item!!, it) } else v
        OP_UNION -> {
            val i = if (v is Map<*, *>) n.values.indexOf(v[n.tag]) else -1
            if (i >= 0) prepareFields(n.branches[i], v as Map<*, *>) else v
        }
        OP_VARIANTS -> if (v is Map<*, *>) {
            val out = prepareFields(n, v)
            for (i in n.variants.indices) {
                val name = n.variantNames[i]
                if (out.containsKey(name)) out[name] = prepare(n.variants[i], out[name])
            }
            out
        } else {
            v
        }
        else -> v
    }

    private fun prepareFields(n: Node, v: Map<*, *>): LinkedHashMap<Any?, Any?> {
        val out = LinkedHashMap<Any?, Any?>(v)
        for (i in n.fields.indices) {
            val name = n.fieldNames[i]
            if (out.containsKey(name)) out[name] = prepare(n.fields[i], out[name])
        }
        return out
    }

    // ---- comparison -----------------------------------------------------------

    /** Compares a decoded value with the expected JSON value, checking the Kotlin types too. */
    private fun check(n: Node, exp: Any?, act: Any?, at: String) {
        val where = if (at == "") "the root" else at
        when (n.op) {
            OP_BOOL -> assertEquals(exp, act as? Boolean, where)
            OP_U8, OP_I8, OP_U16, OP_I16, OP_U32, OP_I32, OP_I64 -> {
                assertTrue(act is Long, "$where: expected a Long, got ${act?.javaClass}")
                assertEquals(big(exp), BigInteger.valueOf(act as Long), where)
            }
            OP_U64 -> {
                assertTrue(act is ULong, "$where: expected a ULong, got ${act?.javaClass}")
                assertEquals(big(exp), BigInteger((act as ULong).toString()), where)
            }
            OP_F32, OP_F64 -> {
                assertTrue(act is Double, "$where: expected a Double, got ${act?.javaClass}")
                val e = (exp as Number).toDouble()
                val a = act as Double
                if (!(e.isNaN() && a.isNaN())) {
                    assertEquals(hex64(e), hex64(a), "$where: bits")
                }
                if (n.op == OP_F32 && !a.isNaN()) assertEquals(a, a.toFloat().toDouble(), "$where: not a binary32")
            }
            OP_STRING, OP_ENUM -> assertEquals(exp, act as? String, where)
            OP_BYTES -> {
                assertTrue(act is ByteArray, "$where: expected a ByteArray")
                assertEquals(exp, hex(act as ByteArray), where)
            }
            OP_OBJECT -> checkFields(n, exp, act, at)
            OP_ARRAY -> {
                assertTrue(act is List<*>, "$where: expected a List")
                val e = exp as List<*>
                val a = act as List<*>
                assertEquals(e.size, a.size, "$where: length")
                for (i in e.indices) check(n.item!!, e[i], a[i], "$at/$i")
            }
            OP_UNION -> {
                assertTrue(act is Map<*, *>, "$where: expected a Map")
                val a = act as Map<*, *>
                assertEquals(n.tag, a.keys.firstOrNull(), "$where: tag property first")
                val i = n.values.indexOf(a[n.tag])
                assertTrue(i >= 0, "$where: decoded tag")
                assertEquals((exp as Map<*, *>)[n.tag], a[n.tag], "$where: tag")
                checkFields(n.branches[i], exp, act, at)
            }
            OP_VARIANTS -> {
                checkFields(n, exp, act, at)
                val e = exp as Map<*, *>
                val present = n.variantNames.indices.filter { e.containsKey(n.variantNames[it]) }
                assertEquals(1, present.size, "$where: one variant in the expected value")
                val i = present[0]
                val name = n.variantNames[i]
                check(n.variants[i], e[name], (act as Map<*, *>)[name], "$at/$name")
            }
            else -> fail("unknown op ${n.op}")
        }
    }

    private fun checkFields(n: Node, exp: Any?, act: Any?, at: String) {
        val where = if (at == "") "the root" else at
        assertTrue(act is Map<*, *>, "$where: expected a Map")
        val e = exp as Map<*, *>
        val a = act as Map<*, *>
        assertEquals(e.keys.toList(), a.keys.toList(), "$where: keys in wire order")
        for (i in n.fields.indices) {
            val name = n.fieldNames[i]
            if (e.containsKey(name)) check(n.fields[i], e[name], a[name], "$at/$name")
        }
    }

    private fun big(v: Any?): BigInteger = when (v) {
        is Long -> BigInteger.valueOf(v)
        is BigInteger -> v
        else -> fail("expected an integer in the vector, got $v")
    }

    private fun hex64(d: Double) = String.format("%016x", d.toRawBits())
}

internal fun unhex(s: String): ByteArray = ByteArray(s.length / 2) { s.substring(it * 2, it * 2 + 2).toInt(16).toByte() }

internal fun hex(b: ByteArray): String = b.joinToString("") { String.format("%02x", it.toInt() and 0xff) }
