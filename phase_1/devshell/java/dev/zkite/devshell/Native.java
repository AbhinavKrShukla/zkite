package dev.zkite.devshell;

/**
 * The JNI surface of the Dev Shell. Every method is a thin call into Rust
 * (../src/lib.rs), which in turn calls into the already-built zkite_py runtime
 * (../src/runtime.py) — see ../../PLAN.md M4.
 */
public final class Native {
    private Native() {}

    /**
     * Boots the embedded interpreter (once per process) and renders the app currently
     * on disk at {@code appDir/main.py}, if any.
     *
     * @param pythonPrefix directory containing {@code lib/python3.14/}
     * @param zkitePkgDir   directory containing the {@code zkite/} framework package
     * @param appDir       directory containing (or awaiting) the user's {@code main.py}
     * @return an HTML string — either the rendered app, an error overlay, or a
     *         "waiting for zkite dev" placeholder if no app has been pushed yet
     */
    public static native String boot(String pythonPrefix, String zkitePkgDir, String appDir);

    /**
     * Delivers a DOM event dispatched from the WebView bridge (zkite-runtime.js) to the
     * Python widget tree.
     *
     * @return the newly rendered HTML if the event was handled, or {@code null} if no
     *         handler was registered for (id, event) — the WebView then leaves the DOM
     *         alone rather than morphing in a no-op
     */
    public static native String dispatch(String id, String event, String payload);

    /**
     * Starts the background thread that dials {@code host:port} (reached via
     * {@code adb reverse}) and applies pushed files / reload signals from
     * {@code zkite dev}. Returns immediately; call once per process.
     */
    public static native void startReloadServer(String host, int port, String appDir);

    /**
     * Non-blocking: returns HTML produced by a file-push-triggered reload since the
     * last call, or {@code null} if nothing new has landed. Polled from the UI thread
     * on a short timer rather than pushed via an async JNI callback — see the design
     * note at the top of ../src/lib.rs for why.
     */
    public static native String pollHtml();

    /**
     * Offers a platform Back press to {@code zkite_py}'s navigation stack
     * (phase_3/PLAN.md §16).
     *
     * @return the newly rendered HTML if navigation consumed the press, or {@code
     *         null} if nothing wanted it — in which case the caller must finish the
     *         Activity, which is what keeps Back exiting a one-screen app as it
     *         always has
     */
    public static native String handleBack();
}
