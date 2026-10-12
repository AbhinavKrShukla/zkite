package dev.zkite.phase0;

/**
 * The JNI surface of the phase-0 shim. Deliberately one method: everything else the
 * real framework needs (IME, insets, back gesture, a11y) is phase 1+ work, and adding
 * it here would make the spike prove less, not more.
 */
public final class Native {
    private Native() {}

    /**
     * Initialise the embedded CPython interpreter and run {@code print("hi")}.
     *
     * @param prefix directory containing {@code lib/python3.14/}
     * @return a single {@code ZKITE_SPIKE1_RESULT ...} line
     */
    public static native String boot(String prefix);
}
