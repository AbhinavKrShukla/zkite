package dev.zkite.appshell;

/**
 * The JNI surface of the release App Shell. Every method is a thin call into Rust
 * (../src/lib.rs), which in turn calls into the already-built zkite_py runtime
 * (phase_1/devshell/src/runtime.py, reused unchanged — see ../../PLAN.md §1.2).
 *
 * <p>Deliberately smaller than the Dev Shell's {@code Native}: no {@code pollHtml}/
 * {@code startReloadServer} — a shipped app has no {@code zkite dev} to connect to.
 */
public final class Native {
    private Native() {}

    /**
     * Boots the embedded interpreter (once per process) and renders the app baked
     * into {@code appDir/main.py} at build time.
     *
     * @param pythonPrefix directory containing {@code lib/python3.14/}
     * @param zkitePkgDir   directory containing the {@code zkite/} framework package
     * @param appDir       directory containing this app's {@code main.py} (unpacked
     *                     once from the APK's {@code assets/app.zip}, never pushed
     *                     over a socket — there is no dev loop in a release build)
     * @return an HTML string — the rendered app, or an error overlay if {@code
     *         main.py} itself failed to import
     */
    public static native String boot(String pythonPrefix, String zkitePkgDir, String appDir);

    /**
     * Delivers a DOM event dispatched from the WebView bridge (zkite-runtime.js) to
     * the Python widget tree.
     *
     * @return the newly rendered HTML if the event was handled, or {@code null} if no
     *         handler was registered for (id, event) — the WebView then leaves the DOM
     *         alone rather than morphing in a no-op
     */
    public static native String dispatch(String id, String event, String payload);

    /**
     * Offers a platform Back press to {@code zkite_py}'s navigation stack
     * (phase_3/PLAN.md §16).
     *
     * <p>Synchronous, unlike the native renderer's twin: on this backend Python is
     * already called on the UI thread for every tap, so there is no render thread to
     * ask and no reason to defer. The caller can therefore act on the answer
     * directly.
     *
     * @return the newly rendered HTML if navigation consumed the press, or {@code
     *         null} if nothing wanted it — in which case the caller must do the
     *         platform default and finish the Activity, which is what keeps Back
     *         exiting a one-screen app exactly as it always has
     */
    public static native String handleBack();
}
