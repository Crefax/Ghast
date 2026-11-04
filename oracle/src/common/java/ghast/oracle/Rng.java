package ghast.oracle;

/**
 * SplitMix64: a tiny, fully specified PRNG so that fuzz cases are a pure function of the seed on every
 * JVM (independent of {@code java.util.Random} or {@code SplittableRandom} implementation details).
 */
public final class Rng {
    private long state;

    public Rng(long seed) {
        this.state = seed;
    }

    public long nextLong() {
        long z = (state += 0x9E3779B97F4A7C15L);
        z = (z ^ (z >>> 30)) * 0xBF58476D1CE4E5B9L;
        z = (z ^ (z >>> 27)) * 0x94D049BB133111EBL;
        return z ^ (z >>> 31);
    }

    /** Uniform integer in {@code [0, bound)}. */
    public int nextInt(int bound) {
        if (bound <= 0) {
            throw new IllegalArgumentException("bound must be positive");
        }
        return (int) Long.remainderUnsigned(nextLong(), bound);
    }

    /** Uniform integer in {@code [lo, hi]}. */
    public int range(int lo, int hi) {
        return lo + nextInt(hi - lo + 1);
    }

    /** Uniform double in {@code [0, 1)} with 53 random bits. */
    public double nextDouble() {
        return (nextLong() >>> 11) * 0x1.0p-53;
    }

    public boolean chance(double p) {
        return nextDouble() < p;
    }

    public <T> T pick(T[] values) {
        return values[nextInt(values.length)];
    }

    public int pick(int[] values) {
        return values[nextInt(values.length)];
    }

    /** Derives an independent stream (for per-case generators). */
    public Rng fork() {
        return new Rng(nextLong());
    }
}
