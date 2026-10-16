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

/** Native selection handles, toolbar and clipboard for a focused TextField.
 * Geometry and bundled typography come from the renderer, in physical pixels.
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
            int textColor, int bgColor, boolean isPassword, String geometry) {
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
        editText.setInputType(isPassword
                ? (InputType.TYPE_CLASS_TEXT | InputType.TYPE_TEXT_VARIATION_PASSWORD)
                : InputType.TYPE_CLASS_TEXT);
        editText.setTextColor(textColor);
        editText.setTextSize(TypedValue.COMPLEX_UNIT_PX, textSizePx);
        editText.setGravity(android.view.Gravity.TOP | android.view.Gravity.LEFT);
        try {
            org.json.JSONObject g = new org.json.JSONObject(geometry);
            String file = g.getString("font") + (g.getBoolean("bold") ? "-bold" : "") + ".ttf";
            android.graphics.Typeface face = android.graphics.Typeface.createFromFile(
                    new java.io.File(surfaceView.getContext().getFilesDir(), "fonts/" + file));
            editText.setTypeface(g.getBoolean("italic")
                    ? android.graphics.Typeface.create(face, android.graphics.Typeface.ITALIC) : face);
            editText.setLetterSpacing((float) g.getDouble("spacing"));
            editText.setPadding((int) Math.round(g.getDouble("left")),
                    Math.max(0, (int) Math.round(g.getDouble("baseline") + editText.getPaint().getFontMetrics().ascent)),
                    (int) Math.round(g.getDouble("right")), 0);
            org.json.JSONArray radii = g.getJSONArray("radii");
            float[] corners = new float[8];
            for (int i = 0; i < 4; i++) corners[2*i] = corners[2*i+1] = (float) radii.getDouble(i);
            android.graphics.drawable.GradientDrawable background = new android.graphics.drawable.GradientDrawable();
            background.setColor(bgColor);
            background.setCornerRadii(corners);
            editText.setBackground(background);
        } catch (Exception e) {
            Log.e(TAG, "Selection overlay style", e);
            editText.setBackground(new ColorDrawable(bgColor));
        }

        suppressCallbacks = true;
        editText.setText(text);
        int len = text.length();
        editText.setSelection(clamp(selStart, len), clamp(selEnd, len));
        suppressCallbacks = false;

        visible = true;
        editText.setVisibility(View.VISIBLE);
        editText.requestFocus();
        // Android starts its selection action mode through this public action.
        // Wait for layout and change the selection: an identical range is a no-op.
        final int selectionStart = clamp(selStart, text.length());
        final int selectionEnd = clamp(selEnd, text.length());
        editText.post(new Runnable() {
            @Override public void run() {
                if (!visible) return;
                suppressCallbacks = true;
                editText.setSelection(selectionEnd);
                android.os.Bundle args = new android.os.Bundle();
                args.putInt(android.view.accessibility.AccessibilityNodeInfo.ACTION_ARGUMENT_SELECTION_START_INT, selectionStart);
                args.putInt(android.view.accessibility.AccessibilityNodeInfo.ACTION_ARGUMENT_SELECTION_END_INT, selectionEnd);
                editText.performAccessibilityAction(android.view.accessibility.AccessibilityNodeInfo.ACTION_SET_SELECTION, args);
                suppressCallbacks = false;
            }
        });
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

    void updateRect(float x, float y, float w, float h) {
        if (!visible) return;
        FrameLayout.LayoutParams lp = (FrameLayout.LayoutParams) editText.getLayoutParams();
        int left = Math.round(x) + parentOffsetX, top = Math.round(y) + parentOffsetY;
        int width = Math.round(w), height = Math.round(h);
        if (lp.leftMargin == left && lp.topMargin == top && lp.width == width && lp.height == height) return;
        lp.leftMargin = left;
        lp.topMargin = top;
        lp.width = width;
        lp.height = height;
        editText.setLayoutParams(lp);
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
