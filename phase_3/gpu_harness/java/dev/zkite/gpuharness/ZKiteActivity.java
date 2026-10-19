package dev.zkite.gpuharness;

import android.app.Activity;
import android.graphics.Insets;
import android.graphics.Color;
import android.os.Build;
import android.os.Bundle;
import android.os.Process;
import android.os.SystemClock;
import android.util.Log;
import android.view.View;
import android.view.ViewConfiguration;
import android.view.ViewGroup;
import android.view.Window;
import android.view.WindowInsets;
import android.view.WindowInsetsController;
import android.widget.FrameLayout;
import org.json.JSONObject;

import java.io.File;
import java.io.FileOutputStream;
import java.io.IOException;
import java.io.InputStream;
import java.io.OutputStream;

/**
 * Phase 3's GPU render harness Activity (../../PLAN.md §4/§5). Hosts a
 * {@link ZKiteSurfaceView} instead of a {@code WebView} — everything else about this
 * class (plain {@code android.app.Activity}, insets-as-padding, no lambdas, the
 * unpack-once-behind-a-stamp-file idiom) matches
 * {@code phase_1/devshell}/{@code phase_2/appshell}'s {@code ZKiteActivity} on
 * purpose: same class name, same package shape (only {@code dev.zkite.gpuharness}
 * instead of {@code .devshell}/{@code .appshell}), same conventions, because
 * ../../PLAN.md §1's whole point is that the native renderer is a *third backend*
 * behind the same kind of Activity, not a different architecture. §5 pushed that
 * further: this Activity now unpacks the exact same three Python assets
 * {@code phase_2.appshell.ZKiteActivity} does ({@code python-stdlib.zip}/
 * {@code zkite-framework.zip}/{@code app.zip}), for the same reason — a real
 * {@code zkite_py} app, not a hand-built Rust tree, drives what gets rendered.
 *
 * <p><b>Plain {@code android.app.Activity}, not {@code AppCompatActivity}</b> — for
 * a different reason than the WebView shells' (they inherit IME/insets handling from
 * a real {@code WebView} child). This harness has no text input at all yet (IME is
 * explicitly its own not-yet-started item, ../../PLAN.md §5), so there is nothing
 * AppCompat would currently add. Revisit when IME work starts.
 *
 * <p>Assets are prepared off the UI thread. The surface is attached only after
 * Native.boot publishes its command queue, so startup cannot lose surface events.
 * A branded loading view remains above the surface until its first presented frame.
 */
public class ZKiteActivity extends Activity {
    static final String TAG = "ZKiteGpuHarness";
    // One zip of six faces (sans/serif/mono x regular/bold) rather than six separate
    // assets — see Native.boot's own doc comment and ../../PLAN.md §9.4 for why the
    // set is what it is, and stage_fonts.sh for where the files come from.
    static final String FONTS_ASSET = "fonts.zip";
    static final String STDLIB_ASSET = "python-stdlib.zip";
    static final String ZKITE_PKG_ASSET = "zkite-framework.zip";
    static final String APP_ASSET = "app.zip";
    static final String STAMP_VERSION = "2"; // bump to force re-unpack on next launch
    /** Present only in the Dev Shell build — see {@link #startReloadServerIfDevShell}. */
    static final String DEVMODE_ASSET = "zkite-devmode";
    /** Matches phase_1/cli/src/dev_cmd.rs's default and phase_1/devshell's. */
    static final int RELOAD_PORT = 8765;
    private SystemBarsView systemBarsView;
    private ZKiteSurfaceView surfaceView;
    private FrameLayout content;
    private View loadingView;
    private android.widget.TextView loadingMessage;
    private android.widget.ProgressBar loadingProgress;
    private boolean nativeBooted;
    private dev.zkite.runtime.DeviceServices deviceServices;


    @Override
    protected void onCreate(Bundle savedInstanceState) {
        super.onCreate(savedInstanceState);
        deviceServices = new dev.zkite.runtime.DeviceServices(this);

        // No title bar: this app draws every pixel itself (that's the entire point of
        // phase 3's renderer), so the platform's default ActionBar is dead screen
        // space eating into the surface, not a feature. Requested here rather than
        // via a manifest theme to sidestep zkite-pack's AXML compiler — it's a
        // phase-0-vintage spike with known gaps in resolving some framework resource
        // references (see phase_1/PLAN.md's `android:configChanges` finding), and a
        // code-only `Window` feature request has no such dependency at all.
        requestWindowFeature(Window.FEATURE_NO_TITLE);
        // The renderer pans the focused field using IME insets. Prevent Android
        // from also panning the entire window when the selection editor takes focus.
        if (android.os.Build.VERSION.SDK_INT >= 30) {
            getWindow().setSoftInputMode(android.view.WindowManager.LayoutParams.SOFT_INPUT_ADJUST_NOTHING);
        }


        // Same explicit-load-before-the-app-lib order as appshell's ZKiteActivity:
        // `libzkitegpuharness.so` is linked against `libpython3.14.so` (both staged
        // into the same lib/<abi>/ — see ../scripts/build_apk.sh), and loading the
        // dependency first avoids relying on the dynamic linker resolving it
        // implicitly via DT_NEEDED.
        System.loadLibrary("python3.14");
        System.loadLibrary("zkitegpuharness");

        showLoading();
        new Thread(new Runnable() {
            @Override public void run() {
                try {
                    File pythonPrefix = new File(getFilesDir(), "python");
                    File zkitePkgDir = new File(getFilesDir(), "zkite_framework");
                    File appDir = new File(getFilesDir(), "app");
                    File fontDir = new File(getFilesDir(), "fonts");

                    unpackZipAssetOnce(STDLIB_ASSET, pythonPrefix, "stdlib-" + STAMP_VERSION);
                    linkExtensionModules(pythonPrefix);
                    unpackZipAssetOnce(ZKITE_PKG_ASSET, zkitePkgDir, "zkitepkg-" + STAMP_VERSION);
                    // Optional, and this is the one asset that legitimately can be absent:
                    // the Dev Shell ships no app at all — `zkite dev` pushes `main.py` over
                    // the reload socket into exactly this directory, and `runtime.py`'s
                    // `_zkite_setup_tree` already renders a "waiting for `zkite dev`…" tree
                    // when it finds no main. Treating a missing app.zip as fatal (which is
                    // what unpackZipAssetOnce's IOException would do, via the catch below
                    // that also skips Native.boot) would make the Dev Shell impossible.
                    if (!unpackOptionalZipAssetOnce(APP_ASSET, appDir, "app-" + STAMP_VERSION)) {
                        Log.i(TAG, "no bundled app.zip — dev shell mode, waiting for `zkite dev`");
                    }
                    unpackZipAssetOnce(FONTS_ASSET, fontDir, "fonts-" + STAMP_VERSION);

                    runOnUiThread(new Runnable() {
                        @Override public void run() {
                            if (!isFinishing() && !isDestroyed()) finishStartup();
                        }
                    });
                } catch (final IOException error) {
                    Log.e(TAG, "asset unpack failed", error);
                    runOnUiThread(new Runnable() {
                        @Override public void run() {
                            if (!isFinishing() && !isDestroyed()) {
                                loadingProgress.setVisibility(View.GONE);
                                loadingMessage.setText("Unable to start. Please close and reopen the app.");
                            }
                        }
                    });
                }
            }
        }, "zkite-assets").start();
    }

    private void showLoading() {
        content = new FrameLayout(this);
        content.setBackgroundColor(Color.WHITE);
        android.widget.LinearLayout loading = new android.widget.LinearLayout(this);
        loading.setOrientation(android.widget.LinearLayout.VERTICAL);
        loading.setGravity(android.view.Gravity.CENTER);
        loading.setBackgroundColor(Color.WHITE);
        loading.setClickable(true);
        float density = getResources().getDisplayMetrics().density;
        android.widget.ImageView icon = new android.widget.ImageView(this);
        icon.setImageDrawable(getApplicationInfo().loadIcon(getPackageManager()));
        loading.addView(icon, new android.widget.LinearLayout.LayoutParams((int)(72*density), (int)(72*density)));
        loadingMessage = new android.widget.TextView(this);
        loadingMessage.setText(getApplicationInfo().loadLabel(getPackageManager()));
        loadingMessage.setTextColor(Color.rgb(30, 41, 59));
        loadingMessage.setTextSize(18);
        loadingMessage.setGravity(android.view.Gravity.CENTER);
        loadingMessage.setPadding((int)(24*density), (int)(20*density), (int)(24*density), (int)(20*density));
        loading.addView(loadingMessage);
        loadingProgress = new android.widget.ProgressBar(this);
        loading.addView(loadingProgress, new android.widget.LinearLayout.LayoutParams((int)(28*density), (int)(28*density)));
        loadingView = loading;
        content.addView(loading, new FrameLayout.LayoutParams(-1, -1));
        setContentView(content);
    }

    /** SurfaceView posts this only after the renderer has submitted a real frame. */
    void onFirstFrame() {
        if (loadingView == null || isDestroyed()) return;
        final View loading = loadingView;
        loadingView = null;
        loading.animate().alpha(0f).setDuration(120).withEndAction(new Runnable() {
            @Override public void run() { content.removeView(loading); }
        }).start();
    }

    private void finishStartup() {
        File pythonPrefix = new File(getFilesDir(), "python");
        File zkitePkgDir = new File(getFilesDir(), "zkite_framework");
        File appDir = new File(getFilesDir(), "app");
        File fontDir = new File(getFilesDir(), "fonts");
        // This device's own calibrated tap-vs-drag distance (../../PLAN.md §5) —
        // not a guessed pixel constant, since touch slop is
        // density/OEM-dependent and every other Android view already uses
        // exactly this value for exactly this decision.
        float touchSlopPx = ViewConfiguration.get(this).getScaledTouchSlop();
        // Same reasoning, for double-tap-to-select-word (../../PLAN.md §5/§7.5):
        // real platform constants, not guessed ones, the same way touchSlopPx
        // already is — `getScaledDoubleTapSlop()` is the max finger movement
        // between the two taps, `getDoubleTapTimeout()` (a plain static, no
        // density scaling) the max time between them.
        float doubleTapSlopPx = ViewConfiguration.get(this).getScaledDoubleTapSlop();
        float doubleTapTimeoutMs = ViewConfiguration.getDoubleTapTimeout();

        // This display's device-pixels-per-dp (../../PLAN.md §13). The renderer
        // lays out in dp and scales once at paint; without this it laid out
        // Tailwind's lengths against a device-pixel surface and every app came
        // out 1/density too small on every real phone. Read from the platform
        // rather than derived from DPI by hand: `density` is what the framework
        // itself uses for every dp conversion (and what the WebView's own device
        // pixel ratio follows), so taking it from anywhere else could disagree.
        float density = getResources().getDisplayMetrics().density;

        // This panel's actual refresh rate (../../PLAN.md §13.2). Same
        // "ask the platform, don't guess" reasoning as touchSlopPx and density:
        // a fling used to be paced by a hardcoded 16ms, which is both the wrong
        // number on a 90/120Hz display and — more importantly — was being waited
        // *in addition to* each frame's own work rather than inclusive of it.
        float refreshRateHz = getWindowManager().getDefaultDisplay().getRefreshRate();

        // The platform half of the cold-start measurement (../../PLAN.md §12.2
        // item 1): process start → here is zygote fork, Activity creation and
        // this shell's own asset unpack. The render thread logs the other half
        // (`ZKITEPERF first-frame since_boot_call=`), so the two sum to
        // process-start → first pixel with neither side reasoning about the
        // other's clock. Split rather than combined because the split is the
        // more useful shape: it separates what this backend controls from what
        // the platform charges every app regardless of backend, which is exactly
        // the distinction §12 has to draw to compare two backends fairly.
        Log.i(TAG, "ZKITEPERF boot-call process_rel="
                + (SystemClock.elapsedRealtime() - Process.getStartElapsedRealtime()) + "ms");

        // Safe to call directly on the UI thread — see Native.boot's doc comment:
        // this only spawns a thread, it does not itself do any Python init,
        // rendering, or block on GPU init.
        Native.boot(pythonPrefix.getAbsolutePath(), zkitePkgDir.getAbsolutePath(),
                appDir.getAbsolutePath(), fontDir.getAbsolutePath(), touchSlopPx,
                doubleTapSlopPx, doubleTapTimeoutMs, density, refreshRateHz);

        nativeBooted = true;
        startReloadServerIfDevShell(appDir);

        surfaceView = new ZKiteSurfaceView(this);
        // Opt the window into a *stable, always-full-window* content frame before
        // anything is laid out (../../PLAN.md §7.14). Without this, Android sizes the
        // content frame differently depending on whether the IME has ever been shown
        // — at launch it is inset by the system bars, and after an IME show/hide cycle
        // it is the full window — which silently moves the SurfaceView (and therefore
        // its surface) up by the status-bar height mid-session. Pinning it to
        // "always full window" makes the inset margins applied below the *only* thing
        // positioning the surface, so its geometry no longer depends on IME history.
        // This is also the direction the platform is going regardless: edge-to-edge is
        // mandatory for apps targeting SDK 35+, so opting in deliberately now is
        // strictly better than inheriting it later by surprise.
        Window window = getWindow();
        if (Build.VERSION.SDK_INT >= 30) {
            window.setDecorFitsSystemWindows(false);
        } else {
            // Same effect on 26-29, where setDecorFitsSystemWindows doesn't exist:
            // lay out edge to edge and keep those bounds stable while system bars
            // come and go.
            window.getDecorView().setSystemUiVisibility(
                    View.SYSTEM_UI_FLAG_LAYOUT_STABLE
                            | View.SYSTEM_UI_FLAG_LAYOUT_FULLSCREEN
                            | View.SYSTEM_UI_FLAG_LAYOUT_HIDE_NAVIGATION);
        }
        systemBarsView = new SystemBarsView(this);
        content.addView(systemBarsView, 0, new FrameLayout.LayoutParams(-1, -1));
        content.addView(surfaceView, 1, new FrameLayout.LayoutParams(-1, -1));
        applySystemBars("{}");
        applyInsetsAsMargins(surfaceView);
        registerBackHandler();
    }

    /** Called on the UI thread by ZKiteSurfaceView when Python's screen changes. */
    void applySystemBars(String json) {
        try {
            JSONObject style = new JSONObject(json);
            int status = Color.parseColor(style.optString("status_color", "#ffffff"));
            int navigation = Color.parseColor(style.optString("navigation_color", "#ffffff"));
            boolean statusDark = style.optBoolean("status_dark_icons", true);
            boolean navigationDark = style.optBoolean("navigation_dark_icons", true);
            systemBarsView.setColors(status, navigation);
            Window window = getWindow();
            window.setStatusBarColor(Color.TRANSPARENT);
            window.setNavigationBarColor(Color.TRANSPARENT);
            if (Build.VERSION.SDK_INT >= 28) window.setNavigationBarDividerColor(Color.TRANSPARENT);
            if (Build.VERSION.SDK_INT >= 29) {
                window.setStatusBarContrastEnforced(false);
                window.setNavigationBarContrastEnforced(false);
            }
            if (Build.VERSION.SDK_INT >= 30) {
                WindowInsetsController controller = window.getInsetsController();
                if (controller != null) {
                    int mask = WindowInsetsController.APPEARANCE_LIGHT_STATUS_BARS
                            | WindowInsetsController.APPEARANCE_LIGHT_NAVIGATION_BARS;
                    int appearance = (statusDark ? WindowInsetsController.APPEARANCE_LIGHT_STATUS_BARS : 0)
                            | (navigationDark ? WindowInsetsController.APPEARANCE_LIGHT_NAVIGATION_BARS : 0);
                    controller.setSystemBarsAppearance(appearance, mask);
                }
            } else {
                View decor = window.getDecorView();
                int flags = decor.getSystemUiVisibility()
                        & ~(View.SYSTEM_UI_FLAG_LIGHT_STATUS_BAR | View.SYSTEM_UI_FLAG_LIGHT_NAVIGATION_BAR);
                if (statusDark) flags |= View.SYSTEM_UI_FLAG_LIGHT_STATUS_BAR;
                if (navigationDark) flags |= View.SYSTEM_UI_FLAG_LIGHT_NAVIGATION_BAR;
                decor.setSystemUiVisibility(flags);
            }
        } catch (Exception error) {
            Log.e(TAG, "invalid system bar settings", error);
        }
    }

    /**
     * Routes Back to {@code zkite_py}'s navigation stack (../../PLAN.md §16).
     *
     * <p><b>Two mechanisms, because the platform has two.</b> On API 33+ the modern
     * one is {@code OnBackInvokedDispatcher}, which is also what drives the
     * predictive-back animation; on older releases it is {@code onBackPressed()}.
     * Registering the modern callback <em>suppresses</em> {@code onBackPressed()}
     * entirely once the app is {@code enableOnBackInvokedCallback}-opted-in, so the
     * two never both fire and there is no double-pop to guard against — but only if
     * both are wired, which is why this app does both rather than picking one.
     *
     * <p>Registered with {@code PRIORITY_DEFAULT}: this must sit below any system
     * override and below whatever a future {@code Sheet}/{@code Dialog} registers,
     * which is the same ordering the Python-side back chain already implements
     * within one render (innermost handler first).
     *
     * <p>Both paths <b>always consume</b> the press and never call
     * {@code super.onBackPressed()}. Whether anything actually wanted it is only
     * knowable inside {@code zkite_py}, behind the render thread's queue, so the
     * answer arrives asynchronously — see {@link Native#onBackPressed()}. When the
     * answer is "nothing wanted it", the render thread calls
     * {@link ZKiteSurfaceView#finishActivity()} and the app exits exactly as it did
     * before navigation existed.
     */
    private static final class BackCallback implements android.window.OnBackInvokedCallback {
        @Override
        public void onBackInvoked() {
            Log.i(TAG, "back: OnBackInvokedCallback fired");
            Native.onBackPressed();
        }
    }

    private void registerBackHandler() {
        if (Build.VERSION.SDK_INT >= 33) {
            getOnBackInvokedDispatcher().registerOnBackInvokedCallback(
                    android.window.OnBackInvokedDispatcher.PRIORITY_DEFAULT,
                    new BackCallback());
            Log.i(TAG, "back: registered OnBackInvokedCallback (SDK " + Build.VERSION.SDK_INT + ")");
        } else {
            Log.i(TAG, "back: legacy onBackPressed path only (SDK " + Build.VERSION.SDK_INT + ")");
        }
    }

    /**
     * Pre-API-33 Back. Deliberately does not call {@code super.onBackPressed()} — see
     * {@link #registerBackHandler()} for why every Back press is consumed here and
     * the exit decision is made on the render thread instead.
     *
     * <p>On API 33+ with the callback above registered this is never invoked, so the
     * two cannot both act on one press.
     */
    @Override
    public void onBackPressed() {
        Log.i(TAG, "back: onBackPressed() called");
        if (nativeBooted) Native.onBackPressed();
        else finish();
    }

    @Override
    protected void onResume() {
        super.onResume();
        if (deviceServices != null) deviceServices.resume();
        if (surfaceView != null && surfaceView.getHolder().getSurface() != null && surfaceView.getHolder().getSurface().isValid()) {
            int w = surfaceView.getWidth();
            int h = surfaceView.getHeight();
            if (w > 0 && h > 0) {
                Native.onSurfaceChanged(w, h);
            }
        }
    }

    @Override
    protected void onPause() {
        if (deviceServices != null) deviceServices.pause();
        super.onPause();
    }

    @Override
    protected void onActivityResult(int request, int result, android.content.Intent data) {
        super.onActivityResult(request, result, data);
        if (deviceServices != null) deviceServices.onActivityResult(request, result, data);
    }

    @Override
    public void onRequestPermissionsResult(int request, String[] permissions, int[] results) {
        super.onRequestPermissionsResult(request, permissions, results);
        if (deviceServices != null) deviceServices.onPermissions(request, permissions, results);
    }

    @Override
    protected void onDestroy() {
        if (deviceServices != null) deviceServices.destroy();
        // The user is actually leaving — stop the render thread and release its GPU
        // resources rather than letting them outlive the Activity for however long the
        // process happens to stick around. Distinct from surfaceDestroyed (fires on
        // every rotation too, and only tears down the surface, not the GPU device).
        if (nativeBooted) Native.onDestroy();
        super.onDestroy();
    }

    /**
     * Insets the surface into the safe area using layout <b>margins</b>, not padding
     * (../../PLAN.md §7.14).
     *
     * <p><b>Why margins — the bug this replaces.</b> This method used to call
     * {@code view.setPadding(...)}. Padding on an ordinary {@code View} insets its
     * <em>content</em>; padding on a {@code SurfaceView} does <b>nothing</b> to the
     * surface, because a {@code SurfaceView}'s surface is sized and positioned from
     * the view's own layout bounds and ignores padding entirely. The old code
     * therefore never actually inset anything — it only <em>appeared</em> to work at
     * launch, because the content frame Android handed the Activity happened to
     * already be inset by the system bars. The moment the IME was shown and hidden,
     * that frame became the full window, the SurfaceView was re-laid-out at y=0
     * instead of y=117, and every pixel ZKite drew moved up by exactly the status-bar
     * height — pushing the topmost widget off-screen behind the status bar for the
     * rest of the session. That is the long-standing "a text node vanishes once a
     * TextField is focused, and never comes back" symptom (../../PLAN.md §7.6): the
     * glyph was never failing to render, it was being drawn underneath the status
     * bar. Margins genuinely move and resize the view, so the surface follows them,
     * and every coordinate space downstream (touch, layout, caret, a11y bounds)
     * keeps its existing meaning of "relative to the safe area" with no changes.
     *
     * <p>Paired with {@code setDecorFitsSystemWindows(false)} in {@link #onCreate}:
     * the parent frame is now always the full window, so these margins are the single
     * source of truth for where the surface sits, rather than being added on top of
     * an inset the platform may or may not have already applied.
     */
    private static final class InsetsListener implements View.OnApplyWindowInsetsListener {
        private final SystemBarsView systemBarsView;

        InsetsListener(SystemBarsView systemBarsView) {
            this.systemBarsView = systemBarsView;
        }

        @Override
        public WindowInsets onApplyWindowInsets(View view, WindowInsets insets) {
            int top, bottom, left, right;
            if (Build.VERSION.SDK_INT >= 30) {
                // Deliberately systemBars() only, *not* systemBars() | ime(): the
                // keyboard must not resize the surface. ZKite handles the IME by
                // panning the focused field into view itself
                // (`render_thread.rs`'s `compute_ime_pan`, ../../PLAN.md §7.9),
                // driven by ZKiteSurfaceView.onImeInsetsChanged. Folding the IME
                // inset in here as well would resize the surface on every
                // keyboard show/hide — churning the whole GPU swapchain and
                // vello_hybrid renderer twice per focus — *and* double-count
                // against that pan.
                Insets bars = insets.getInsets(WindowInsets.Type.systemBars());
                top = bars.top; bottom = bars.bottom; left = bars.left; right = bars.right;
            } else {
                top = insets.getSystemWindowInsetTop();
                bottom = insets.getSystemWindowInsetBottom();
                left = insets.getSystemWindowInsetLeft();
                right = insets.getSystemWindowInsetRight();
            }
            systemBarsView.setBarInsets(left, right, bottom);
            ViewGroup.LayoutParams base = view.getLayoutParams();
            if (base instanceof ViewGroup.MarginLayoutParams) {
                ViewGroup.MarginLayoutParams lp = (ViewGroup.MarginLayoutParams) base;
                if (lp.leftMargin != left || lp.topMargin != top
                        || lp.rightMargin != right || lp.bottomMargin != bottom) {
                    lp.leftMargin = left;
                    lp.topMargin = top;
                    lp.rightMargin = right;
                    lp.bottomMargin = bottom;
                    view.setLayoutParams(lp);
                }
            } else {
                // setContentView wraps the view in a FrameLayout, so this should
                // be unreachable; logged rather than silently leaving the surface
                // un-inset (which is exactly the failure mode this method exists
                // to have fixed).
                Log.e(TAG, "surface view's LayoutParams are not MarginLayoutParams; insets not applied");
            }
            // The overlay is a *sibling* of the surface view in the same (now
            // full-window) parent, and positions itself with margins in the
            // surface's own coordinate space — so it has to be shifted by the
            // same amount the surface just was, or it would land under the
            // status bar while the field it covers sits below it.
            if (view instanceof ZKiteSurfaceView) {
                ((ZKiteSurfaceView) view).onSystemBarInsetsChanged(left, top, right, bottom);
                // A view has only one OnApplyWindowInsetsListener at a time —
                // ZKiteSurfaceView.forwardWindowInsets (../../PLAN.md §5) needs the
                // *real* system insets delivery to drive GameTextInput's IME-
                // visibility detection, so it can't install its own listener here
                // too; this is the one place real insets deliveries actually land.
                ((ZKiteSurfaceView) view).forwardWindowInsets(insets);
            }
            return insets;
        }
    }

    private void applyInsetsAsMargins(View v) {
        v.setOnApplyWindowInsetsListener(new InsetsListener(systemBarsView));
        // The listener above only runs on the *next* insets dispatch. Ask for one now
        // so the very first frame is already inset correctly, rather than rendering
        // one full-window frame and then jumping.
        v.requestApplyInsets();
    }

    /**
     * {@link #unpackZipAssetOnce} for an asset that may legitimately not be bundled.
     * Returns whether it was there. Still creates the destination directory either
     * way, so callers downstream (and {@code zkite dev}, which pushes files into it)
     * always have somewhere to write.
     */
    private boolean unpackOptionalZipAssetOnce(String assetName, File destDir, String stampName)
            throws IOException {
        boolean present;
        InputStream probe = null;
        try {
            probe = getAssets().open(assetName);
            present = true;
        } catch (IOException e) {
            present = false;
        }
        if (probe != null) {
            try {
                probe.close();
            } catch (IOException ignored) {
            }
        }
        if (present) {
            unpackZipAssetOnce(assetName, destDir, stampName);
        } else if (!destDir.isDirectory() && !destDir.mkdirs()) {
            throw new IOException("could not create " + destDir);
        }
        return present;
    }

    /**
     * Starts the {@code zkite dev} reload socket, but <b>only in a Dev Shell build</b>
     * (../../../../PLAN.md §15).
     *
     * <p>This one Activity class fronts two very different artifacts: the generic
     * Dev Shell that {@code zkite dev} installs once and pushes Python into, and every
     * standalone app {@code zkite build --backend native} produces. They differ by
     * exactly one bundled marker asset. phase_2/PLAN.md §1.2 is explicit about why a
     * release build must not run this: there is no {@code zkite dev} for it to reach,
     * so the retry loop would be an infinite background reconnect burning battery,
     * behind an {@code INTERNET} permission with no runtime purpose — and the
     * generated manifest for a built app correspondingly does not request one, so
     * starting it there would fail at the socket anyway.
     *
     * <p>Checked by opening the asset rather than by a build-time constant, so the
     * shipped {@code .so} and {@code classes.dex} stay byte-identical between the two
     * artifacts — the Dev Shell and a release build differ only in what is packed
     * beside them, which is what makes {@code zkite build --backend native} able to
     * reuse the Dev Shell's own prebuilt binaries.
     */
    private void startReloadServerIfDevShell(File appDir) {
        InputStream marker = null;
        try {
            marker = getAssets().open(DEVMODE_ASSET);
        } catch (IOException e) {
            return; // a normal built app — not a Dev Shell
        }
        try {
            marker.close();
        } catch (IOException ignored) {
        }
        Log.i(TAG, "dev shell: starting reload socket on 127.0.0.1:" + RELOAD_PORT);
        Native.startReloadServer("127.0.0.1", RELOAD_PORT, appDir.getAbsolutePath());
    }

    /** Refresh managed assets when their installed APK bytes change. */
    private void unpackZipAssetOnce(String assetName, File destDir, String stampName) throws IOException {
        long t0 = System.nanoTime();
        if (dev.zkite.runtime.ZipAssets.unpack(new File(getApplicationInfo().sourceDir),
                assetName, destDir, stampName)) {
            Log.i(TAG, "unpacked " + assetName + " in "
                    + ((System.nanoTime() - t0) / 1_000_000) + " ms");
        }
    }

    /** Identical rationale to phase_2/appshell: extension modules must dlopen from
     * the app's own native lib dir (Android 10+), so they're staged into
     * lib/&lt;abi&gt;/ at build time and pointed to via a .pth file `site` picks up
     * at interpreter startup. */
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
