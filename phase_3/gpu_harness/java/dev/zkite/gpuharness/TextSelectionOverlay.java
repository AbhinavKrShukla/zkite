package dev.zkite.gpuharness;

import android.content.Context;
import android.graphics.drawable.ColorDrawable;
import android.text.Editable;
import android.text.InputType;
import android.text.TextWatcher;
import android.util.Log;
import android.util.TypedValue;
import android.view.KeyEvent;
import android.view.View;
import android.view.ViewGroup;
import android.view.inputmethod.EditorInfo;
import android.view.inputmethod.InputMethodManager;
import android.widget.EditText;
import android.widget.FrameLayout;
import android.widget.TextView;

/**
 * A real, fully visible {@code android.widget.EditText} laid on top of a focused
 * {@code TextField}'s own screen rect, for the duration of a selection gesture only
 * (../../PLAN.md §7.8) — the "native-EditText-overlay" approach chosen over
 * hand-building Flutter-style selection handles/copy-paste UI from scratch. Ordinary
 * tap-to-type editing never touches this class at all; it stays on the already-working
 * {@code GameTextInput}-driven path (../../PLAN.md §7.7). This class only enters the
 * picture on a long-press (`render_thread.rs`'s `dispatch_long_press`), and owns the
 * rest of that focus session once it does: real Android selection handles, a real
 * cut/copy/paste toolbar, and real system-clipboard integration, all for free from the
 * platform. Deliberately not trying to visually fuse with the GPU-rendered text engine
 * underneath (parley/vello) — matching the user's own stated risk tolerance going in
 * (see the PLAN.md section this implements): a brief switch to Android's native text
 * rendering during a deliberate, bounded "select text" interaction is an acceptable
 * trade for not hand-rolling selection UI, revisit only if that turns out to matter
 * more in practice than expected.
 *
 * <p><b>Why fully visible instead of a transparent/invisible overlay mirroring the GPU
 * text underneath:</b> handle/toolbar positions are driven by this {@code EditText}'s
 * *own* internal text layout, which uses a different shaping engine (Android's
 * {@code Layout}) than the GPU path ({@code parley}) — the two would only ever be
 * approximately aligned, never exactly. Rather than chase pixel-perfect agreement
 * between two independent text engines for glyphs nobody is meant to look at twice,
 * this overlay is simply styled to *resemble* the field (background/text color/size —
 * see {@link #show}) and shown opaque, fully occluding the GPU-painted field
 * underneath for as long as it's up. No double-image is possible by construction: the
 * overlay's rect always matches the field's own container rect exactly.
 *
 * <p>No lambdas / method references, same {@code javac -source 8} constraint as every
 * other Java file in this codebase — see {@code ZKiteActivity}'s class doc.
 */
final class TextSelectionOverlay {
    private static final String TAG = "ZKiteGpuHarness";

    private final ZKiteSurfaceView surfaceView;
    private final EditText editText;
    /** Guards `reportChange`/`reportChangeAndHide` against a `TextWatcher`/
     * `onSelectionChanged` firing for a change *this* class made to itself (seeding
     * text/selection in {@link #show}), not the user — those aren't real edits. */
    private boolean suppressCallbacks;
    /** The authority on "is this currently showing," checked and cleared *before*
     * touching {@link #editText} in {@link #reportChangeAndHide} — not read back
     * from {@code editText.getVisibility()}, because setting a focused view's
     * visibility to {@code GONE} can itself synchronously re-enter this same method
     * via the focus-change listener below (Android clears focus from a view that
     * becomes invisible), and a flag set *first* is what makes that re-entrant call
     * a harmless no-op instead of a duplicate dismiss report to native. */
    private boolean visible;

    private static final class CustomEditText extends EditText {
        private final TextSelectionOverlay overlay;

        CustomEditText(Context context, TextSelectionOverlay overlay) {
            super(context);
            this.overlay = overlay;
        }

        @Override
        protected void onSelectionChanged(int selStart, int selEnd) {
            super.onSelectionChanged(selStart, selEnd);
            // TextView's constructor can call this override before our field is
            // assigned. The overlay itself is also still being constructed then.
            if (overlay != null) {
                overlay.reportChange();
            }
        }
    }

    private static final class OverlayTextWatcher implements TextWatcher {
        private final TextSelectionOverlay overlay;

        OverlayTextWatcher(TextSelectionOverlay overlay) {
            this.overlay = overlay;
        }

        @Override
        public void beforeTextChanged(CharSequence s, int start, int count, int after) {}

        @Override
        public void onTextChanged(CharSequence s, int start, int before, int count) {}

        @Override
        public void afterTextChanged(Editable s) {
            overlay.reportChange();
        }
    }

    private static final class EditorActionListener implements TextView.OnEditorActionListener {
        private final TextSelectionOverlay overlay;

        EditorActionListener(TextSelectionOverlay overlay) {
            this.overlay = overlay;
        }

        @Override
        public boolean onEditorAction(TextView v, int actionId, KeyEvent event) {
            overlay.reportChangeAndHide();
            return false;
        }
    }

    private static final class FocusChangeListener implements View.OnFocusChangeListener {
        private final TextSelectionOverlay overlay;

        FocusChangeListener(TextSelectionOverlay overlay) {
            this.overlay = overlay;
        }

        @Override
        public void onFocusChange(View v, boolean hasFocus) {
            if (!hasFocus) {
                overlay.reportChangeAndHide();
            }
        }
    }

    TextSelectionOverlay(ZKiteSurfaceView surfaceView) {
        this.surfaceView = surfaceView;
        this.editText = new CustomEditText(surfaceView.getContext(), this);
        editText.setVisibility(View.GONE);
        editText.setSingleLine(true);
        editText.setIncludeFontPadding(false);
        editText.setImeOptions(EditorInfo.IME_ACTION_DONE);
        editText.addTextChangedListener(new OverlayTextWatcher(this));
        editText.setOnEditorActionListener(new EditorActionListener(this));
        editText.setOnFocusChangeListener(new FocusChangeListener(this));

        ViewGroup parent = (ViewGroup) surfaceView.getParent();
        if (parent == null) {
            // Shouldn't happen — see this field's own doc comment on why a real
            // parent is expected to always exist by the time anything ever
            // constructs a TextSelectionOverlay at all. Logged, not fatal: `show`
            // re-checks and no-ops the same way if this somehow still is null then.
            Log.e(TAG, "TextSelectionOverlay: surfaceView has no parent yet");
        } else {
            parent.addView(editText, new FrameLayout.LayoutParams(0, 0));
        }
    }

    /**
     * How far the surface view is offset inside the shared parent by the system-bar
     * insets (../../PLAN.md §7.14) — see {@link #show}, which adds this to every
     * position it is given, since those arrive in the surface's own coordinate space
     * rather than the parent's. Kept in sync by
     * {@code ZKiteSurfaceView.onSystemBarInsetsChanged}, and seeded at construction
     * for the (normal) case where this object is created long after the insets that
     * positioned the surface already arrived.
     */
    private int parentOffsetX;
    private int parentOffsetY;

    void setParentOffset(int x, int y) {
        parentOffsetX = x;
        parentOffsetY = y;
    }

    /** Forwards the overlay's current text/selection to native — every keystroke, a
     * dragged handle, or a cut/paste from its native toolbar. Not called while
     * {@link #suppressCallbacks} (this class seeding its own state in {@link #show},
     * not a real user edit). */
    private void reportChange() {
        if (suppressCallbacks || !visible) {
            return;
        }
        Native.onSelectionOverlayChanged(editText.getText().toString(), editText.getSelectionStart(),
                editText.getSelectionEnd());
    }

    /** The one path every dismiss trigger (focus loss, the IME's "done" action, or
     * an explicit {@link #hide} from native) funnels through — see {@link #visible}'s
     * own doc comment on why it, not {@code getVisibility()}, is what guards
     * re-entrancy here. */
    private void reportChangeAndHide() {
        if (!visible) {
            return;
        }
        visible = false;
        String text = editText.getText().toString();
        int start = editText.getSelectionStart();
        int end = editText.getSelectionEnd();
        editText.setVisibility(View.GONE);
        Native.onSelectionOverlayDismissed(text, start, end);
    }

    /**
     * Shows the overlay over `[x, y, w, h]` (the field's own container rect, already
     * in this view's pixel space — see {@code ZKiteSurfaceView.showSelectionOverlay}'s
     * own doc comment), pre-filled with `text` and `[selStart, selEnd)` selected,
     * styled to resemble the field it's covering (see this class's own doc comment
     * on why "resemble," not "pixel-match"), and requests real Android focus so its
     * native selection handles/toolbar take over.
     *
     * <p><b>{@code isPassword} (../../PLAN.md §5/§7.5):</b> sets a real
     * {@code TYPE_TEXT_VARIATION_PASSWORD} on this {@code EditText} so Android's
     * own {@code PasswordTransformationMethod} masks the text and its stock
     * {@code Editor} hides Cut/Copy from the toolbar for a real selection (both for
     * free, the same behavior any other password `EditText` gets) — `text` itself
     * is still the real, unmasked value (see this method's own caller,
     * {@code render_thread.rs}'s {@code show_selection_overlay}, for why pre-
     * masking it here instead would be wrong). Reset on *every* call, not just when
     * `true` — this one `EditText` is reused across every field's own selection
     * session (see {@link #editText}'s construction, lazy-created once), so a
     * plain-text field shown after a password one must explicitly clear whatever
     * `inputType` the previous session left behind, not inherit it.
     */
    void show(String text, int selStart, int selEnd, float x, float y, float w, float h, float textSizePx,
            int textColor, int bgColor, boolean isPassword) {
        ViewGroup parent = (ViewGroup) surfaceView.getParent();
        if (parent == null || editText.getParent() == null) {
            Log.e(TAG, "showSelectionOverlay: overlay was never attached to a parent");
            return;
        }
        FrameLayout.LayoutParams lp = (FrameLayout.LayoutParams) editText.getLayoutParams();
        lp.width = Math.round(w);
        lp.height = Math.round(h);
        // `x`/`y` are in the *surface's* coordinate space, but this EditText is a
        // sibling of the surface view in a parent that spans the whole window, and
        // the surface view is itself offset within that parent by the system-bar
        // insets (../../PLAN.md §7.14). Without adding that offset back the overlay
        // would sit exactly one status-bar height above the field it is supposed to
        // be covering.
        lp.leftMargin = Math.round(x) + parentOffsetX;
        lp.topMargin = Math.round(y) + parentOffsetY;
        editText.setLayoutParams(lp);
        editText.setBackground(new ColorDrawable(bgColor));
        editText.setTextColor(textColor);
        editText.setTextSize(TypedValue.COMPLEX_UNIT_PX, textSizePx);
        editText.setPadding(0, 0, 0, 0);
        editText.setInputType(isPassword
                ? (InputType.TYPE_CLASS_TEXT | InputType.TYPE_TEXT_VARIATION_PASSWORD)
                : InputType.TYPE_CLASS_TEXT);

        suppressCallbacks = true;
        editText.setText(text);
        int len = text.length();
        editText.setSelection(clamp(selStart, len), clamp(selEnd, len));
        suppressCallbacks = false;

        visible = true;
        editText.setVisibility(View.VISIBLE);
        editText.requestFocus();
        // `editText.performLongClick()` here was tried this session as a possible
        // fix for the "two long-presses" gap (../../PLAN.md §7.8's own known-gaps
        // note) — confirmed live, not assumed, that it does *not* bring up the
        // floating toolbar (handles/selection already show from `setSelection`
        // above regardless). Removed again since it did nothing; the finding
        // itself is recorded in PLAN.md rather than left as dead code here.
        InputMethodManager imm =
                (InputMethodManager) surfaceView.getContext().getSystemService(Context.INPUT_METHOD_SERVICE);
        if (imm != null) {
            imm.showSoftInput(editText, InputMethodManager.SHOW_IMPLICIT);
        }
    }

    /** Called by native (`ZKiteSurfaceView.hideSelectionOverlay`) when a tap lands
     * somewhere this overlay doesn't cover — see {@code render_thread.rs}'s
     * {@code blur_text_field}. Idempotent: a no-op if nothing is currently showing. */
    void hide() {
        reportChangeAndHide();
    }

    private static int clamp(int v, int len) {
        if (v < 0) {
            return 0;
        }
        if (v > len) {
            return len;
        }
        return v;
    }
}
