package ghast.oracle;

/**
 * Minimal deterministic JSON writer for golden vectors.
 *
 * <p>Floating point values are written bit-exact as a two element array {@code [decimal, "hex"]}:
 * the decimal is Java's shortest round-trip representation (JDK 19+), the hex string is the raw IEEE
 * bits ({@code Double.doubleToRawLongBits} as 16 hex digits, {@code Float.floatToRawIntBits} as 8). The
 * hex is authoritative; the decimal is for humans and is {@code null} for NaN / infinities.
 */
public final class Json {
    private final StringBuilder sb = new StringBuilder(256);
    /** Whether a separator is needed before the next element at the current nesting level. */
    private boolean needComma;

    public Json beginObject() {
        sep();
        sb.append('{');
        needComma = false;
        return this;
    }

    public Json endObject() {
        sb.append('}');
        needComma = true;
        return this;
    }

    public Json beginArray() {
        sep();
        sb.append('[');
        needComma = false;
        return this;
    }

    public Json endArray() {
        sb.append(']');
        needComma = true;
        return this;
    }

    public Json key(String k) {
        sep();
        string(k);
        sb.append(':');
        needComma = false;
        return this;
    }

    public Json value(String s) {
        sep();
        if (s == null) {
            sb.append("null");
        } else {
            string(s);
        }
        needComma = true;
        return this;
    }

    public Json value(boolean b) {
        sep();
        sb.append(b);
        needComma = true;
        return this;
    }

    public Json value(long v) {
        sep();
        sb.append(v);
        needComma = true;
        return this;
    }

    /** Bit-exact double: {@code [decimal, "16 hex digits"]}. */
    public Json f64(double d) {
        sep();
        sb.append('[');
        decimal(d);
        sb.append(",\"");
        hex(Double.doubleToRawLongBits(d), 16);
        sb.append("\"]");
        needComma = true;
        return this;
    }

    /** Bit-exact float: {@code [decimal, "8 hex digits"]}. */
    public Json f32(float f) {
        sep();
        sb.append('[');
        if (Float.isFinite(f)) {
            sb.append(Float.toString(f));
        } else {
            sb.append("null");
        }
        sb.append(",\"");
        hex(Float.floatToRawIntBits(f) & 0xffffffffL, 8);
        sb.append("\"]");
        needComma = true;
        return this;
    }

    public Json field(String k, String v) {
        return key(k).value(v);
    }

    public Json field(String k, boolean v) {
        return key(k).value(v);
    }

    public Json field(String k, long v) {
        return key(k).value(v);
    }

    public Json fieldF64(String k, double v) {
        return key(k).f64(v);
    }

    public Json fieldF32(String k, float v) {
        return key(k).f32(v);
    }

    /** Appends pre-rendered JSON as one element. */
    public Json raw(String json) {
        sep();
        sb.append(json);
        needComma = true;
        return this;
    }

    @Override
    public String toString() {
        return sb.toString();
    }

    private void sep() {
        if (needComma) {
            sb.append(',');
        }
    }

    private void decimal(double d) {
        if (Double.isFinite(d)) {
            sb.append(Double.toString(d));
        } else {
            sb.append("null");
        }
    }

    private void hex(long bits, int digits) {
        for (int i = digits - 1; i >= 0; i--) {
            sb.append(Character.forDigit((int) ((bits >>> (i * 4)) & 0xf), 16));
        }
    }

    private void string(String s) {
        sb.append('"');
        for (int i = 0; i < s.length(); i++) {
            char c = s.charAt(i);
            switch (c) {
                case '"' -> sb.append("\\\"");
                case '\\' -> sb.append("\\\\");
                case '\n' -> sb.append("\\n");
                case '\r' -> sb.append("\\r");
                case '\t' -> sb.append("\\t");
                default -> {
                    if (c < 0x20) {
                        sb.append(String.format("\\u%04x", (int) c));
                    } else {
                        sb.append(c);
                    }
                }
            }
        }
        sb.append('"');
    }
}
