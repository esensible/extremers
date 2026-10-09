package bitwire

/**
 * The refusal codes of SPEC §7. [wire] is the code as written in the spec and
 * the test vectors.
 */
enum class WireCode(val wire: String) {
    /** load: the descriptor fails SPEC §6.3. */
    DESCRIPTOR("descriptor"),

    /** decode: the message ends before the value does. */
    TRUNCATED("truncated"),

    /** decode: bytes after the end of the value. */
    TRAILING("trailing"),

    /** decode: malformed wire data (bad varint, index >= n, stray bits, bad UTF-8, ...). */
    INVALID("invalid"),

    /**
     * decode, hosts without 64-bit integers only. Never raised by this
     * implementation: every wire integer has an exact Kotlin value.
     */
    PRECISION("precision"),

    /** encode: the value has the wrong type for its node, or a required property is missing. */
    SHAPE("shape"),

    /** encode: right type, but cannot be written (integer not whole or out of width, unknown enum value or tag). */
    RANGE("range");

    override fun toString(): String = wire
}

/** Every refusal of [Descriptor.load], [Wire.decode] and [Wire.encode]. */
class WireException(val code: WireCode, detail: String) : RuntimeException("${code.wire}: $detail")
