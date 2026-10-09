package bitwire

internal const val OP_BOOL = 1
internal const val OP_U8 = 2
internal const val OP_I8 = 3
internal const val OP_U16 = 4
internal const val OP_I16 = 5
internal const val OP_U32 = 6
internal const val OP_I32 = 7
internal const val OP_U64 = 8
internal const val OP_I64 = 9
internal const val OP_F32 = 10
internal const val OP_F64 = 11
internal const val OP_STRING = 12
internal const val OP_BYTES = 13
internal const val OP_ENUM = 14
internal const val OP_OBJECT = 15
internal const val OP_ARRAY = 16
internal const val OP_UNION = 17

private const val OPTIONAL = 0x80
private const val FORMAT_VERSION = 1
private const val MAX_DEPTH = 32
private const val MAX_FIELDS = 256
private const val MAX_U32 = 0xffffffffL

private val NO_NODES = arrayOf<Node>()
private val NO_NAMES = arrayOf<String>()

/**
 * One loaded descriptor node (SPEC §6.1), with everything the walk needs
 * precomputed: bit widths, names, and value-to-index maps.
 */
internal class Node(@JvmField val op: Int, @JvmField val optional: Boolean) {
    /** Encodes to nothing (SPEC §6.3 rule 8). */
    @JvmField var zero = false

    /** ENUM / UNION: number of values or branches. */
    @JvmField var n = 0

    /** ENUM / UNION: index width, bitLength(n - 1). */
    @JvmField var bits = 0

    /** First name of this node in the names section. */
    @JvmField var name0 = 0

    /** True for a UNION branch. */
    @JvmField var branch = false

    /** ENUM: the values; UNION: the tag values, in index order. */
    @JvmField var values: Array<String> = NO_NAMES

    /** ENUM / UNION: value or tag to index. */
    @JvmField var index: HashMap<String, Int> = HashMap(0)

    /** OBJECT: the fields and their names, in wire order. */
    @JvmField var fields: Array<Node> = NO_NODES
    @JvmField var fieldNames: Array<String> = NO_NAMES

    /** ARRAY: the item node. */
    @JvmField var item: Node? = null

    /** UNION: the branch objects and the tag property name. */
    @JvmField var branches: Array<Node> = NO_NODES
    @JvmField var tag: String = ""
}

/**
 * A validated descriptor (SPEC §6). Load it once with [load] and reuse it:
 * it is immutable and may be shared between threads.
 */
class Descriptor internal constructor(
    /** Format version (always 1). */
    val version: Int,
    private val hashBytes: ByteArray,
    /** Total number of nodes. */
    val nodeCount: Long,
    internal val root: Node,
    /** Descriptor size in bytes. */
    val size: Int,
) {
    /** The 8-byte schema hash as 16 lowercase hex digits (SPEC §5). */
    val hash: String = hashBytes.joinToString("") { ((it.toInt() and 0xff) or 0x100).toString(16).substring(1) }

    /** A copy of the 8 raw schema hash bytes. */
    fun hashBytes(): ByteArray = hashBytes.copyOf()

    companion object {
        /**
         * Parses and validates [bytes] (every check of SPEC §6.3).
         *
         * @throws WireException with [WireCode.DESCRIPTOR] on any failure.
         */
        @JvmStatic
        fun load(bytes: ByteArray): Descriptor = Loader(bytes).load()
    }
}

private class Loader(private val b: ByteArray) {
    private var p = 0
    private var count = 0L
    private var names = 0L

    private fun fail(detail: String): Nothing = throw WireException(WireCode.DESCRIPTOR, detail)

    private fun u8(): Int {
        if (p >= b.size) fail("unexpected end of descriptor")
        return b[p++].toInt() and 0xff
    }

    /** Descriptor varint: at most 5 bytes, canonical, at most 2^32 - 1. */
    private fun varint(): Long {
        var v = 0L
        var shift = 0
        var i = 0
        var byte: Int
        while (true) {
            byte = u8()
            v = v or ((byte and 0x7f).toLong() shl shift)
            i++
            if (byte < 0x80) break
            if (i == 5) fail("varint longer than 5 bytes")
            shift += 7
        }
        if (i > 1 && byte == 0) fail("varint has a redundant final zero byte")
        if (v > MAX_U32) fail("varint exceeds 2^32-1")
        return v
    }

    private fun node(depth: Int, inObject: Boolean): Node {
        if (depth > MAX_DEPTH) fail("nesting deeper than $MAX_DEPTH")
        val opByte = u8()
        val optional = (opByte and OPTIONAL) != 0
        val op = opByte and 0x7f
        if (optional && !inObject) fail("optional flag on a node that is not an object field")
        count++
        val n = Node(op, optional)
        when (op) {
            OP_BOOL, OP_U8, OP_I8, OP_U16, OP_I16, OP_U32, OP_I32, OP_U64, OP_I64,
            OP_F32, OP_F64, OP_STRING, OP_BYTES -> {}
            OP_ENUM -> {
                n.n = u8() + 1
                n.bits = bitLength(n.n - 1)
                n.zero = n.n == 1
                n.name0 = names.toInt()
                names += n.n
            }
            OP_OBJECT -> {
                val nf = varint()
                if (nf > MAX_FIELDS) fail("more than $MAX_FIELDS fields")
                val size = varint()
                n.name0 = names.toInt()
                names += nf
                val start = p
                var zero = true
                n.fields = Array(nf.toInt()) {
                    val f = node(depth + 1, true)
                    if (f.optional || !f.zero) zero = false
                    f
                }
                n.zero = zero
                if ((p - start).toLong() != size) fail("object size does not match its fields")
            }
            OP_ARRAY -> {
                val size = varint()
                val start = p
                val item = node(depth + 1, false)
                if (item.zero) fail("array items that encode to nothing")
                n.item = item
                if ((p - start).toLong() != size) fail("array size does not match its item")
            }
            OP_UNION -> {
                n.n = u8() + 1
                n.bits = bitLength(n.n - 1)
                val size = varint()
                n.name0 = names.toInt()
                names += 1 + n.n
                val start = p
                n.branches = Array(n.n) {
                    val br = node(depth + 1, false)
                    if (br.op != OP_OBJECT) fail("union branch is not an object")
                    br.branch = true
                    br
                }
                n.zero = n.n == 1 && n.branches[0].zero
                if ((p - start).toLong() != size) fail("union size does not match its branches")
            }
            else -> fail("unknown op $opByte")
        }
        // Each name takes at least one byte, so a count above the bytes left
        // can never be satisfied; refusing here also bounds name allocation.
        if (names > b.size) fail("more names than the descriptor can hold")
        return n
    }

    fun load(): Descriptor {
        val version = u8()
        if (version != FORMAT_VERSION) fail("unsupported format version $version")
        if (b.size - p < 8) fail("unexpected end of descriptor")
        val hash = b.copyOfRange(p, p + 8)
        p += 8
        val nodeCount = varint()
        val root = node(1, false)
        if (count != nodeCount) fail("node count does not match the nodes")
        if (names > b.size - p) fail("unexpected end of descriptor")
        val table = Array(names.toInt()) {
            val len = varint()
            if (len > b.size - p) fail("unexpected end of descriptor")
            val s = utf8Decode(b, p, len.toInt()) ?: fail("invalid UTF-8 in names")
            p += len.toInt()
            s
        }
        if (p != b.size) fail("trailing bytes after the names")
        attach(root, table)
        return Descriptor(version, hash, nodeCount, root, b.size)
    }

    private fun lookup(list: Array<String>, what: String): HashMap<String, Int> {
        val m = HashMap<String, Int>(list.size * 2)
        for (i in list.indices) {
            if (m.put(list[i], i) != null) fail("duplicate $what \"${list[i]}\"")
        }
        return m
    }

    /** SPEC §6.3 rule 11: an object's field names (and its union's tag) are distinct, none is __proto__. */
    private fun checkFieldNames(obj: Node, tag: String?) {
        val seen = HashSet<String>(obj.fieldNames.size * 2 + 2)
        for (name in obj.fieldNames) {
            if (!seen.add(name)) fail("duplicate field name \"$name\"")
        }
        if (tag != null && !seen.add(tag)) fail("duplicate field name \"$tag\"")
        if ("__proto__" in seen) fail("field name \"__proto__\" is not allowed")
    }

    private fun attach(n: Node, table: Array<String>) {
        when (n.op) {
            OP_ENUM -> {
                n.values = Array(n.n) { table[n.name0 + it] }
                n.index = lookup(n.values, "enum value")
            }
            OP_OBJECT -> {
                attachFields(n, table)
                if (!n.branch) checkFieldNames(n, null)
            }
            OP_ARRAY -> attach(n.item!!, table)
            OP_UNION -> {
                n.tag = table[n.name0]
                n.values = Array(n.n) { table[n.name0 + 1 + it] }
                n.index = lookup(n.values, "union tag value")
                for (br in n.branches) {
                    attachFields(br, table)
                    checkFieldNames(br, n.tag)
                }
            }
        }
    }

    private fun attachFields(n: Node, table: Array<String>) {
        n.fieldNames = Array(n.fields.size) { table[n.name0 + it] }
        for (f in n.fields) attach(f, table)
    }
}

/** Bit length of 0 <= x <= 255. */
private fun bitLength(x: Int): Int = 32 - Integer.numberOfLeadingZeros(x)

/**
 * Strict UTF-8 (no overlong forms, surrogates, values above U+10FFFF or bad
 * continuation bytes) to a String, or null when invalid.
 */
internal fun utf8Decode(b: ByteArray, start: Int, len: Int): String? {
    val end = start + len
    var i = start
    while (i < end && b[i] >= 0) i++
    if (i == end) return String(b, start, len, Charsets.ISO_8859_1) // all ASCII
    val out = CharArray(len)
    var k = 0
    for (j in start until i) out[k++] = b[j].toInt().toChar()
    var p = i
    while (p < end) {
        val c = b[p++].toInt() and 0xff
        if (c < 0x80) {
            out[k++] = c.toChar()
            continue
        }
        val need: Int
        val min: Int
        var cp: Int
        when {
            c in 0xc2..0xdf -> { need = 1; min = 0x80; cp = c and 0x1f }
            c in 0xe0..0xef -> { need = 2; min = 0x800; cp = c and 0x0f }
            c in 0xf0..0xf4 -> { need = 3; min = 0x10000; cp = c and 0x07 }
            else -> return null
        }
        if (p + need > end) return null
        for (x in 0 until need) {
            val cc = b[p++].toInt() and 0xff
            if ((cc and 0xc0) != 0x80) return null
            cp = (cp shl 6) or (cc and 0x3f)
        }
        if (cp < min || cp > 0x10ffff || (cp in 0xd800..0xdfff)) return null
        if (cp >= 0x10000) {
            out[k++] = Character.highSurrogate(cp)
            out[k++] = Character.lowSurrogate(cp)
        } else {
            out[k++] = cp.toChar()
        }
    }
    return String(out, 0, k)
}
