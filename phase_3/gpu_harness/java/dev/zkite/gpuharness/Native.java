package dev.zkite.gpuharness;

import android.view.Surface;

/**
 * The JNI surface of the GPU render harness (../../PLAN.md §4). Thin calls into
 * Rust (../src/lib.rs), which dispatches onto a dedicated render thread
 * (../src/render_thread.rs) rather than doing any work on the calling thread —
 * see that file's module doc for why.
 */
public final class Native {
    private Native() {}

    /**
     * Spawns the render thread, which embeds CPython + {@code zkite_py} on itself
     * (../../PLAN.md §5) before doing anything else. {@code pythonPrefix}/
     * {@code zkitePkgDir}/{@code appDir} are the same three unpacked-asset paths
     * {@code phase_2.appshell.Native#boot} takes; {@code fontDir} is this harness's own
     * pre-existing need, now a directory rather than two file paths — it holds the
     * six faces (sans/serif/mono x regular/bold) staged by
     * {@code phase_1/webruntime/scripts/stage_fonts.sh}, which is the same set the
     * WebView backend loads via {@code @font-face} so both backends render identical
     * typefaces (../../PLAN.md §9.4). A bold face per family is required so
     * {@code zkite_renderer::Renderer} has something heavier to select for
     * {@code font-bold}/{@code font-semibold}/etc. (../../PLAN.md §9.2 item 1 — a
     * single registered face silently rendered every weight identically);
     * {@code touchSlopPx} is
     * {@code ViewConfiguration.getScaledTouchSlop()}, read once by
     * {@code ZKiteActivity} and handed down for tap-vs-drag gesture disambiguation
     * (../../PLAN.md §5) — this device's own calibrated value, not a guessed
     * constant on the Rust side. {@code doubleTapSlopPx}/{@code doubleTapTimeoutMs}
     * are the same idea for double-tap-to-select-word (../../PLAN.md §5/§7.5) —
     * {@code ViewConfiguration.getScaledDoubleTapSlop()}/{@code
     * .getDoubleTapTimeout()}.
     *
     * <p>{@code density} is {@code DisplayMetrics.density} — this display's device
     * pixels per density-independent pixel (dpi/160). It is the whole of ../../PLAN.md
     * §13's fix: without it the renderer laid out Tailwind's own unit-less lengths
     * (twcore resolves {@code px-6} to 24) directly against a surface measured in
     * *device* pixels, so every native app rendered at {@code 1/density} the size of
     * the same app on the WebView backend, which treats those as CSS px and multiplies
     * by the device pixel ratio itself. Everything above the paint/touch boundary works
     * in dp from here on; this is the one number that converts between the two spaces,
     * so it is read once, from the platform, and never guessed.
     *
     * <p>{@code refreshRateHz} is {@code Display.getRefreshRate()} — the period an
     * in-flight fling aims to deliver frames at (../../PLAN.md §13.2). Read from the
     * platform rather than hardcoded because this device advertises 30/60/90/120 and
     * runs at 60: a fixed 16ms would under-drive a 120Hz panel by half and over-drive
     * a 30Hz one. It is a target period, not a vsync signal — the swapchain is still
     * what synchronizes to the display.
     *
     * <p>Safe to call from the UI thread directly (unlike the
     * WebView backends' Python boot, which runs synchronously on the calling thread):
     * all this does synchronously is a small file read and a thread spawn — Python
     * init, font registration, and every render happen on the spawned thread. A
     * second call while a render thread is already running is a no-op (logged).
     */
    public static native void boot(String pythonPrefix, String zkitePkgDir, String appDir, String fontDir,
            float touchSlopPx, float doubleTapSlopPx, float doubleTapTimeoutMs, float density,
            float refreshRateHz);

    /** Forwards a new/recreated drawing surface, in {@code width}x{@code height} pixels. */
    public static native void onSurfaceCreated(Surface surface, int width, int height);

    /** Forwards a surface resize (including the one Android delivers right after create). */
    public static native void onSurfaceChanged(int width, int height);

    /**
     * Blocks until the render thread has dropped the GPU surface and released the
     * {@code ANativeWindow} — must be called, and must be allowed to block, before
     * {@code surfaceDestroyed} returns; see Android's own contract for that callback.
     */
    public static native void onSurfaceDestroyed();

    /** Forwards ACTION_DOWN, in surface-local pixels — starts a gesture. */
    public static native void onTouchDown(float x, float y);

    /** Forwards ACTION_MOVE, in surface-local pixels — may turn a gesture into a drag/scroll. */
    public static native void onTouchMove(float x, float y);

    /** Forwards ACTION_UP, in surface-local pixels — ends a gesture, dispatching a tap
     * if (and only if) it never turned into a drag. */
    public static native void onTouchUp(float x, float y);

    /** Forwards ACTION_CANCEL — ends a gesture without ever dispatching a tap,
     * regardless of how little it had moved; see {@code ZKiteSurfaceView}'s doc. */
    public static native void onTouchCancel();

    /** Refresh the current accessibility tree when a service attaches to an idle app. */
    public static native void requestRedraw();

    /**
     * Registers {@code ZKiteSurfaceView} itself with the render thread (../../PLAN.md
     * §5, IME) — needed so native can later call {@code focusAndShowKeyboard}/
     * {@code blurAndHideKeyboard} on it. Called once, from the view's own
     * constructor.
     */
    public static native void setSurfaceView(android.view.View view);

    /**
     * Forwards a freshly built {@code gametextinput.InputConnection}, from
     * {@code ZKiteSurfaceView#onCreateInputConnection} (../../PLAN.md §5). Declared
     * as {@code Object}, not the concrete vendored type, so this file doesn't need a
     * compile dependency on {@code vendor/gametextinput/classes.jar} — the native
     * side only ever treats it as an opaque {@code jobject} (see
     * {@code text_input.rs}'s module doc on the global-ref handoff this implies).
     */
    public static native void onInputConnectionCreated(Object inputConnection);

    /**
     * Forwards a {@code gametextinput.State} update from
     * {@code ZKiteSurfaceView}'s {@code Listener.stateChanged} — see
     * {@link #onInputConnectionCreated} for why this is {@code Object}.
     */
    public static native void onTextInputEvent(Object state);

    /**
     * The IME was dismissed some way other than native's own blur path (the back
     * button, an editor "done" action, ...) — see ../../PLAN.md §5.
     */
    public static native void onKeyboardDismissed();

    /**
     * Pushes a new {@code [start, end)} selection into the focused field's live
     * {@code Editable}, replacing its text with {@code text} in the same call (see
     * {@code ZKiteSurfaceView.setSelection}'s own doc comment on why the caller must
     * always pass the field's real, current text, never a stale copy) — for
     * tap-to-position-cursor (../../PLAN.md §7.7). Must only be called already on
     * the UI thread.
     */
    public static native void setSelection(String text, int start, int end);

    /**
     * Shows the text-selection overlay (../../PLAN.md §7.8) — a real, fully visible
     * {@code android.widget.EditText} positioned over the just-long-pressed
     * {@code TextField}'s own screen rect, so Android's native selection handles and
     * copy/paste toolbar do the work instead of a hand-built equivalent. {@code x}/
     * {@code y}/{@code w}/{@code h} are surface-local pixels, the same coordinate
     * space {@link #onTouchDown} etc. already use; {@code textColor}/{@code bgColor}
     * are packed {@code 0xAARRGGBB} ints, matching {@code android.graphics.Color}'s
     * own convention. {@code isPassword} (../../PLAN.md §5/§7.5) is the field's own
     * `secure=True` — the overlay gets a real {@code TYPE_TEXT_VARIATION_PASSWORD}
     * when set, so it masks (and disables Cut/Copy for) the real text natively,
     * rather than being handed already-masked characters. Must only be called
     * already on the UI thread.
     */
    public static native void showSelectionOverlay(String text, int selStart, int selEnd,
            float x, float y, float w, float h, float textSizePx, int textColor, int bgColor,
            boolean isPassword);

    /**
     * Hides the overlay {@link #showSelectionOverlay} showed, if any — e.g. because
     * a tap landed elsewhere. A no-op if it isn't currently showing. Must only be
     * called already on the UI thread.
     */
    public static native void hideSelectionOverlay();

    /**
     * The overlay's own text or selection changed — typing, a dragged handle, or a
     * cut/paste from its native toolbar (../../PLAN.md §7.8). {@code text}/
     * {@code start}/{@code end} are its new, complete state, the same shape
     * {@link #onTextInputEvent} decodes from {@code GameTextInput}, but sourced from
     * a real {@code EditText}'s {@code TextWatcher}/{@code onSelectionChanged}
     * instead. Only sent while the overlay still has focus — see
     * {@link #onSelectionOverlayDismissed} for its final state.
     */
    public static native void onSelectionOverlayChanged(String text, int start, int end);

    /**
     * The overlay lost focus (a tap elsewhere took it, or native explicitly called
     * {@link #hideSelectionOverlay}) — {@code text}/{@code start}/{@code end} are
     * its final state at the moment of dismissal, so native can fold it back into
     * {@code focused_text}/{@code focused_selection} exactly once rather than
     * relying only on the (already-sent) last {@link #onSelectionOverlayChanged}.
     */
    public static native void onSelectionOverlayDismissed(String text, int start, int end);

    /**
     * The IME's own real bottom inset changed — {@code bottomPx} is how much of the
     * bottom of this view is currently covered (the keyboard's height when shown,
     * zero once it's gone), in the same surface-local pixel space every other touch/
     * layout call here already uses (../../PLAN.md §5/§7.5). Drives
     * {@code render_thread.rs}'s IME pan-into-view compensation.
     */
    public static native void onImeInsetsChanged(int bottomPx);

    /**
     * Stops the render thread and releases its GPU resources (device/adapter/queue) —
     * call once, from {@code ZKiteActivity.onDestroy()}, when the user is actually
     * leaving the app. Distinct from {@link #onSurfaceDestroyed()}: that fires on
     * every rotation/backgrounding and only tears down the surface, deliberately
     * keeping the GPU device warm for the next {@code onSurfaceCreated}.
     */
    public static native void onDestroy();

    /**
     * The user pressed Back (../../../../PLAN.md §16) — the system button, the back
     * gesture, or Android 13+'s {@code OnBackInvokedCallback}. {@code ZKiteActivity}
     * funnels all three here.
     *
     * <p><b>Returns immediately; it does not report whether the press was
     * handled.</b> Whether anything wanted it is only knowable inside {@code zkite_py}
     * (a {@code Nav} on screen may want to pop), which lives behind the render
     * thread's queue — and that thread may be mid-frame or inside a CPython call.
     * Blocking the UI thread on it would be an ANR waiting for a slow frame, so the
     * caller always consumes the press and the render thread calls
     * {@link ZKiteSurfaceView#finishActivity()} back on the UI thread if nothing
     * wanted it.
     */
    public static native void onBackPressed();

    /**
     * Starts the {@code zkite dev} reload socket (../../../../PLAN.md §15) — a
     * background thread that dials {@code host:port}, which {@code adb reverse}
     * forwards to the host's {@code zkite dev} process, and applies pushed files plus
     * reload requests.
     *
     * <p><b>Only called in a Dev Shell build</b>, gated on the {@code zkite-devmode}
     * marker asset — see {@code ZKiteActivity.startReloadServerIfDevShell}. A shipped
     * {@code zkite build --backend native} app must never run it (phase_2/PLAN.md
     * §1.2), and its generated manifest correspondingly has no {@code INTERNET}
     * permission.
     *
     * <p>Unlike the Dev Shell's WebView twin, this has no ordering requirement
     * against {@link #boot}: the socket thread never touches Python or JNI, it only
     * sends one command down the render thread's existing channel, and the render
     * thread initialises the interpreter before it reads that channel at all.
     */
    public static native void startReloadServer(String host, int port, String appDir);
}
