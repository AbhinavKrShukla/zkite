package dev.zkite.gpuharness;

import android.app.Activity;
import android.content.Context;
import android.text.InputType;
import android.util.Log;
import android.view.MotionEvent;
import android.view.SurfaceHolder;
import android.view.SurfaceView;
import android.view.WindowInsets;
import android.view.inputmethod.EditorInfo;
import android.view.inputmethod.InputMethodManager;

import androidx.core.graphics.Insets;
import androidx.core.view.WindowInsetsCompat;

import com.google.androidgamesdk.gametextinput.GameTextInput;
import com.google.androidgamesdk.gametextinput.InputConnection;
import com.google.androidgamesdk.gametextinput.Listener;
import com.google.androidgamesdk.gametextinput.Settings;
import com.google.androidgamesdk.gametextinput.State;

/**
 * The native-rendered counter app's surface (../../PLAN.md §1/§4/§5). Everything this
 * class does is forward Android lifecycle/input events into Rust unchanged — no
 * layout, no paint, no state, and (as of §5) no tap-vs-scroll decision either: down/
 * move/up all cross the JNI boundary as-is, and `render_thread.rs`'s own gesture
 * state machine decides what they mean, the same "this side is plumbing, the far side
 * of JNI is logic" split every other piece of this class already follows. Deliberately
 * a plain {@code SurfaceView}, not a {@code TextureView}: this app draws full-screen
 * with no need to be transformed, animated, or composited as an ordinary View layer,
 * so the lower-overhead {@code SurfaceView} (its own independent {@code Surface}, not
 * a GPU texture the rest of the View hierarchy has to composite) is the right default
 * per Android's own guidance — revisit only if a future feature (e.g. a transition)
 * needs otherwise.
 *
 * <p><b>As of ../../PLAN.md §5, also this app's IME entry point</b> — focusable, with
 * a real {@code onCreateInputConnection} backed by {@code vendor/gametextinput}'s
 * {@code InputConnection} rather than a hand-rolled one (see that plan section, and
 * phase 0's README, for why: keyboard-vendor-specific {@code InputConnection} bugs
 * are exactly the long tail this vendored class exists to absorb instead of ZKite's
 * own surface area). Showing/hiding the keyboard goes through plain
 * {@link InputMethodManager} calls here ({@link #focusAndShowKeyboard}/
 * {@link #blurAndHideKeyboard}), not GameTextInput's own C-side
 * {@code showIme}/{@code hideIme} — see {@code text_input.rs}'s module doc for the
 * reasoning (in short: the C calls assume an already-established
 * {@code InputConnection}, which doesn't exist yet on first focus, and this file's
 * two methods are simpler to reason about than when that assumption does or doesn't
 * hold).
 *
 * <p>No lambdas / method references, same {@code javac -source 8} reason as every
 * other Java file in this codebase — see {@code ZKiteActivity}'s class doc.
 */
public class ZKiteSurfaceView extends SurfaceView implements SurfaceHolder.Callback, Listener {
    /** The IME's own view of the focused field's text/selection/composing region —
     * held only so a future caller could inspect it; the render thread already gets
     * every update via {@link #stateChanged}, this isn't read from anywhere yet. */
    private InputConnection gameTextInputConnection;

    /** The `input_type` spec (../../PLAN.md §5/§7.5, "text"/"password"/"email"/...
     * — {@code zkite_py}'s {@code TextField._tree_attrs}, passed down verbatim by
     * {@code render_thread.rs}'s {@code focus_text_field}) that the *next*
     * {@link #onCreateInputConnection} should build its {@code EditorInfo} from.
     * Set by {@link #focusAndShowKeyboard} before it does anything that could
     * trigger that callback (first-ever focus, or a {@code restartInput} for a
     * field whose type differs from whatever's currently active) — `onCreate
     * InputConnection` itself takes no parameters of its own, so this is the only
     * way to hand it *which* field is being focused. */
    private String pendingInputTypeSpec = "text";
    /** Whichever spec {@link #onCreateInputConnection} last actually built an
     * `EditorInfo` from — compared against a new {@link #focusAndShowKeyboard}
     * call's spec to decide whether a `restartInput` (forcing a fresh
     * `InputConnection`, the only way to change an *already-focused* view's
     * keyboard type) is actually needed, versus just re-showing the same one. */
    private String currentInputTypeSpec;

    public ZKiteSurfaceView(Context context) {
        super(context);
        getHolder().addCallback(this);
        // Neither is on by default for a plain SurfaceView — without both, focus
        // (and therefore onCreateInputConnection) can never reach this view via a
        // touch, only via a hardware d-pad/keyboard, which real phones don't have.
        setFocusable(true);
        setFocusableInTouchMode(true);
        Native.setSurfaceView(this);
    }

    private static final class ApplySystemBarsRunnable implements Runnable {
        private final ZKiteSurfaceView view;
        private final String json;

        ApplySystemBarsRunnable(ZKiteSurfaceView view, String json) {
            this.view = view;
            this.json = json;
        }

        @Override
        public void run() {
            if (view.getContext() instanceof ZKiteActivity) {
                ((ZKiteActivity) view.getContext()).applySystemBars(json);
            }
        }
    }

    /** JNI enters on the render thread; window changes belong to the UI thread. */
    public void applySystemBars(final String json) {
        post(new ApplySystemBarsRunnable(this, json));
    }

    /**
     * Forwards a real system {@link WindowInsets} delivery to
     * {@link #gameTextInputConnection}, if one exists yet. Called from
     * {@code ZKiteActivity}'s own insets-as-padding listener (not installed as this
     * view's *own* {@code OnApplyWindowInsetsListener} — a view has only one such
     * listener at a time, {@code setOnApplyWindowInsetsListener} replaces
     * whichever was set before, and {@code ZKiteActivity} already needs one for
     * padding; see its own comment on this call site).
     *
     * <p><b>Third real bug in this integration, found the same way as the two
     * documented on {@link #focusAndShowKeyboard} (a real device, not inspection):
     * </b> {@code gti.InputConnection}'s own {@code mSoftKeyboardActive} field —
     * the *only* thing that gates whether {@code Listener.stateChanged} ever fires
     * (traced via {@code javap}, since gametextinput ships no source) — is written
     * in exactly one place: {@code InputConnection.onApplyWindowInsets}, comparing
     * the IME's real visibility (from the {@code WindowInsetsCompat} the system
     * delivers) against its own cached value. It is <b>not</b> set by
     * {@code setSoftKeyboardActive} despite that method's name — that method only
     * drives {@code requestFocus}/{@code showSoftInput}/{@code hideSoftInputFromWindow}.
     * Nothing called {@code onApplyWindowInsets} before this fix, so every commit
     * was silently landing in {@code gameTextInputConnection}'s {@code Editable}
     * (confirmed via its own debug logcat lines — the edits were real) while
     * {@code mSoftKeyboardActive} stayed permanently {@code false} and the listener
     * call was skipped every single time. The fix is giving it the *real* system
     * insets callback, not synthesizing one.
     *
     * <p><b>Fourth real bug, found immediately after fixing the third (a real
     * device, not inspection):</b> {@code WindowInsetsCompat.toWindowInsetsCompat(
     * insets, view)} (the two-arg overload, passing {@code this}) crashed with
     * {@code NoClassDefFoundError: androidx/core/R$id} — {@code ViewCompat}'s
     * static initializer reads dozens of {@code androidx.core.R$id} fields (View-tag
     * keys for its accessibility/insets bookkeeping), and that {@code R$id} class is
     * generated by AAPT2 from the AAR's own {@code res/values/ids.xml} at normal
     * Gradle build time — it does not exist in {@code classes.jar}, and this
     * pipeline (../../PLAN.md's whole premise) has no AAPT2 resource-merging step to
     * generate it. Traced (via {@code javap} on {@code WindowInsetsCompat.class})
     * to exactly where the two-arg overload touches {@code ViewCompat}: only inside
     * `if (view != null && view.isAttachedToWindow())`, to populate "root window
     * insets" — a feature `mSoftKeyboardActive`'s own visibility check doesn't need
     * (that reads the insets object's own `isVisible(Type.ime())`, independent of
     * root-insets tracking). The one-arg overload (`toWindowInsetsCompat(insets)`)
     * is literally the two-arg one called with `view=null` — same conversion,
     * `ViewCompat` never touched — so it's the correct fix here, not a workaround:
     * this integration doesn't want the root-insets feature at all. */
    void forwardWindowInsets(WindowInsets insets) {
        if (gameTextInputConnection != null) {
            gameTextInputConnection.onApplyWindowInsets(this, WindowInsetsCompat.toWindowInsetsCompat(insets));
        }
    }

    /** Tells the input method framework this view supports text editing at all.
     * Without it, the system has no particular reason to ever call
     * {@link #onCreateInputConnection} just because this view has focus — this is
     * the standard signal a real text-editing view (e.g. {@code EditText}) gives. */
    @Override
    public boolean onCheckIsTextEditor() {
        return true;
    }

    @Override
    public android.view.inputmethod.InputConnection onCreateInputConnection(EditorInfo outAttrs) {
        // `Settings` wants an `EditorInfo` it owns (see vendor/gametextinput's own
        // InputConnection constructor) — not `outAttrs`, which is what
        // GameTextInput.copyEditorInfo below actually fills in for Android.
        // `pendingInputTypeSpec` (../../PLAN.md §5/§7.5) is whatever
        // `focusAndShowKeyboard` set right before whatever triggered *this* call
        // (first-ever focus, or a `restartInput` for a field whose type changed) —
        // see `androidInputTypeFor`'s own doc comment for the actual mapping.
        EditorInfo settingsInfo = new EditorInfo();
        settingsInfo.inputType = androidInputTypeFor(pendingInputTypeSpec);
        settingsInfo.imeOptions = EditorInfo.IME_ACTION_DONE;
        Settings settings = new Settings(settingsInfo, /* forwardKeyEvents= */ true);
        gameTextInputConnection = new InputConnection(getContext(), this, settings);
        gameTextInputConnection.setListener(this);
        currentInputTypeSpec = pendingInputTypeSpec;
        GameTextInput.copyEditorInfo(gameTextInputConnection.getEditorInfo(), outAttrs);
        Native.onInputConnectionCreated(gameTextInputConnection);
        Log.i("ZKiteGpuHarness", "onCreateInputConnection: built " + gameTextInputConnection
                + " for type=" + currentInputTypeSpec);
        return gameTextInputConnection;
    }

    /** Maps a `zkite_py` `TextField`'s `input_type` attr (../../PLAN.md §5/§7.5 —
     * `"password"` when `secure=True`, else whatever `type=` was given, defaulting
     * to `"text"` — see `TextField._tree_attrs`, `phase_1/zkite_py/src/zkite/
     * widgets.py`) to a real Android `InputType` flag combination, mirroring what
     * an HTML {@code <input type=...>} of the same name gets for free on the
     * WebView backend. Unknown specs fall back to plain `TYPE_CLASS_TEXT` — the
     * same behavior every field got before this mapping existed — rather than
     * failing closed, since `type=` is a free-form string on this widget (nothing
     * validates it against a closed set), same as real HTML. */
    private static int androidInputTypeFor(String spec) {
        if ("password".equals(spec)) {
            return InputType.TYPE_CLASS_TEXT | InputType.TYPE_TEXT_VARIATION_PASSWORD;
        } else if ("email".equals(spec)) {
            return InputType.TYPE_CLASS_TEXT | InputType.TYPE_TEXT_VARIATION_EMAIL_ADDRESS;
        } else if ("url".equals(spec)) {
            return InputType.TYPE_CLASS_TEXT | InputType.TYPE_TEXT_VARIATION_URI;
        } else if ("tel".equals(spec) || "phone".equals(spec)) {
            return InputType.TYPE_CLASS_PHONE;
        } else if ("number".equals(spec)) {
            return InputType.TYPE_CLASS_NUMBER;
        } else if ("search".equals(spec)) {
            return InputType.TYPE_CLASS_TEXT;
        }
        return InputType.TYPE_CLASS_TEXT;
    }

    /** Called by native ({@code render_thread.rs}'s {@code call_surface_view_method})
     * when a tap lands on a {@code TextField} — requests view focus and asks the
     * system to show the keyboard, which is what actually triggers
     * {@link #onCreateInputConnection} on first focus (gaining view focus alone
     * doesn't reliably do it).
     *
     * <p><b>Real bug found on-device, not by inspection:</b> the render thread calls
     * this via a plain JNI instance-method call from its own attached env — no
     * {@code FindClass}-by-name involved (see {@code text_input.rs}'s module doc for
     * that *different*, already-fixed thread issue), so the call itself always
     * reached this method. But {@code requestFocus()} threw
     * {@code CalledFromWrongThreadException} every time — Android's own
     * {@code ViewRootImpl} asserts view methods run on the UI thread, a rule with
     * nothing to do with JNI, and it applies just as much to a JNI call from a
     * background thread as to ordinary background-thread Java code. The exception
     * was silently swallowed at the JNI boundary (a pending Java exception after a
     * JNI call doesn't crash the native caller by itself) until this method's own
     * logging plus an explicit {@code ExceptionCheck} on the Rust side surfaced it.
     * Fixed the standard way: if not already on the UI thread, {@link #post} this
     * same call and return — {@code View.post} queues a {@code Runnable} on the
     * view's own {@code Handler}, which is always bound to the UI thread's
     * {@code Looper}. See ../../PLAN.md §5.
     *
     * <p><b>A second real bug, found the same way (a real device, not inspection):
     * </b> even after the thread-hop fix, {@code Listener.stateChanged} never fired
     * on a real edit — confirmed live: {@code gti.InputConnection}'s own debug log
     * showed real {@code commitText}/{@code setComposingText} calls happening
     * correctly on every keystroke, but nothing reached
     * {@code Native.onTextInputEvent}. Traced (via {@code javap} against the
     * vendored class, since gametextinput ships no source) to
     * {@code InputConnection.stateUpdated}: it only calls the listener when its own
     * private {@code mSoftKeyboardActive} flag is {@code true} — and the *only*
     * setter for that flag is {@code InputConnection.setSoftKeyboardActive}, which
     * this method never called (it drove {@link InputMethodManager} directly
     * instead, which shows/hides the real keyboard but never touches that internal
     * flag). Fixed by calling {@code setSoftKeyboardActive} once
     * {@link #gameTextInputConnection} exists — which also makes the manual
     * {@code requestFocus}/{@code showSoftInput} pair below redundant on every call
     * *after* the first (that method does the same two calls itself), so they're
     * skipped once a connection already exists.
     *
     * <p><b>{@code inputTypeSpec} (../../PLAN.md §5/§7.5)</b> is the about-to-be-
     * focused field's own `input_type` (`"text"`/`"password"`/`"email"`/... —
     * {@code render_thread.rs}'s {@code focus_text_field} reads it straight off the
     * `zkite_py` tree). Stored in {@link #pendingInputTypeSpec} for
     * {@link #onCreateInputConnection} to build its `EditorInfo` from. **Switching
     * focus between two already-visited fields of different types is the one case
     * that needs more than just re-showing the existing connection**: Android only
     * calls `onCreateInputConnection` again on its own initiative (first focus, or
     * after the view loses and regains it) — merely changing which field this
     * class considers "focused" doesn't, so without an explicit nudge a password
     * field focused after a plain-text one would keep the *first* field's
     * `EditorInfo` (wrong `inputType`, stale `Editable` reference) for the rest of
     * the session. `InputMethodManager.restartInput(View)` is the documented way to
     * force that nudge on an already-focused view: it forces the current input
     * session to end and a fresh one to begin, calling `onCreateInputConnection`
     * again and reporting the new connection to the IME — the same technique other
     * custom-rendered text stacks (e.g. Flutter's) use for exactly this. */
    private static final class FocusAndShowKeyboardRunnable implements Runnable {
        private final ZKiteSurfaceView view;
        private final String inputTypeSpec;

        FocusAndShowKeyboardRunnable(ZKiteSurfaceView view, String inputTypeSpec) {
            this.view = view;
            this.inputTypeSpec = inputTypeSpec;
        }

        @Override
        public void run() {
            view.focusAndShowKeyboard(inputTypeSpec);
        }
    }

    public void focusAndShowKeyboard(final String inputTypeSpec) {
        if (getHandler() != null && Thread.currentThread() != getHandler().getLooper().getThread()) {
            post(new FocusAndShowKeyboardRunnable(this, inputTypeSpec));
            return;
        }
        pendingInputTypeSpec = inputTypeSpec;
        if (gameTextInputConnection != null) {
            if (!inputTypeSpec.equals(currentInputTypeSpec)) {
                InputMethodManager imm =
                        (InputMethodManager) getContext().getSystemService(Context.INPUT_METHOD_SERVICE);
                if (imm != null) {
                    imm.restartInput(this);
                }
            }
            gameTextInputConnection.setSoftKeyboardActive(true, InputMethodManager.SHOW_IMPLICIT);
            return;
        }
        // First-ever focus: no InputConnection exists yet to call
        // setSoftKeyboardActive on. requestFocus()+showSoftInput() is what
        // synchronously drives onCreateInputConnection (see that method's own doc
        // comment) — confirmed live via the real call stack this produces
        // (showSoftInput -> checkFocus -> startInputInner -> createInputConnection
        // -> onCreateInputConnection, all on this thread, all before showSoftInput
        // returns).
        requestFocus();
        InputMethodManager imm = (InputMethodManager) getContext().getSystemService(Context.INPUT_METHOD_SERVICE);
        if (imm != null) {
            imm.showSoftInput(this, InputMethodManager.SHOW_IMPLICIT);
        }
        if (gameTextInputConnection != null) {
            gameTextInputConnection.setSoftKeyboardActive(true, InputMethodManager.SHOW_IMPLICIT);
        }
    }

    /** Called by native when focus moves off every {@code TextField} (a tap
     * elsewhere, per {@code render_thread.rs}'s {@code dispatch_tap}) — hides the
     * keyboard and drops view focus. Same UI-thread hop as
     * {@link #focusAndShowKeyboard} and for the identical reason
     * ({@code clearFocus()} has the same thread assertion as {@code requestFocus()}). */
    private static final class BlurAndHideKeyboardRunnable implements Runnable {
        private final ZKiteSurfaceView view;

        BlurAndHideKeyboardRunnable(ZKiteSurfaceView view) {
            this.view = view;
        }

        @Override
        public void run() {
            view.blurAndHideKeyboard();
        }
    }

    public void blurAndHideKeyboard() {
        if (getHandler() != null && Thread.currentThread() != getHandler().getLooper().getThread()) {
            post(new BlurAndHideKeyboardRunnable(this));
            return;
        }
        if (gameTextInputConnection != null) {
            // Also clears mSoftKeyboardActive — see focusAndShowKeyboard's own doc
            // comment on why that flag matters beyond just hiding the keyboard.
            gameTextInputConnection.setSoftKeyboardActive(false, 0);
        } else {
            InputMethodManager imm = (InputMethodManager) getContext().getSystemService(Context.INPUT_METHOD_SERVICE);
            if (imm != null) {
                imm.hideSoftInputFromWindow(getWindowToken(), 0);
            }
        }
        clearFocus();
    }

    /**
     * Called by native when a Back press reached {@code zkite_py} and nothing wanted
     * it (../../../../PLAN.md §16) — finishes the Activity, which is the platform
     * default this path exists to preserve.
     *
     * <p>Same UI-thread hop as {@link #focusAndShowKeyboard}/
     * {@link #blurAndHideKeyboard} and for the same reason ({@code Activity.finish()}
     * must run on the UI thread, and a JNI call from the render thread is still a
     * background-thread call as far as the framework is concerned — see
     * ../../../../PLAN.md §7.3 item 2, which is where this class learned that the
     * hard way).
     *
     * <p>Reached via {@code getContext()} rather than a stored {@code Activity}
     * reference deliberately: a {@code SurfaceView}'s context <em>is</em> its hosting
     * Activity here, and holding a second strong reference to it from a
     * native-owned global ref is exactly the shape of an Activity leak across a
     * rotation. The {@code instanceof} guard is not defensive padding — a
     * {@code ContextWrapper} is a legitimate thing to be constructed with, and
     * finishing nothing is better than casting blindly.
     */
    private static final class FinishActivityRunnable implements Runnable {
        private final ZKiteSurfaceView view;

        FinishActivityRunnable(ZKiteSurfaceView view) {
            this.view = view;
        }

        @Override
        public void run() {
            view.finishActivity();
        }
    }

    public void finishActivity() {
        if (getHandler() != null && Thread.currentThread() != getHandler().getLooper().getThread()) {
            post(new FinishActivityRunnable(this));
            return;
        }
        Context ctx = getContext();
        if (ctx instanceof Activity) {
            ((Activity) ctx).finish();
        } else {
            Log.w("ZKiteGpuHarness", "finishActivity: context is not an Activity, ignoring");
        }
    }

    /** Called by native ({@code render_thread.rs}'s {@code set_cursor_position}) when
     * a tap lands inside the (now-)focused field's own text — pushes a new
     * {@code [offset, offset)} selection into the real {@code Editable} so the IME's
     * own idea of the cursor matches where the user actually tapped, not just
     * wherever the last commit left it. See ../../PLAN.md §7.7.
     *
     * <p><b>{@code text} must be the field's real, current, full text.</b>
     * {@code gti.InputConnection.setState} (traced via {@code javap} — no source
     * ships) clears its {@code Editable} and reinserts {@code text} from scratch on
     * every call; passing anything other than what's actually there right now
     * silently overwrites it. `render_thread.rs` always passes its own live
     * `focused_text`, which is kept in sync with every real edit via
     * {@code Native.onTextInputEvent} — never a value computed independently here.
     *
     * <p>Same UI-thread hop as {@link #focusAndShowKeyboard}/
     * {@link #blurAndHideKeyboard}, for the identical reason — this ends up calling
     * into {@code GameTextInput}, which is confined to the UI thread; see
     * {@code text_input.rs}'s module doc. */
    private static final class SetSelectionRunnable implements Runnable {
        private final ZKiteSurfaceView view;
        private final String text;
        private final int start;
        private final int end;

        SetSelectionRunnable(ZKiteSurfaceView view, String text, int start, int end) {
            this.view = view;
            this.text = text;
            this.start = start;
            this.end = end;
        }

        @Override
        public void run() {
            view.setSelection(text, start, end);
        }
    }

    public void setSelection(final String text, final int start, final int end) {
        if (getHandler() != null && Thread.currentThread() != getHandler().getLooper().getThread()) {
            post(new SetSelectionRunnable(this, text, start, end));
            return;
        }
        Native.setSelection(text, start, end);
    }

    /** Lazily created on first use, not in the constructor (../../PLAN.md §7.8):
     * {@link TextSelectionOverlay} needs this view's own parent {@code ViewGroup} to
     * add its {@code EditText} as a sibling, and a view has no parent yet at
     * construction time — {@code ZKiteActivity} adds this view to the window's
     * content {@code FrameLayout} only *after* constructing it. By the time a tap
     * could ever long-press a {@code TextField}, {@code onAttachedToWindow} has long
     * since fired, so {@code getParent()} is always real here. */
    private TextSelectionOverlay selectionOverlay;

    private TextSelectionOverlay selectionOverlay() {
        if (selectionOverlay == null) {
            selectionOverlay = new TextSelectionOverlay(this);
            // Created lazily, on the first long-press — long after the insets that
            // positioned this view arrived (../../PLAN.md §7.14). Seed it with the
            // current offset rather than waiting for the *next* insets dispatch,
            // which may never come.
            selectionOverlay.setParentOffset(systemBarInsetLeft, systemBarInsetTop);
        }
        return selectionOverlay;
    }

    /** Called by native ({@code render_thread.rs}'s {@code call_show_selection_
     * overlay_method}) after a long-press hit-tests to a word inside the focused
     * field — see ../../PLAN.md §7.8. {@code x}/{@code y}/{@code w}/{@code h} are
     * the field's *container* rect (not just its text), already in this view's own
     * pixel space, matching {@link #onTouchDown} etc.; {@code textColor}/
     * {@code bgColor} are packed {@code 0xAARRGGBB} ints. {@code isPassword}
     * (../../PLAN.md §5/§7.5) is the field's own `secure=True` — see
     * {@code TextSelectionOverlay.show}'s own doc comment for what it does with
     * it. Same UI-thread hop as every other native-called method here —
     * {@code TextSelectionOverlay} ends up touching real {@code View}s, which
     * carry the identical thread assertion {@link #focusAndShowKeyboard} already
     * documents. */
    private static final class ShowSelectionOverlayRunnable implements Runnable {
        private final ZKiteSurfaceView view;
        private final String text;
        private final int selStart;
        private final int selEnd;
        private final float x;
        private final float y;
        private final float w;
        private final float h;
        private final float textSizePx;
        private final int textColor;
        private final int bgColor;
        private final boolean isPassword;

        ShowSelectionOverlayRunnable(ZKiteSurfaceView view, String text, int selStart, int selEnd,
                float x, float y, float w, float h, float textSizePx, int textColor, int bgColor,
                boolean isPassword) {
            this.view = view;
            this.text = text;
            this.selStart = selStart;
            this.selEnd = selEnd;
            this.x = x;
            this.y = y;
            this.w = w;
            this.h = h;
            this.textSizePx = textSizePx;
            this.textColor = textColor;
            this.bgColor = bgColor;
            this.isPassword = isPassword;
        }

        @Override
        public void run() {
            view.showSelectionOverlay(text, selStart, selEnd, x, y, w, h, textSizePx, textColor, bgColor,
                    isPassword);
        }
    }

    public void showSelectionOverlay(final String text, final int selStart, final int selEnd, final float x,
            final float y, final float w, final float h, final float textSizePx, final int textColor,
            final int bgColor, final boolean isPassword) {
        if (getHandler() != null && Thread.currentThread() != getHandler().getLooper().getThread()) {
            post(new ShowSelectionOverlayRunnable(this, text, selStart, selEnd, x, y, w, h, textSizePx,
                    textColor, bgColor, isPassword));
            return;
        }
        selectionOverlay().show(text, selStart, selEnd, x, y, w, h, textSizePx, textColor, bgColor, isPassword);
    }

    /** Called by native ({@code render_thread.rs}'s {@code blur_text_field}) when a
     * tap lands somewhere the currently-open selection overlay doesn't cover — see
     * ../../PLAN.md §7.8. A no-op (both here and inside {@code TextSelectionOverlay}
     * itself) if nothing is currently showing; safe to call unconditionally. */
    private static final class HideSelectionOverlayRunnable implements Runnable {
        private final ZKiteSurfaceView view;

        HideSelectionOverlayRunnable(ZKiteSurfaceView view) {
            this.view = view;
        }

        @Override
        public void run() {
            view.hideSelectionOverlay();
        }
    }

    public void hideSelectionOverlay() {
        if (getHandler() != null && Thread.currentThread() != getHandler().getLooper().getThread()) {
            post(new HideSelectionOverlayRunnable(this));
            return;
        }
        if (selectionOverlay != null) {
            selectionOverlay.hide();
        }
    }

    // ---- com.google.androidgamesdk.gametextinput.Listener ----------------------
    // Called by `gameTextInputConnection` itself whenever the IME edits the field —
    // see vendor/gametextinput/classes.jar's `InputConnection`, which invokes these
    // internally on every commit/compose/delete. Native.onTextInputEvent forwards
    // into `GameTextInput_processEvent` (text_input.rs), which is what actually
    // decodes `state` — this method never reads it directly.

    @Override
    public void stateChanged(State state, boolean dismissedByImeAction) {
        Log.i("ZKiteGpuHarness", "stateChanged: text=" + state.text + " dismissed=" + dismissedByImeAction);
        Native.onTextInputEvent(state);
        if (dismissedByImeAction) {
            Native.onKeyboardDismissed();
        }
    }

    /** Forwards the IME's own real bottom inset (../../PLAN.md §5/§7.5) — the
     * height, in pixels, of whatever's currently covering the bottom of this view
     * (the keyboard, when shown; zero once it's gone). `render_thread.rs` uses this
     * to pan the focused field above the keyboard when it would otherwise be
     * covered — see that file's `redraw`/`node_scroll_offsets` for the actual
     * mechanism (it reuses the existing per-node scroll-offset machinery, keyed on
     * the tree's own root, rather than a bespoke transform). Called by
     * `gametextinput.InputConnection` itself, driven by the same real
     * {@code WindowInsetsCompat} delivery that {@link #forwardWindowInsets} already
     * forwards to it for {@code mSoftKeyboardActive} — no separate insets source
     * needed. */
    @Override
    public void onImeInsetsChanged(Insets insets) {
        // `insets.bottom` is measured against the *window*, but everything
        // `render_thread.rs` computes (including `compute_ime_pan`, which compares
        // it against `self.height`) is in this view's own surface space — and as of
        // ../../PLAN.md §7.14 the view is inset from the window bottom by the
        // navigation bar. Converting here, at the one place that knows both
        // numbers, keeps the native side working in a single coordinate space:
        // without it the pan over-shoots by exactly the nav-bar height.
        Native.onImeInsetsChanged(Math.max(0, insets.bottom - systemBarInsetBottom));
    }

    /**
     * The system-bar insets currently applied to this view as layout margins by
     * {@code ZKiteActivity.applyInsetsAsMargins} (../../PLAN.md §7.14). Tracked here
     * for two consumers that need to convert between window space and this view's
     * own surface space: {@link #onImeInsetsChanged} (above) and the selection
     * overlay, which is a sibling in the same full-window parent and so must be
     * offset by the same margins to line up with the field it covers.
     */
    void onSystemBarInsetsChanged(int left, int top, int right, int bottom) {
        systemBarInsetLeft = left;
        systemBarInsetTop = top;
        systemBarInsetBottom = bottom;
        if (selectionOverlay != null) {
            selectionOverlay.setParentOffset(left, top);
        }
    }

    private int systemBarInsetLeft;
    private int systemBarInsetTop;
    private int systemBarInsetBottom;

    /**
     * Whether Android's accessibility framework is currently enabled — i.e. whether
     * it is legal, right now, to send an {@code AccessibilityEvent} at all
     * (../../PLAN.md §8.7).
     *
     * <p><b>This is not a nice-to-have optimization; without it the app hard-crashes.</b>
     * {@code AccessibilityManager.sendAccessibilityEvent} throws
     * {@code IllegalStateException("Accessibility off. Did you forget to check
     * that?")} when accessibility is disabled and the caller is on the main looper —
     * AOSP's own wording, and a direct instruction to call exactly this method first.
     * {@code accesskit_android}'s event-raising path
     * ({@code event.rs}'s {@code send_completed_event}) calls
     * {@code ViewParent.requestSendAccessibilityEvent}, which reaches that throw, and
     * {@code .unwrap()}s the result — so the exception becomes a Rust panic on the UI
     * thread, which {@code render_thread.rs}'s panic hook turns into a logged
     * {@code SIGABRT}: the whole process dies.
     *
     * <p>The window in which that happens is wider than "the user runs a screen
     * reader": <em>any</em> transient accessibility client — notably {@code adb shell
     * uiautomator dump}, which every prior session in ../../PLAN.md §8.4 used to
     * verify the a11y tree — flips accessibility on, which moves AccessKit's own
     * adapter from {@code Placeholder} to {@code Active}, and then disconnects,
     * leaving accessibility off but the adapter still {@code Active} and still
     * raising events on every subsequent tree update. Confirmed by a live A/B on a
     * real device (see ../../PLAN.md §8.7): identical tap, accessibility on → works;
     * accessibility off → SIGABRT.
     */
    public boolean isAccessibilityEnabled() {
        android.view.accessibility.AccessibilityManager am =
                (android.view.accessibility.AccessibilityManager)
                        getContext().getSystemService(Context.ACCESSIBILITY_SERVICE);
        return am != null && am.isEnabled();
    }

    @Override
    public void onSoftwareKeyboardVisibilityChanged(boolean visible) {
        if (!visible) {
            Native.onKeyboardDismissed();
        }
    }

    @Override
    public void onEditorAction(int actionId) {
        // Single-line fields only send IME_ACTION_DONE (see onCreateInputConnection)
        // — treated uniformly as "stop editing," not dispatched to a `zkite_py`
        // handler of its own (no `on_submit`-style event exists yet).
        Native.onKeyboardDismissed();
    }

    @Override
    public void surfaceCreated(SurfaceHolder holder) {
        // Real dimensions arrive in surfaceChanged, which Android guarantees fires at
        // least once immediately after surfaceCreated — but Native.onSurfaceCreated
        // needs *some* size to hand the render thread a window at all, so pass what
        // the holder already reports (usually correct) and let the surfaceChanged
        // call true it up.
        android.view.Surface surface = holder.getSurface();
        if (surface != null && surface.isValid()) {
            int w = getWidth();
            int h = getHeight();
            Native.onSurfaceCreated(surface, w, h);
        }
    }

    @Override
    public void surfaceChanged(SurfaceHolder holder, int format, int width, int height) {
        android.view.Surface surface = holder.getSurface();
        if (surface != null && surface.isValid()) {
            Native.onSurfaceChanged(width, height);
        }
    }

    @Override
    public void surfaceDestroyed(SurfaceHolder holder) {
        // Must block until Rust has actually dropped the ANativeWindow — see
        // Native.onSurfaceDestroyed's doc and render_thread.rs's DestroyWindow
        // handling. Android's own contract for this callback requires exactly this.
        Native.onSurfaceDestroyed();
    }

    @Override
    public boolean onTouchEvent(MotionEvent event) {
        // Every down/move/up crosses the JNI boundary raw — see the class doc.
        // `ACTION_MOVE` can (and does, per Android's own docs) batch multiple
        // historical points per event on a high-report-rate touchscreen; forwarding
        // only the final coordinate (getX/getY) rather than each historical sample is
        // a deliberate simplification — real drag distance is still tracked
        // correctly (`render_thread.rs` diffs successive calls), just at up to one
        // point per frame instead of the touch controller's full sample rate. A
        // future refinement, not a correctness gap: nothing currently needs
        // sub-frame drag precision.
        int action = event.getActionMasked();
        float x = event.getX();
        float y = event.getY();
        Log.i("ZKiteGpuHarness", "onTouchEvent action=" + action + " x=" + x + " y=" + y);
        switch (action) {
            case MotionEvent.ACTION_DOWN:
                Native.onTouchDown(x, y);
                return true;
            case MotionEvent.ACTION_MOVE:
                Native.onTouchMove(x, y);
                return true;
            case MotionEvent.ACTION_UP:
                Native.onTouchUp(x, y);
                return true;
            case MotionEvent.ACTION_CANCEL:
                // Android is explicitly saying this gesture didn't complete normally
                // (e.g. a parent intercepted it) — must never be treated as "maybe a
                // tap" the way a real ACTION_UP is, so this is its own JNI call, not
                // onTouchUp with the coordinates ignored.
                Native.onTouchCancel();
                return true;
            default:
                return true;
        }
    }
}
