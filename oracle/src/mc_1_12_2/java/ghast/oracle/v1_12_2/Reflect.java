package ghast.oracle.v1_12_2;

import java.lang.reflect.Field;
import java.util.HashMap;
import java.util.Map;

/**
 * Reflection helpers. Field names are the Yarn (or raw intermediary) names of the remapped jar, so they
 * resolve at run time exactly as written here.
 */
final class Reflect {
    /** {@code sun.misc.Unsafe}, looked up reflectively (jdk.unsupported) to allocate without constructors. */
    private static final Object UNSAFE;
    private static final java.lang.reflect.Method ALLOCATE;
    private static final Map<String, Field> FIELDS = new HashMap<>();

    static {
        try {
            Class<?> unsafe = Class.forName("sun.misc.Unsafe");
            Field f = unsafe.getDeclaredField("theUnsafe");
            f.setAccessible(true);
            UNSAFE = f.get(null);
            ALLOCATE = unsafe.getMethod("allocateInstance", Class.class);
        } catch (ReflectiveOperationException e) {
            throw new ExceptionInInitializerError(e);
        }
    }

    private Reflect() {}

    /** Allocates an instance without running any constructor (fields keep JVM defaults). */
    @SuppressWarnings("unchecked")
    static <T> T allocate(Class<T> type) {
        try {
            return (T) ALLOCATE.invoke(UNSAFE, type);
        } catch (ReflectiveOperationException e) {
            throw new IllegalStateException(e);
        }
    }

    static Field field(Class<?> owner, String name) {
        String key = owner.getName() + '#' + name;
        Field f = FIELDS.get(key);
        if (f == null) {
            try {
                f = owner.getDeclaredField(name);
            } catch (NoSuchFieldException e) {
                throw new IllegalStateException("no field " + key, e);
            }
            f.setAccessible(true);
            FIELDS.put(key, f);
        }
        return f;
    }

    static Object get(Class<?> owner, Object target, String name) {
        try {
            return field(owner, name).get(target);
        } catch (IllegalAccessException e) {
            throw new IllegalStateException(e);
        }
    }

    static boolean getBoolean(Class<?> owner, Object target, String name) {
        try {
            return field(owner, name).getBoolean(target);
        } catch (IllegalAccessException e) {
            throw new IllegalStateException(e);
        }
    }

    static int getInt(Class<?> owner, Object target, String name) {
        try {
            return field(owner, name).getInt(target);
        } catch (IllegalAccessException e) {
            throw new IllegalStateException(e);
        }
    }

    static float getFloat(Class<?> owner, Object target, String name) {
        try {
            return field(owner, name).getFloat(target);
        } catch (IllegalAccessException e) {
            throw new IllegalStateException(e);
        }
    }

    static double getDouble(Class<?> owner, Object target, String name) {
        try {
            return field(owner, name).getDouble(target);
        } catch (IllegalAccessException e) {
            throw new IllegalStateException(e);
        }
    }

    static void set(Class<?> owner, Object target, String name, Object value) {
        try {
            field(owner, name).set(target, value);
        } catch (IllegalAccessException e) {
            throw new IllegalStateException(e);
        }
    }

    static void setDouble(Class<?> owner, Object target, String name, double value) {
        try {
            field(owner, name).setDouble(target, value);
        } catch (IllegalAccessException e) {
            throw new IllegalStateException(e);
        }
    }

    /** Calls a no-argument method by its Yarn / intermediary name. */
    static Object invoke(Class<?> owner, Object target, String name) {
        try {
            java.lang.reflect.Method m = owner.getDeclaredMethod(name);
            m.setAccessible(true);
            return m.invoke(target);
        } catch (ReflectiveOperationException e) {
            throw new IllegalStateException(e);
        }
    }

    static void setInt(Class<?> owner, Object target, String name, int value) {
        try {
            field(owner, name).setInt(target, value);
        } catch (IllegalAccessException e) {
            throw new IllegalStateException(e);
        }
    }
}
