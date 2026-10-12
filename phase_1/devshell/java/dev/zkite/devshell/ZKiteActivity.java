package dev.zkite.devshell;

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
 * The Dev Shell — plan1.md's "generic Dev Shell APK", installed once, hosting
 * whatever app {@code zkite dev} pushes into {@code filesDir/app/}. See ../../PLAN.md
 * M4.
 *
 * <p><b>Plain {@code android.app.Activity}, not {@code AppCompatActivity}</b> — a
 * deliberate change from ../../PLAN.md's original M4 sketch, worth explaining since it
 * contradicts phase_0's own README ("the shipping shim needs AppCompat for IME,
 * insets and the back gesture"). That statement is true for a *custom-rendered* text
 * field with no real {@code View} behind it (phase 3's problem — see phase_0's known
 * gaps). It is not true here: {@code WebView} is a real Android {@code View} with its
 * own internal {@code ImeAdapter}/{@code InputConnection} handling for every
 * {@code <input>} in the page, independent of what the *hosting* Activity extends.
 * Insets and back-gesture are both plain-platform APIs on modern Android (this class
 * uses {@code View.setOnApplyWindowInsetsListener} directly). Avoiding AppCompat
 * avoids the one gap phase_0's spike 2 explicitly punted on — resolving an AndroidX
 * AAR without Gradle — for zero loss on the WebView backend. Revisit only if phase 3's
 * native renderer needs it for its own, unrelated reasons.
 *
 * <p>No lambdas / method references anywhere in this file, on purpose: {@code javac
 * -source 8 -bootclasspath android.jar} (../scripts/build_apk.sh, matching phase_0's
 * own javac invocation) can't desugar them against the stub android.jar — it fails
 * with "cannot find symbol: method metafactory". Anonymous inner classes instead.
 */
public class ZKiteActivity extends Activity {
    static final String TAG = "ZKiteDevShell";
    static final String STDLIB_ASSET = "python-stdlib.zip";
    static final String ZKITE_PKG_ASSET = "zkite-framework.zip";
    /** The asset path the page is loaded from — see {@link #serveAppFile}. */
    static final String WEBRUNTIME_PREFIX = "/android_asset/webruntime/";
    static final String STAMP_VERSION = "1"; // bump to force re-unpack on next launch
    static final int RELOAD_PORT = 8765;
    static final int POLL_INTERVAL_MS = 120;

    private WebView webView;
    private final Handler mainHandler = new Handler(Looper.getMainLooper());

    private volatile boolean webViewReady = false;
    private volatile String pendingInitialHtml = null;
    private boolean initialApplied = false;

    private final Runnable pollRunnable = new Runnable() {
        @Override
        public void run() {
            String html = Native.pollHtml();
            if (html != null) {
                applyHtml(html);
            }
            mainHandler.postDelayed(this, POLL_INTERVAL_MS);
        }
    };

    private final Runnable maybeApplyInitialHtmlRunnable = new Runnable() {
        @Override
        public void run() {
            maybeApplyInitialHtml();
        }
    };

    @Override
    protected void onCreate(Bundle savedInstanceState) {
        super.onCreate(savedInstanceState);

        webView = new WebView(this);
        webView.getSettings().setJavaScriptEnabled(true);
        webView.getSettings().setDomStorageEnabled(true);
        WebView.setWebContentsDebuggingEnabled(true); // chrome://inspect during dev
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
            System.loadLibrary("zkitedevshell");

            File pythonPrefix = new File(getFilesDir(), "python");
            File zkitePkgDir = new File(getFilesDir(), "zkite_framework");
            File appDir = new File(getFilesDir(), "app");
            appDir.mkdirs();

            unpackZipAssetOnce(STDLIB_ASSET, pythonPrefix, "stdlib-" + STAMP_VERSION);
            linkExtensionModules(pythonPrefix);
            unpackZipAssetOnce(ZKITE_PKG_ASSET, zkitePkgDir, "zkitepkg-" + STAMP_VERSION);

            html = Native.boot(pythonPrefix.getAbsolutePath(), zkitePkgDir.getAbsolutePath(),
                    appDir.getAbsolutePath());

            // Only start the reload socket once the interpreter is known-initialized —
            // otherwise a reload frame could race PyGILState_Ensure against an
            // uninitialized interpreter. See ../src/lib.rs's design note.
            Native.startReloadServer("127.0.0.1", RELOAD_PORT, appDir.getAbsolutePath());
            mainHandler.post(pollRunnable);
        } catch (Throwable t) {
            Log.e(TAG, "boot failed", t);
            html = "<div id=\"r\" class=\"flex items-center justify-center bg-red-950 text-red-200 p-6 font-mono text-sm\">boot failed: "
                    + t + "</div>";
        }
        pendingInitialHtml = html;
        mainHandler.post(maybeApplyInitialHtmlRunnable);
    }

    private void maybeApplyInitialHtml() {
        if (initialApplied || !webViewReady || pendingInitialHtml == null) {
            return;
        }
        initialApplied = true;
        applyHtml(pendingInitialHtml);
    }

    /**
     * Routes Back to {@code zkite_py}'s navigation stack (phase_3/PLAN.md §16) —
     * identical to {@code appshell}'s, so the dev loop and a shipped app behave the
     * same way. Both mechanisms are wired ({@code OnBackInvokedCallback} on API 33+,
     * {@code onBackPressed()} below it); exactly one fires per press.
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

    /** Pre-API-33 Back — see {@link #handleBackPress()}, which owns the decision. */
    @Override
    public void onBackPressed() {
        handleBackPress();
    }

    /**
     * Ask {@code zkite_py} whether anything wanted the press; finish if not. Off the
     * UI thread for the same reason {@code appshell}'s twin is — see its doc comment.
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
                            finish();
                        }
                    }
                });
            }
        }, "zkite-back").start();
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
     * Forwards the WebView's console to logcat. Identical to
     * {@code phase_2/appshell}'s — see that copy's doc comment for why this is a
     * {@code WebChromeClient} rather than a new bridge method, and why the cold-start
     * anchor is completed on this side.
     *
     * <p>Duplicated rather than shared for the same reason
     * {@code linkExtensionModules} is: the two shells share {@code runtime.py} and
     * {@code webruntime/}, never Java. The JS half of this instrumentation *is*
     * shared automatically — both shells copy the whole {@code webruntime/}
     * directory — so without this the shared instrumentation would log into a void
     * here while working in the release shell.
     *
     * <p>Note this shell's numbers are not comparable to
     * {@code phase_3/gpu_harness}'s: the Dev Shell also runs a reload-socket thread
     * and a 120ms {@code pollHtml} UI-thread timer (phase_1/PLAN.md §4). The
     * backend comparison in phase_3/PLAN.md §12 uses {@code appshell}, which has
     * neither, for exactly that reason.
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
    // Same pattern as phase_0/spike2's ZKiteActivity.unpackStdlib, generalized to any
    // zip asset (zkite-framework.zip reuses it too, not just the stdlib).

    private void unpackZipAssetOnce(String assetName, File destDir, String stampName) throws IOException {
        long t0 = System.nanoTime();
        if (dev.zkite.runtime.ZipAssets.unpack(new File(getApplicationInfo().sourceDir),
                assetName, destDir, stampName)) {
            Log.i(TAG, "unpacked " + assetName + " in "
                    + ((System.nanoTime() - t0) / 1_000_000) + " ms");
        }
    }

    /** Identical rationale to phase_0/spike2: extension modules must dlopen from the
     * app's own native lib dir (Android 10+), so they're staged into lib/&lt;abi&gt;/
     * at build time and pointed to via a .pth file `site` picks up at interpreter
     * startup. */
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
