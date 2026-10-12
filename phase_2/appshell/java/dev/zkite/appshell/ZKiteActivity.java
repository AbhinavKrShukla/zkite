package dev.zkite.appshell;

import android.app.Activity;
import android.graphics.Insets;
import android.os.Build;
import android.os.Bundle;
import android.os.Handler;
import android.os.Looper;
import android.os.Process;
import android.os.SystemClock;
import android.util.Log;
import android.view.View;
import android.view.WindowInsets;
import android.webkit.ConsoleMessage;
import android.webkit.JavascriptInterface;
import android.webkit.WebChromeClient;
import android.webkit.WebResourceRequest;
import android.webkit.WebResourceResponse;
import android.webkit.WebView;
import android.webkit.WebViewClient;

import org.json.JSONObject;

import java.io.File;
import java.io.FileInputStream;
import java.io.FileOutputStream;
import java.io.IOException;
import java.io.InputStream;
import java.io.OutputStream;

/**
 * The release App Shell — {@code zkite build apk}'s output boots straight into this.
 * See ../../PLAN.md §1.2: byte-for-byte the same boot sequence as
 * {@code phase_1/devshell}'s {@code ZKiteActivity}, minus the reload-socket thread and
 * the {@code pollHtml} UI-thread timer, plus one extra one-time asset unpack
 * ({@code app.zip} — the specific app's own code, baked in at build time instead of
 * pushed over {@code adb reverse}).
 *
 * <p>Plain {@code android.app.Activity}, not {@code AppCompatActivity} — same
 * reasoning as the Dev Shell (see that class's doc comment): {@code WebView} handles
 * IME/insets/back-gesture itself, independent of what the hosting Activity extends.
 *
 * <p>No lambdas / method references, for the same {@code javac -source 8
 * -bootclasspath android.jar} reason documented on the Dev Shell's class doc.
 */
public class ZKiteActivity extends Activity {
    static final String TAG = "ZKiteAppShell";
    static final String STDLIB_ASSET = "python-stdlib.zip";
    static final String ZKITE_PKG_ASSET = "zkite-framework.zip";
    static final String APP_ASSET = "app.zip";
    /** The asset path the page is loaded from — see {@link #serveAppFile}. */
    static final String WEBRUNTIME_PREFIX = "/android_asset/webruntime/";
    static final String STAMP_VERSION = "1"; // bump to force re-unpack on next launch

    private WebView webView;
    private final Handler mainHandler = new Handler(Looper.getMainLooper());

    private volatile boolean webViewReady = false;
    private volatile String pendingInitialHtml = null;
    private boolean initialApplied = false;

    /**
     * The Activity a pushed frame should land in, or {@code null} between destroy and
     * the next create (phase_3/PLAN.md §34.3).
     *
     * <p>Static because the caller is Rust, on a Python worker thread, and has no
     * instance to call through — {@code onFrameFromPython} is the one entry point in
     * this class that is not reached from Java. Cleared in {@code onDestroy} so a
     * background thread that outlives the Activity drops its frame instead of
     * touching a dead {@code WebView}.
     */
    private static volatile ZKiteActivity liveInstance;

    private final Runnable maybeApplyInitialHtmlRunnable = new Runnable() {
        @Override
        public void run() {
            maybeApplyInitialHtml();
        }
    };

    @Override
    protected void onCreate(Bundle savedInstanceState) {
        super.onCreate(savedInstanceState);
        liveInstance = this;

        webView = new WebView(this);
        webView.getSettings().setJavaScriptEnabled(true);
        webView.getSettings().setDomStorageEnabled(true);
        webView.addJavascriptInterface(new Bridge(), "ZKiteNative");
        webView.setWebViewClient(new WebViewClient() {
            @Override
            public void onPageFinished(WebView view, String url) {
                Log.i(TAG, "webview finished loading " + url);
            }

            @Override
            public WebResourceResponse shouldInterceptRequest(WebView view, WebResourceRequest request) {
                WebResourceResponse r = serveAppFile(request.getUrl());
                return r != null ? r : super.shouldInterceptRequest(view, request);
            }
        });
        webView.setWebChromeClient(new WebChromeClient() {
            @Override
            public boolean onConsoleMessage(ConsoleMessage cm) {
                forwardConsoleMessage(cm);
                return true;
            }
        });
        applyInsetsAsPadding(webView);
        setContentView(webView);
        registerBackHandler();
        webView.loadUrl("file:///android_asset/webruntime/index.html");

        new Thread(new Runnable() {
            @Override
            public void run() {
                bootPythonInBackground();
            }
        }, "zkite-boot").start();
    }

    /**
     * Stops routing pushed frames here (phase_3/PLAN.md §34.3).
     *
     * <p>Guarded on identity rather than cleared unconditionally: a configuration
     * change destroys this Activity *after* creating the next one, so an unguarded
     * clear would blank the field the new instance just claimed and silently kill the
     * push path for the rest of the process.
     */
    @Override
    protected void onDestroy() {
        if (liveInstance == this) {
            liveInstance = null;
        }
        super.onDestroy();
    }

    private void applyInsetsAsPadding(WebView v) {
        v.setOnApplyWindowInsetsListener(new View.OnApplyWindowInsetsListener() {
            @Override
            public WindowInsets onApplyWindowInsets(View view, WindowInsets insets) {
                int top, bottom, left, right;
                if (Build.VERSION.SDK_INT >= 30) {
                    Insets bars = insets.getInsets(WindowInsets.Type.systemBars());
                    top = bars.top; bottom = bars.bottom; left = bars.left; right = bars.right;
                } else {
                    top = insets.getSystemWindowInsetTop();
                    bottom = insets.getSystemWindowInsetBottom();
                    left = insets.getSystemWindowInsetLeft();
                    right = insets.getSystemWindowInsetRight();
                }
                view.setPadding(left, top, right, bottom);
                return insets;
            }
        });
    }

    // ---------------------------------------------------------------- boot sequence

    /**
     * Serves a file out of the app's own unpacked directory to the WebView, so an
     * {@code Image(src="logo.png")} resolves on this backend (phase_3/PLAN.md §35.3).
     *
     * <p><b>Why this is needed at all.</b> The page is loaded from
     * {@code file:///android_asset/webruntime/index.html}, so a relative
     * {@code <img src="logo.png">} resolves to
     * {@code file:///android_asset/webruntime/logo.png} — an asset that does not
     * exist, because the app's own files are unpacked to {@code getFilesDir()/app}
     * instead. Every image was therefore broken on this backend too; §35 found it
     * while checking whether native was diverging and discovered neither backend
     * worked.
     *
     * <p><b>Why interception rather than {@code setAllowFileAccessFromFileURLs}.</b>
     * That flag defaults off on API 30+ and turning it on would let *any* script on
     * the page read *any* file the app can — a real widening of the attack surface to
     * fix a path-resolution problem. Interception gives the WebView exactly the files
     * the app shipped and nothing else, needs no setting changed, and behaves the same
     * on every API level.
     *
     * <p><b>Confinement</b> matches {@code zkite_imagestore}'s on the native side, and
     * matches it *deliberately*: a {@code src} that escapes the app directory is
     * refused on both backends rather than being a native-only rule. Checked with
     * {@link File#getCanonicalPath} — which, unlike the native side's lexical check,
     * also resolves symlinks.
     *
     * @return the response, or {@code null} to let the WebView handle the URL itself.
     */
    private WebResourceResponse serveAppFile(android.net.Uri uri) {
        if (uri == null || !"file".equals(uri.getScheme())) {
            return null;
        }
        String path = uri.getPath();
        if (path == null || !path.startsWith(WEBRUNTIME_PREFIX)) {
            return null;
        }
        String rel = path.substring(WEBRUNTIME_PREFIX.length());
        // index.html and the runtime's own JS/CSS/fonts stay with the assets; only
        // names the asset directory does not have fall through to the app directory.
        // Checked by *asking* the asset manager rather than by listing extensions, so
        // this cannot drift as the runtime grows files.
        InputStream asset = null;
        try {
            asset = getAssets().open("webruntime/" + rel);
        } catch (IOException notAnAsset) {
            // Expected for an app's own image — fall through.
        }
        if (asset != null) {
            try {
                asset.close();
            } catch (IOException ignored) {
            }
            return null;
        }
        try {
            File appDir = new File(getFilesDir(), "app");
            File target = new File(appDir, rel);
            if (!target.getCanonicalPath().startsWith(appDir.getCanonicalPath() + File.separator)
                    || !target.isFile()) {
                return null;
            }
            return new WebResourceResponse(guessMimeType(rel), null, new FileInputStream(target));
        } catch (IOException e) {
            Log.w(TAG, "could not serve " + rel + " from the app directory", e);
            return null;
        }
    }

    /**
     * Enough of a MIME table for the formats {@code zkite build} actually stages
     * (see {@code build_cmd.rs}'s {@code ASSET_EXTENSIONS}).
     *
     * <p>{@code null} rather than a guess for anything else: a {@code WebResourceResponse}
     * with a wrong MIME type fails in ways that do not name the MIME type, and letting
     * the WebView sniff is better than asserting something false.
     */
    private static String guessMimeType(String name) {
        String lower = name.toLowerCase(java.util.Locale.ROOT);
        if (lower.endsWith(".png")) {
            return "image/png";
        }
        if (lower.endsWith(".jpg") || lower.endsWith(".jpeg")) {
            return "image/jpeg";
        }
        return null;
    }

    private void bootPythonInBackground() {
        String html;
        try {
            System.loadLibrary("python3.14");
            System.loadLibrary("zkiteappshell");

            File pythonPrefix = new File(getFilesDir(), "python");
            File zkitePkgDir = new File(getFilesDir(), "zkite_framework");
            File appDir = new File(getFilesDir(), "app");

            unpackZipAssetOnce(STDLIB_ASSET, pythonPrefix, "stdlib-" + STAMP_VERSION);
            linkExtensionModules(pythonPrefix);
            unpackZipAssetOnce(ZKITE_PKG_ASSET, zkitePkgDir, "zkitepkg-" + STAMP_VERSION);
            unpackZipAssetOnce(APP_ASSET, appDir, "app-" + STAMP_VERSION);

            // The platform half of the cold-start measurement, logged at the same
            // point the native harness logs its own (phase_3/PLAN.md §12.2 item 1):
            // process start → here is zygote fork, Activity creation and asset
            // unpack, i.e. everything both backends pay identically. What follows
            // differs — see `first-paint`, which JS reports and
            // `forwardConsoleMessage` completes.
            Log.i(TAG, "ZKITEPERF boot-call process_rel="
                    + (SystemClock.elapsedRealtime() - Process.getStartElapsedRealtime()) + "ms");

            html = Native.boot(pythonPrefix.getAbsolutePath(), zkitePkgDir.getAbsolutePath(),
                    appDir.getAbsolutePath());
        } catch (Throwable t) {
            Log.e(TAG, "boot failed", t);
            html = "<div id=\"r\" class=\"flex items-center justify-center bg-red-950 text-red-200 p-6 font-mono text-sm\">boot failed: "
                    + t + "</div>";
        }
        pendingInitialHtml = html;
        mainHandler.post(maybeApplyInitialHtmlRunnable);
    }

    /**
     * Routes Back to {@code zkite_py}'s navigation stack (phase_3/PLAN.md §16).
     *
     * <p>Two mechanisms because the platform has two: {@code OnBackInvokedCallback}
     * on API 33+ (which the generated manifest's {@code
     * enableOnBackInvokedCallback="true"} opts into) and {@code onBackPressed()}
     * below it. Exactly one of them fires for any given press, so there is no
     * double-pop to guard against.
     */
    private void registerBackHandler() {
        if (Build.VERSION.SDK_INT >= 33) {
            getOnBackInvokedDispatcher().registerOnBackInvokedCallback(
                    android.window.OnBackInvokedDispatcher.PRIORITY_DEFAULT,
                    new android.window.OnBackInvokedCallback() {
                        @Override
                        public void onBackInvoked() {
                            handleBackPress();
                        }
                    });
        }
    }

    /**
     * Pre-API-33 Back. Deliberately never calls {@code super.onBackPressed()} — see
     * {@link #handleBackPress()}, which owns the exit decision for both paths.
     */
    @Override
    public void onBackPressed() {
        handleBackPress();
    }

    /**
     * Ask {@code zkite_py} whether anything wanted the press; finish the Activity if
     * not.
     *
     * <p><b>Off the UI thread deliberately</b>, even though {@code Native.handleBack}
     * is synchronous and could be called directly from here. Running {@code view()}
     * on the UI thread would put an app's own Python on the critical path of a
     * gesture, and this shell already established the pattern of keeping Python off
     * it ({@code bootPythonInBackground}). It also makes the two backends behave
     * identically: the native renderer answers this asynchronously because it must
     * (the answer lives behind the render thread), and matching that here means Back
     * has one observable behaviour across both rather than two similar ones.
     */
    private void handleBackPress() {
        new Thread(new Runnable() {
            @Override
            public void run() {
                final String html = Native.handleBack();
                mainHandler.post(new Runnable() {
                    @Override
                    public void run() {
                        if (html != null) {
                            applyHtml(html);
                        } else {
                            // Nothing in the app wanted it: the platform default,
                            // which is what keeps Back exiting a one-screen app
                            // exactly as it did before navigation existed.
                            finish();
                        }
                    }
                });
            }
        }, "zkite-back").start();
    }

    private void maybeApplyInitialHtml() {
        if (initialApplied || !webViewReady || pendingInitialHtml == null) {
            return;
        }
        initialApplied = true;
        applyHtml(pendingInitialHtml);
    }

    /**
     * A frame the app produced on its own — called from Rust (../src/lib.rs's frame
     * sink), on whatever Python thread ran the {@code @store.update()} that made it.
     * See phase_3/PLAN.md §34.3.
     *
     * <p>This is the WebView backend's answer to "nothing can repaint without a
     * touch". Every other path into {@code applyHtml} starts from a Java call that
     * returned HTML ({@code boot}, {@code dispatch}, {@code handleBack}); this one has
     * no such call to return through, which is exactly the asymmetry that made the
     * native backend's push path nearly free and this one real work.
     *
     * <p><b>Not a poll.</b> The Dev Shell parks HTML in a mutex and polls it on a
     * 120ms timer, a trade phase_1/PLAN.md §4 documents as deliberately avoiding
     * cross-thread JNI. A shipped app cannot pay that: a UI-thread timer that runs
     * forever costs battery on every app whether or not it ever pushes a frame. So
     * this is a genuine cross-thread call, and the app is completely idle between
     * frames.
     *
     * <p>Static, and defensive about everything it touches: it can arrive before the
     * WebView has finished loading (held as {@code pendingInitialHtml}, which
     * {@code maybeApplyInitialHtml} drains on ready) or after the Activity is gone
     * (dropped).
     */
    static void onFrameFromPython(final String html) {
        final ZKiteActivity a = liveInstance;
        if (a == null || html == null) {
            return;
        }
        a.mainHandler.post(new Runnable() {
            @Override
            public void run() {
                if (!a.webViewReady) {
                    // Newest frame wins — an older one that never reached the DOM has
                    // nothing to contribute.
                    a.pendingInitialHtml = html;
                    return;
                }
                a.applyHtml(html);
            }
        });
    }

    /** Must run on the UI thread — evaluateJavascript requires it. */
    private void applyHtml(String html) {
        String call = "ZKiteRuntime.applyHtml(" + JSONObject.quote(html) + ")";
        webView.evaluateJavascript(call, null);
    }

    private class Bridge {
        @JavascriptInterface
        public void dispatch(final String id, final String event, final String payload) {
            final String html = Native.dispatch(id, event, payload);
            if (html != null) {
                mainHandler.post(new Runnable() {
                    @Override
                    public void run() {
                        applyHtml(html);
                    }
                });
            }
        }

        @JavascriptInterface
        public void ready() {
            Log.i(TAG, "ZKiteRuntime.ready()");
            webViewReady = true;
            mainHandler.post(maybeApplyInitialHtmlRunnable);
        }
    }

    // ------------------------------------------------------- instrumentation (§12.1)

    /** Prefix `zkite-runtime.js` puts on every instrumentation line. */
    private static final String PERF_TAG = "ZKITEPERF";

    /**
     * Forwards the WebView's console to logcat, which is what makes
     * {@code zkite-runtime.js}'s §12.1 instrumentation observable at all — and, not
     * incidentally, makes a JS error in the runtime visible for the first time
     * (previously it went nowhere on a device).
     *
     * <p>Chosen over a new {@code ZKiteNative} bridge method for the same reason
     * phase_3/PLAN.md §11.3 reported scroll position through the existing
     * {@code dispatch} bridge: no new bridge surface to keep in sync across two
     * shells, and no JNI change.
     *
     * <p>The cold-start number is completed here rather than in JS because only Java
     * can see {@link Process#getStartElapsedRealtime()}. JS reports its own
     * page-relative time; this adds the process-relative one, so the two clock
     * domains are never reconciled against each other — each is measured where it is
     * actually known.
     */
    private void forwardConsoleMessage(ConsoleMessage cm) {
        String text = cm.message();
        if (text.startsWith(PERF_TAG)) {
            if (text.startsWith(PERF_TAG + " first-paint")) {
                long sinceProcessStart = SystemClock.elapsedRealtime() - Process.getStartElapsedRealtime();
                text = text + " process_rel=" + sinceProcessStart + "ms";
            }
            Log.i(TAG, text);
            return;
        }
        Log.i(TAG, "js[" + cm.messageLevel() + "] " + text + " (" + cm.sourceId() + ":" + cm.lineNumber() + ")");
    }

    // ------------------------------------------------------------- asset unpacking
    // Identical to phase_1/devshell's ZKiteActivity.unpackZipAssetOnce, generalized to
    // any zip asset — app.zip reuses it too, not just the stdlib and framework.

    private void unpackZipAssetOnce(String assetName, File destDir, String stampName) throws IOException {
        long t0 = System.nanoTime();
        if (dev.zkite.runtime.ZipAssets.unpack(new File(getApplicationInfo().sourceDir),
                assetName, destDir, stampName)) {
            Log.i(TAG, "unpacked " + assetName + " in "
                    + ((System.nanoTime() - t0) / 1_000_000) + " ms");
        }
    }

    /** Identical rationale to phase_0/spike2 and phase_1/devshell: extension modules
     * must dlopen from the app's own native lib dir (Android 10+), so they're staged
     * into lib/&lt;abi&gt;/ at build time and pointed to via a .pth file `site` picks
     * up at interpreter startup. */
    private void linkExtensionModules(File pythonPrefix) throws IOException {
        File sitePackages = new File(pythonPrefix, "lib/python3.14/site-packages");
        if (!sitePackages.isDirectory() && !sitePackages.mkdirs()) {
            throw new IOException("could not create " + sitePackages);
        }
        String nativeLibDir = getApplicationInfo().nativeLibraryDir;
        File pth = new File(sitePackages, "zkite-dynload.pth");
        OutputStream os = new FileOutputStream(pth);
        try {
            os.write((nativeLibDir + "\n").getBytes("UTF-8"));
        } finally {
            os.close();
        }
    }


}
