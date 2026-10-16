//! Owns GPU (or CPU-fallback) rendering state on a dedicated OS thread, decoupled
//! from both the JNI calling thread (usually Android's UI thread) and from Java's
//! `SurfaceHolder.Callback` lifecycle. See ../PLAN.md §4.
//!
//! Threading model, and why: `phase_1/devshell`'s file-triggered reload used a
//! `Mutex<Option<String>>` polled every 120ms from the UI thread, deliberately, to
//! avoid async JNI machinery — a documented simplicity trade specific to that
//! situation (the "reactor" there is Java's UI thread, which can only be interrupted
//! by limited polling). That constraint doesn't apply here: this crate owns its own
//! dedicated thread outright, so a blocking channel recv is simpler *and* more
//! responsive than polling would be, not a deviation from that precedent so much as
//! the same "pick the simplest correct mechanism for the situation" reasoning
//! applied to a different situation.
//!
//! GPU-failure fallback (plan1.md §5/§9: "`vello_cpu` stays wired in as a runtime
//! fallback rather than an afterthought"): if `wgpu` can't find an adapter or create a
//! device, every draw goes through `vello_cpu` into a CPU pixel buffer, written
//! straight into the `ANativeWindow`'s own buffer via `ANativeWindow_lock` — no GPU
//! surface involved at all. `ZKITE_FORCE_CPU_RENDERER=1` forces this path even when a
//! GPU is available, so it's exercisable on hardware that happens to support Vulkan
//! (this session's device and emulator both do — see ../PLAN.md §4).
//!
//! Tree source (../PLAN.md §5): this thread also owns the embedded `zkite_py`
//! interpreter via `zkite_pybridge` — the same one-thread-owns-everything reasoning as
//! GPU/renderer state above, so there's no cross-thread GIL juggling to get wrong.
//! `self.current` is the last tree `zkite_pybridge` produced (from `setup` at boot,
//! from `dispatch` after a tap, or from `scroll` when a virtualised list needs
//! different rows); `self.built` is that tree's most recent `zkite_renderer` build,
//! rebuilt on every `redraw()` (resize included) from `self.current` alone — a resize
//! never re-invokes Python, since the app's *state* hasn't changed, only the viewport.
//!
//! Virtualised lists (../PLAN.md §11) are the one exception to that one-way flow, and
//! the reason `redraw()` can lay out twice: a `List` only builds the rows in view, so
//! it has to be *told* where its container is scrolled to before it can build
//! anything. `redraw()` lays out, reports every scrollable container's
//! `(scroll_y, viewport_height)` to `zkite_py` (`sync_viewports`), and lays out again
//! if that produced a new tree. Python re-runs `view()` only when a window genuinely
//! moved, so an ordinary scroll of an ordinary `ScrollView` still costs no user code
//! at all.
//!
//! Gestures (../PLAN.md §5): `ZKiteSurfaceView` forwards raw down/move/up events
//! (`Command::TouchDown`/`TouchMove`/`TouchUp`) rather than a single synthesized tap
//! the way this file used to — real disambiguation needs the whole gesture, not just
//! its endpoint. `TouchDown` resolves (once, for the gesture's whole lifetime) both
//! what would be tapped *and* the nearest scrollable ancestor **on each axis**
//! (../PLAN.md §27 — the two can be different containers, which is what a horizontal
//! strip inside a vertical list is); `TouchMove` decides tap-vs-drag by comparing
//! total movement against Android's own touch slop (passed down from
//! `ZKiteSurfaceView`'s `ViewConfiguration`, not a guessed constant), then either does
//! nothing yet (still within slop) or — at the instant slop is crossed — locks the
//! drag to one axis and updates `self.scroll_offsets` on it, redrawing; `TouchUp`
//! either dispatches a tap (never dragged) or hands the gesture to
//! `maybe_start_fling` (was a drag) — never both.
//! `self.scroll_offsets` is keyed by `zkite_py` node id, not taffy `NodeId`, because
//! the latter don't survive a rebuild (every redraw makes fresh ones) but scroll
//! position obviously must survive a redraw — see
//! `zkite_pybridge::BuiltTree::scroll_offsets_for`, which converts back to this
//! render's `NodeId`s each time it's needed.
//!
//! Momentum (../PLAN.md §5): a real scroll view doesn't stop the instant the finger
//! lifts — it keeps moving and decelerates, and a drag that lands on a still-moving
//! list grabs it in place rather than fighting it. `Gesture::history` records recent
//! (timestamp, x, y) samples while dragging, trimmed to `VELOCITY_WINDOW`;
//! `maybe_start_fling` turns that into a release velocity *on the axis the drag
//! locked to* — a fling is one drag's tail and cannot change axis — and, above
//! `MIN_FLING_START_VELOCITY`, installs `self.fling`. `self.fling` is then animated by
//! `tick_fling`, called once per `FRAME_INTERVAL` — see `run()`'s loop, which switches
//! from an indefinitely blocking `rx.recv()` to a timed `rx.recv_timeout()` only while
//! `self.fling.is_some()`, so an idle render thread still never wakes up for nothing.
//! `Command::TouchDown` always clears `self.fling` first (a new touch grabs whatever's
//! in motion, same as Android's own fling-cancel-on-down), so a fling and a live
//! gesture are never both animating `self.scroll_offsets` at once.
//!
//! Input coalescing (../PLAN.md §6, found on a real device doing a real fast scroll):
//! every `TouchMove` costs a full `redraw()` (rebuild the tree, relayout, reshape every
//! text run, paint), and `Command`s queue up in `rx` — an ordinary unbounded
//! `mpsc::channel`, and `Native.onTouchMove`'s JNI send never blocks the UI thread —
//! faster than that always finishes. Left alone, a fast real scroll visibly lags the
//! finger while the backlog grows, then keeps grinding through it, one stale position
//! at a time, for seconds after the finger has already lifted — the render thread has
//! no way to tell a stale `TouchMove` from a current one once it's sitting in the
//! channel. `run()`'s loop fixes this by draining (via non-blocking `try_recv`, so it
//! never waits for more input than what's already queued) any run of consecutive
//! `TouchMove`s down to just the last one before acting on it — `redraw()` then always
//! reflects where the finger actually is *now*, not a backlog of where it used to be.
//! A different command found mid-drain (most commonly `TouchUp`, right behind a
//! flurry of moves) is saved in `pending` and handled next, in its original order —
//! never silently dropped.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, SyncSender};
use std::time::{Duration, Instant};

use zkite_pybridge::{BuiltTree, FocusedField, RawNode};
use zkite_renderer::{NodeId, PositionedGlyph, Renderer};
use ndk::hardware_buffer_format::HardwareBufferFormat;
use ndk::native_window::NativeWindow;
use rustc_hash::{FxHashMap, FxHashSet};

use crate::text_input::{GlobalRef, TextState};

/// The platform-agnostic AccessKit schema crate (../PLAN.md §5 item 2) — this file
/// only needs `Command::A11yAction`'s payload type and its `Action` variants; the
/// actual Android JNI/`InjectingAdapter` wiring is `a11y.rs`'s job, not this file's.
use accesskit::{Action, ActionRequest};

/// Momentum-scrolling tuning. Unlike `touch_slop` (read once from Android's own
/// `ViewConfiguration` and passed down at `boot()`), there's no platform constant to
/// pull a fling deceleration curve from here — this thread owns no `View`/`Context` to
/// ask, and piping one across JNI for a single float isn't worth it. These are
/// hand-picked to land in the same ballpark as a native Android/iOS scroll fling, not
/// derived from a platform API — recalibrate against a real device if the feel is ever
/// judged off, same discipline as everything else in this file.
///
/// **Every distance/velocity constant below is in dp (dp/s), not device pixels**,
/// since ../PLAN.md §13 moved touch coordinates into dp and these are compared
/// against differences of them. Fling *distance* was unaffected by that change —
/// travel was `v0 * FLING_TAU`, linear in `v0`, so the same flick covered the same
/// physical distance it did when both sides were device pixels. What did change is
/// the two threshold constants: they now mean the same physical speed on every
/// device rather than a speed that got `density` times stricter as screens got
/// denser, which is the more correct reading of a "is this a flick" test and is why
/// they were reinterpreted rather than divided by a density to preserve the old
/// numbers exactly.
///
/// **That linearity is itself gone as of ../PLAN.md §41**: travel is now
/// `v0 * fling_tau(v0)`, which goes as `v0^1.74`. The dp reasoning above is unchanged
/// — `FLING_REFERENCE_VELOCITY` is a dp/s speed like the thresholds, so the curve
/// means the same thing on every screen density — but "linear in `v0`" no longer
/// describes this file and is left above only as the history of the dp change.
/// Fallback frame period, used only if the platform reports an implausible refresh
/// rate at `boot()`. The real one comes from `Display.getRefreshRate()` and lives in
/// `AppPaths::frame_period` — see ../PLAN.md §13.2 for why a hardcoded 16ms was
/// actively wrong here and not merely imprecise.
pub(crate) const FALLBACK_FRAME_INTERVAL: Duration = Duration::from_micros(16_667);
/// How far back `Gesture::history` looks when estimating release velocity — a short
/// recent window, not the whole drag, so the estimate reflects how the finger was
/// moving *at the moment of lift-off* rather than an average dragged down by a fast
/// start followed by a deliberate slow-down-before-release. Same shape as Android's
/// own `VelocityTracker`, reimplemented locally since this thread has no `View` to ask
/// for one.
const VELOCITY_WINDOW: Duration = Duration::from_millis(100);
/// Below this much time between the oldest and newest sample in the window, a
/// velocity estimate divides by a near-zero `dt` and is too noisy to trust — treated
/// as "no fling" rather than risking a jitter-driven launch velocity.
const MIN_VELOCITY_SAMPLE_DT: f32 = 0.008;
/// Release speed (dp/s) below which a lift-off reads as "the drag stopped
/// deliberately," not a flick.
const MIN_FLING_START_VELOCITY: f32 = 80.0;
/// Remaining speed (dp/s) below which an in-flight fling's motion is imperceptible —
/// ends the animation (and the per-frame ticking in `run()` that comes with one)
/// instead of asymptotically approaching zero forever.
const MIN_FLING_STOP_VELOCITY: f32 = 20.0;
/// `fling_tau` and `frame_periods` live in `zkite_frametiming`, not here, and the
/// reason is ../PLAN.md §15.5: this crate does not build for the host, so a formula
/// written in it can never carry a unit test. §41 shipped `fling_tau` — its entire
/// change — verified only by a table of device measurements, and recorded the absence
/// in its own known-gaps list. Both are pure arithmetic over numbers, which makes
/// moving them the same call §35 made for `zkite_imagestore`; the constants and their
/// documentation moved with them (§42).
use zkite_frametiming::{fling_tau, frame_periods};

/// The event name a finished animation is dispatched under (../PLAN.md §47). One
/// constant because it is spelled in three places that must agree — here, the
/// `handles()` guard beside it, and `zkite_py`'s `Widget._register_events`, which
/// derives it from the `on_animend=` keyword an author writes.
const ANIM_END_EVENT: &str = "animend";
/// Android's own documented default (`ViewConfiguration.getLongPressTimeout()`) —
/// not read from the device the way `touch_slop` is (that needs a `Context`, not
/// available to a background thread with no `View` of its own), but every OEM ships
/// this same 500ms default in practice. A `TouchUp` whose gesture never dragged past
/// `touch_slop` and dwelled at least this long reads as a long-press (../PLAN.md
/// §7.8) instead of a tap. Checked at *release*, not fired mid-press the way a real
/// Android long-press is (with its own haptic feedback before lift-off) — this
/// thread has no "still down after N ms" timer callback the way a `View` does, and
/// firing at release is simpler and sufficient for what §7.8 actually needs (an
/// initial selection range), at the cost of no feedback until the finger lifts.
const LONG_PRESS_THRESHOLD: Duration = Duration::from_millis(500);

/// What a `Command::Reload` sends back to the socket thread that requested it:
/// whatever the app printed while re-rendering, and the traceback if `view()` raised.
/// Both go to `zkite dev`'s terminal; the traceback is *also* already on screen as the
/// error overlay, which is the "simultaneously in the terminal" plan1.md §6 asks for.
pub struct ReloadReport {
    pub output: String,
    pub error: Option<String>,
}

pub enum Command {
    NewWindow { window: NativeWindow, width: i32, height: i32, generation: u64 },
    Resize { width: i32, height: i32 },
    DestroyWindow { ack: SyncSender<()> },
    /// `zkite dev` pushed a file and asked for a reload (../PLAN.md §15). Handled on
    /// this thread rather than on the socket thread that received it, because this
    /// thread is the one that already owns every "new tree from Python → repaint"
    /// path (`dispatch`, `scroll`, a11y actions) — routing the reload through the
    /// same door means hot reload inherits all of it, including the shape cache, the
    /// scroll offsets that survive a rebuild, and the IME pan, with no new
    /// re-entrancy to reason about. The `ack` lets the socket thread write the
    /// resulting output/traceback back to `zkite dev`'s terminal in the same turn.
    Reload { ack: SyncSender<ReloadReport> },
    /// Python rendered a frame **nobody asked for** — a `@store.update()` that did not
    /// happen inside an event dispatch, i.e. a background thread finishing a network
    /// fetch, or a timer (../PLAN.md §34).
    ///
    /// This is the one `Command` that does not originate from Android. It is sent by
    /// the frame sink `run()` installs into `zkite_pybridge`, which CPython calls on
    /// whatever thread did the `update()` — so the tree arrives as JSON that has
    /// already crossed out of Python, and this thread never has to acquire the GIL to
    /// service it. Same shape as `TextInputEvent` and `A11yAction`: only plain,
    /// already-decoded owned data crosses the channel.
    ///
    /// **Carrying the frame rather than a bare "something changed" is what makes it
    /// race-free.** The sink is handed exactly the tree that was built; a second
    /// `update()` landing behind it queues a second `FrameReady` rather than silently
    /// replacing the first one's contents mid-flight. `run()` then coalesces a run of
    /// them to the newest and never parses the ones it skipped, so the expensive half
    /// (parse, layout, reshape, paint) is paid once per burst — the same trade
    /// `TouchMove` coalescing already makes, for the same reason.
    FrameReady { tree_json: String },
    /// The user pressed Back — the system button, the back gesture, or Android 13+'s
    /// predictive-back callback; `ZKiteActivity` funnels all three here (../PLAN.md
    /// §16).
    ///
    /// **Deliberately fire-and-forget, unlike `DestroyWindow`'s acked shutdown.**
    /// Android wants a synchronous decision from `onBackPressed` (consume, or call
    /// through and finish), and the honest answer to "did anything want this?" lives
    /// in Python, behind this thread's queue — which may be mid-frame or mid-CPython
    /// call. Blocking the UI thread on that is an ANR waiting for a slow frame. So
    /// Java always consumes the press, and if nothing here wanted it this thread
    /// calls `ZKiteSurfaceView.finishActivity()` back on the UI thread instead.
    ///
    /// The tradeoff, stated rather than discovered later: if this thread is ever
    /// genuinely wedged, Back does nothing at all rather than exiting. That is a
    /// strictly better failure than an ANR dialog, and it is the same posture
    /// `onSurfaceDestroyed`'s ack timeout already takes toward a wedged thread —
    /// except this one cannot hang Android's own callback at all, because it never
    /// waits.
    Back,
    TouchDown { x: f32, y: f32 },
    TouchMove { x: f32, y: f32 },
    TouchUp { x: f32, y: f32 },
    /// Android says this gesture didn't complete normally (`MotionEvent.
    /// ACTION_CANCEL`) — ends it without ever dispatching a tap, unlike `TouchUp`.
    TouchCancel,
    /// The one-time handoff of `ZKiteSurfaceView`'s own object (already a global ref —
    /// see `GlobalRef`'s doc) from `Native.setSurfaceView`, called once from
    /// `ZKiteActivity.onCreate` right after constructing it. Needed so the render
    /// thread can later call `focusAndShowKeyboard`/`blurAndHideKeyboard` on it — see
    /// ../PLAN.md §5 (IME).
    SurfaceViewReady(GlobalRef),
    Redraw,
    /// `Listener.stateChanged` fired with a new IME text state, already decoded into
    /// plain owned data by `lib.rs`'s `Native_onTextInputEvent` (on the UI thread —
    /// see `text_input.rs`'s module doc for why `GameTextInput` itself never crosses
    /// to this thread at all). Updates whatever `TextField` is currently focused.
    /// See ../PLAN.md §5.
    TextInputEvent(TextState),
    /// The IME was dismissed without going through this crate's own
    /// `blur_text_field` (e.g. the user pressed the back button, or an on-screen
    /// "done"/"dismiss" affordance) — `Listener.onSoftwareKeyboardVisibilityChanged`/
    /// `onEditorAction` both map to this. Clears focus without re-issuing a hide
    /// request the system has already acted on. See ../PLAN.md §5.
    KeyboardDismissed,
    /// The native-`EditText` selection overlay's own text or selection changed —
    /// typing, a dragged handle, or a cut/paste from its native toolbar (../PLAN.md
    /// §7.8). Sourced from `TextSelectionOverlay`'s `TextWatcher`/
    /// `onSelectionChanged`, forwarded while the overlay still has focus (i.e. every
    /// one of these arrives before the eventual `SelectionOverlayDismissed`).
    SelectionOverlayChanged { text: String, start: i32, end: i32 },
    /// The overlay lost focus (a tap elsewhere took it, or this crate explicitly
    /// hid it — see `TextSelectionOverlay.hide`) — `text`/`start`/`end` are its
    /// final state at the moment of dismissal. See ../PLAN.md §7.8.
    SelectionOverlayDismissed { text: String, start: i32, end: i32 },
    /// The IME's own real bottom inset changed (../PLAN.md §5/§7.5) — `bottom` is
    /// how much of the bottom of the surface is currently covered (the keyboard's
    /// height when shown, `0.0` once it's gone), in the same pixel space every
    /// other layout/touch value in this file already uses. Drives `redraw`'s pan-
    /// into-view compensation for whatever field is focused.
    ImeInsetsChanged { bottom: f32 },
    /// An AccessKit action request (../PLAN.md §5 item 2) — a TalkBack
    /// double-tap-to-activate, a screen-reader "focus next element" gesture, etc.
    /// Sourced from `a11y::Actions::do_action`, which runs on whatever thread
    /// Android calls `Delegate.performAction` on (never assumed to be this thread —
    /// see `a11y.rs`'s own module doc), so — same shape as `TextInputEvent` — only
    /// plain, already-decoded data crosses the channel; nothing JNI-flavored is left
    /// to do once it's handled here.
    A11yAction(accesskit::ActionRequest),
    Stop,
}

/// Everything `zkite_pybridge::setup` needs, unpacked from assets by
/// `ZKiteActivity.java` before `Native.boot` is ever called — same three paths
/// `phase_2/appshell`'s `Native.boot` already takes (`prefix`/`zkite_pkg_dir`/
/// `app_dir`), since this is the same embedding pointed at the same kind of unpacked
/// assets, just consumed on this crate's own render thread instead of the JNI
/// caller's thread. See ../PLAN.md §5.
pub struct AppPaths {
    pub python_prefix: PathBuf,
    pub zkite_pkg_dir: String,
    pub app_dir: String,
    /// Android's own per-device touch-slop constant
    /// (`ViewConfiguration.getScaledTouchSlop()`, read once in `ZKiteSurfaceView` and
    /// passed down at boot) — the distance a touch must travel before
    /// it's treated as a drag instead of a tap. Deliberately not a constant guessed
    /// here: it's calibrated per device/density, and every other Android view uses
    /// exactly this value for exactly this decision.
    ///
    /// **In dp**, not the device pixels `getScaledTouchSlop` returns: `lib.rs`'s
    /// `Native_boot` divides by `scale` on the way in, because every distance this is
    /// compared against is a dp distance from here on (../PLAN.md §13).
    pub touch_slop: f32,
    /// Android's own per-device double-tap-slop constant
    /// (`ViewConfiguration.getScaledDoubleTapSlop()`, ../PLAN.md §5/§7.5) — the max
    /// distance between two taps' points for them to still count as one double-tap.
    /// Same "ask, don't guess" reasoning as `touch_slop`, and in dp for the same
    /// reason.
    pub double_tap_slop: f32,
    /// Android's own double-tap-timeout constant (`ViewConfiguration.
    /// getDoubleTapTimeout()`, a plain static — no density scaling, unlike the two
    /// slop constants) — the max time between two taps for them to still count as
    /// one double-tap.
    pub double_tap_timeout: Duration,
    /// This display's device pixels per dp (`DisplayMetrics.density`, read once in
    /// `ZKiteActivity` — ../PLAN.md §13). The single conversion between the two
    /// coordinate spaces this crate deals in:
    ///
    /// - **dp** — everything above the platform boundary. `twcore`'s resolved
    ///   lengths, the taffy tree, every `PositionedNode.rect`, scroll offsets, caret
    ///   and tap-to-offset geometry, the IME pan. This is the space Tailwind's own
    ///   numbers are written in (`px-6` is 24 *CSS* px, which is 24 dp), and the
    ///   space the WebView backend has always worked in, so both backends now agree
    ///   by construction rather than coincidentally at dpr 1.
    /// - **device px** — the surface, incoming touch coordinates, the IME inset,
    ///   accessibility bounds, and the selection overlay's Android view rect.
    ///
    /// Conversions happen only where those two meet, and each site says so. Anything
    /// in between never sees this number at all — in particular `zkite_renderer` has
    /// no notion of density, which is what keeps every existing layout rule,
    /// conformance fixture and the `min-h-0`/`flex-1` findings valid unchanged.
    pub scale: f32,
    /// One display refresh, from `Display.getRefreshRate()` (../PLAN.md §13.2) — the
    /// interval an in-flight fling aims to deliver a frame at.
    ///
    /// Read from the platform for the same reason `touch_slop` and `scale` are: this
    /// device runs at 60Hz but advertises 30/60/90/120, and a hardcoded 16ms would
    /// under-drive a 120Hz panel by 2x while over-driving a 30Hz one. It is a
    /// *target period*, not a vsync signal — the swapchain (`PresentMode::Fifo`)
    /// remains what actually synchronizes to the display.
    pub frame_period: Duration,
    /// The process's `JavaVM*`, obtained once via `GetJavaVM` in `Native_boot` (which
    /// runs on the UI thread, where a `JNIEnv` — and therefore a route to the
    /// `JavaVM` — is always available) — see ../PLAN.md §5. Lets the render thread
    /// attach itself (`attach_current_thread`) for the plain instance-method JNI
    /// calls it still makes off the UI thread (`call_focus_method`/`call_blur_method`) — *not*
    /// for `GameTextInput`, which stays UI-thread-only; see `text_input.rs`'s module
    /// doc for why those are different. Stored as a raw pointer width integer rather
    /// than `*mut jni_sys::JavaVM` purely so `AppPaths` (moved wholesale into the
    /// render thread's spawn closure) stays trivially `Send` without needing its own
    /// unsafe impl; `attach_current_thread` casts it back. A `JavaVM*` is explicitly
    /// documented as valid from any thread — unlike the `JNIEnv*` obtained *from*
    /// it, which is not — so handing the raw pointer across the spawn boundary and
    /// attaching on the render thread itself is the correct, not just convenient,
    /// way to do this.
    pub java_vm: usize,
}

/// A drag/tap gesture in progress, resolved once at `TouchDown` and reused for its
/// whole lifetime — real touch UIs commit to one scroll target and one tap/drag
/// classification per gesture, not re-deciding mid-drag.
struct Gesture {
    down: (f32, f32),
    /// When this gesture's `TouchDown` arrived — **not** `history[0].0`, deliberately:
    /// `history` is trimmed to `VELOCITY_WINDOW` (100ms) on every `TouchMove`, so for
    /// a real held finger (which almost always has *some* sub-`touch_slop` jitter,
    /// each jitter still a real `TouchMove`) the original down sample would age out
    /// of `history` well before a long-press's own, much longer threshold — silently
    /// breaking long-press detection for exactly the ordinary-human-finger case it
    /// most needs to work for. A separate, never-trimmed field is what §7.8's
    /// long-press check (`TouchUp`) actually reads.
    down_time: Instant,
    last: (f32, f32),
    /// Becomes `true` the first time total movement from `down` exceeds
    /// `touch_slop`, and never reverts for the rest of this gesture.
    dragging: bool,
    /// The nearest ancestor of the touch-down point that would actually *move* if
    /// dragged along each axis, with that axis's range — `zkite_py` node id, not taffy
    /// `NodeId` (see the module doc on why). Resolved once, per axis, by
    /// `Renderer::scrollable_ancestor`. Both `None` means this gesture can only ever
    /// be a tap (or a no-op drag over non-scrolling content), never a scroll.
    ///
    /// **Two targets rather than one, because the two axes genuinely resolve to
    /// different containers** — a horizontal strip inside a vertical list is the
    /// whole point of horizontal scrolling existing, and from a touch on one of its
    /// cards the x answer is the strip and the y answer is the list. Storing one
    /// target and deciding the axis later would force the axis decision back to
    /// touch-down, where it cannot be made (see `axis`).
    ///
    /// Ranges are computed once at touch-down and held: content structure cannot
    /// change mid-drag, because dragging never dispatches to Python.
    scroll_x: Option<ScrollTarget>,
    scroll_y: Option<ScrollTarget>,
    /// Which axis this drag committed to, decided at the instant `dragging` becomes
    /// true and never revisited — `None` until then.
    ///
    /// **Deferring this to the slop crossing is correct here, and it is the opposite
    /// call from `slider` directly below.** The reason the two differ is worth stating
    /// because the §26 write-up argues hard for capture-at-touch-down and this is not
    /// a departure from it: a slider *commits a value* at touch-down, so any window in
    /// which the answer is unknown is a window in which the control visibly does not
    /// respond. Scrolling moves nothing until slop is exceeded — `TouchMove` returns
    /// early while `!dragging` — so the axis is decided at exactly the first moment
    /// anything could move. There is no ambiguous window to be in.
    ///
    /// **And it locks.** A finger is imprecise: an unlocked 2D pan turns every
    /// intended horizontal swipe into a diagonal drift, which is why every native
    /// touch scroller axis-locks. The cost is that a genuinely 2D-scrollable
    /// container (content overflowing both ways, one drag meant to pan both) moves on
    /// one axis at a time. That is the same trade Android's own scrollers make.
    axis: Option<zkite_renderer::ScrollAxis>,
    /// Recent `(timestamp, x, y)` samples, oldest first, trimmed to `VELOCITY_WINDOW`
    /// on every `TouchMove` — `maybe_start_fling` reads the oldest and newest
    /// surviving sample at `TouchUp` to estimate release velocity along whichever axis
    /// `axis` locked to. Populated regardless of target/`dragging` (cheap to keep;
    /// only actually read when both are true at release).
    history: Vec<(Instant, f32, f32)>,
    /// Set when this gesture went down inside a `Slider`, which then owns it outright
    /// for its whole lifetime (../PLAN.md §26).
    ///
    /// **This is a capture at touch-down, not a direction test, and that is the
    /// substance of the §6.2 disambiguation rather than an implementation detail.**
    /// The tempting design is to let the slop test decide — horizontal movement means
    /// the slider, vertical means the scroll ancestor. It is wrong twice. A *vertical*
    /// drag that starts on a slider must still move the slider (which is what every
    /// native slider does, and what a user sliding with a slightly sloppy thumb
    /// expects) and must certainly not scroll the list underneath it; and deferring
    /// the decision until slop is exceeded means the first few millimetres of every
    /// slider drag are ambiguous, which reads as the control sticking.
    ///
    /// So when this is `Some`, neither scroll target is populated in the first place
    /// — there is no race to resolve, because the two are mutually exclusive by
    /// construction at the one moment where the answer is knowable.
    slider: Option<SliderDrag>,
}

/// One axis's answer to "what would move if this gesture became a drag" — see
/// `Gesture::scroll_x`/`scroll_y`.
#[derive(Clone)]
struct ScrollTarget {
    /// `zkite_py` node id, matching `RenderState::scroll_offsets`'s own keys.
    id: Arc<str>,
    /// How far this target can travel on its own axis, from `Renderer::max_scroll`.
    max: f32,
}

/// A `Slider` that captured the current gesture (`Gesture::slider`, ../PLAN.md §26).
#[derive(Clone)]
struct SliderDrag {
    /// Same `zkite_py` node id convention as `ScrollTarget::id`.
    target: Arc<str>,
    /// The slider's own rect in dp, resolved once at touch-down and then held for the
    /// gesture's whole lifetime — deliberately *not* re-read per move.
    ///
    /// A drag dispatches to Python on every step, so unlike `Gesture::max_scroll_y`
    /// the tree really can be rebuilt underneath this one, and a slider whose own
    /// label changes width (`Text(f"Volume: {v}")` above it, which is what a real app
    /// writes) can be re-laid-out mid-drag. Re-reading the rect each move would let
    /// the value the finger selects depend on the value it just selected. Capturing
    /// the geometry at the start is also what a real Android slider does.
    rect: (f32, f32, f32, f32),
    min: f32,
    max: f32,
    step: f32,
    /// The last value actually dispatched, so a move that quantises to the same step
    /// costs nothing — a finger crossing a 1-step-wide band emits one `change`, not
    /// one per `TouchMove`. Without this a slow drag would re-enter CPython at the
    /// full touch sample rate for no state change.
    last_value: f32,
}

/// A momentum-scroll animation in flight after a drag released with enough speed —
/// see the module doc's "Momentum" paragraph. Ticked by `tick_fling`, once per
/// `FRAME_INTERVAL`, until it naturally runs out of speed or hits either scroll bound.
struct Fling {
    /// Same `zkite_py` node id convention as `ScrollTarget::id` and
    /// `self.scroll_offsets`'s keys.
    target: Arc<str>,
    /// Which axis this fling moves — inherited from the `axis` its originating drag
    /// locked to, never re-decided. A fling is the tail of one drag, so it cannot
    /// belong to an axis that drag did not.
    axis: zkite_renderer::ScrollAxis,
    /// px/s, same sign convention as `TouchMove`'s own per-move `delta`: positive
    /// means the scroll offset is increasing (finger was moving up/left, content
    /// sliding to reveal more below/right).
    velocity: f32,
    /// The originating drag's range for `axis` at the moment it started — content
    /// structure can't change while `self.fling` is animating (nothing here calls
    /// back into Python), so this stays valid without re-querying it every tick.
    max_scroll: f32,
    /// This fling's own decay time constant, from `fling_tau(velocity)` at release.
    /// Held here rather than read from a constant because it depends on how hard the
    /// flick was — see `fling_tau`, including why it must not be recomputed per tick.
    tau: f32,
    /// When `tick_fling` last advanced this animation — the reference point for the
    /// next tick's `dt`, not a fixed frame count, so the animation's total duration
    /// and travel are correct even if a tick is delayed (e.g. a `Resize` briefly
    /// blocks the render thread).
    last_tick: Instant,
}

pub fn spawn(fonts: zkite_renderer::FontSet, paths: AppPaths) -> Sender<Command> {
    let (tx, rx) = std::sync::mpsc::channel();
    // The frame sink gets its own clone rather than reaching for `crate::SENDER`,
    // which is deliberate on two counts. `SENDER` is not populated until `spawn`
    // *returns*, so a sink reading it would be disarmed for the whole of boot; and a
    // sink that locked it would be locking a global from inside a CPython call, which
    // is a deadlock waiting for the first `update()` that happens while some JNI entry
    // point holds the same lock.
    let sink_tx = tx.clone();
    std::thread::Builder::new()
        .name("zkite-render".into())
        .spawn(move || run(fonts, paths, rx, sink_tx))
        .expect("failed to spawn zkite-render thread");
    tx
}

fn run(fonts: zkite_renderer::FontSet, paths: AppPaths, rx: Receiver<Command>, sink_tx: Sender<Command>) {
    // Without this, a panic on this thread is invisible: it unwinds, terminates only
    // this thread (default `panic = "unwind"`), and the process lives on showing
    // whatever was last drawn (typically nothing) — no crash, no logcat line, nothing
    // to search for. Found by hitting exactly this blind spot this session (see
    // `configure_surface`'s comment on the `Instance` bug it was masking) — the same
    // "Android drops a bare stderr write" class of problem phase 0's README already
    // documented for `Py_InitializeFromConfig`, now recurring for `std::panic`'s
    // default hook specifically, which the phase 0 fix (routing through
    // `__android_log_write`) didn't happen to cover since it was CPython-specific.
    std::panic::set_hook(Box::new(|info| {
        crate::loge!("zkite-render thread PANICKED: {info}");
    }));

    // Armed *before* the interpreter exists, so there is no window in which the app
    // is running and a pushed frame has nowhere to go. Nothing can actually fire
    // during boot — `State::new`'s `setup` call is a pull, and `zkite_pybridge`
    // suppresses a push made on the thread that is already pulling — but "the sink is
    // installed before Python can run" is a property worth having by construction
    // rather than by that argument. See ../PLAN.md §34.2.
    zkite_pybridge::set_frame_sink(Some(Box::new(move |tree_json: &str| {
        // A closed channel means the render thread has stopped; a `@store.update()`
        // racing teardown is not an error, it just has nothing left to paint on.
        let _ = sink_tx.send(Command::FrameReady { tree_json: tree_json.to_string() });
    })));

    let mut state = match State::new(fonts, paths) {
        Ok(s) => s,
        Err(e) => {
            crate::loge!("zkite-render thread: fatal init error: {e}");
            return;
        }
    };
    // A command already pulled out of `rx` by the coalescing block below that
    // turned out *not* to be a `TouchMove` — held here so the next loop iteration
    // processes it first, instead of losing it. `mpsc::Receiver` has no "put it
    // back" operation, so this is the loop's own one-slot lookahead buffer.
    let mut pending: Option<Command> = None;
    loop {
        // Indefinitely blocking `recv()` is right for the common case (nothing to
        // do until the next Android event) — see the module doc's original
        // reasoning. The one time that's wrong is a fling in flight: nothing is
        // going to send a command to wake this thread up for the next animation
        // frame, so a timed `recv_timeout` stands in for that, and only for as long
        // as `self.fling` is actually `Some`. A real `Command` arriving during that
        // wait (a new touch, a resize, ...) is handled exactly as it would be
        // outside a fling — `recv_timeout`'s `Ok` case is identical to `recv`'s.
        //
        // The wait is a *deadline* (`next_frame_wait`), not a fixed interval — see
        // that method and ../PLAN.md §13.2. Waiting a full frame period after each
        // frame's work is what capped the native backend at ~39fps in §12.6 while
        // its actual per-frame cost sat comfortably inside the budget.
        //
        // The wait is also timed, and that number is the point of ../PLAN.md §42:
        // it is the only measurement of time this thread had and did not use. A
        // drag that leaves no slack here cannot be helped by pacing.
        let waited_from = Instant::now();
        // `next_wake()` is `None` exactly when nothing is self-paced, which is what
        // chooses between a blocking `recv` and a timed one. Computed once so the
        // decision and the duration cannot disagree (../PLAN.md §47).
        let wake = state.next_wake();
        let paced_wait = wake.is_some();
        let received = if let Some(cmd) = pending.take() {
            Some(cmd)
        } else if let Some(wait) = wake {
            match rx.recv_timeout(wait) {
                Ok(cmd) => Some(cmd),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => {
                    crate::logi!("zkite-render thread: channel closed, stopping");
                    return;
                }
            }
        } else {
            match rx.recv() {
                Ok(cmd) => Some(cmd),
                Err(_) => {
                    crate::logi!("zkite-render thread: channel closed, stopping");
                    return;
                }
            }
        };
        state.note_idle(waited_from.elapsed(), paced_wait);
        let Some(mut cmd) = received else {
            state.tick_animation_frame();
            state.dispatch_animation_completions();
            continue;
        };

        // Coalesce a backlog of `TouchMove`s down to just the newest one. Each
        // one triggers a full `redraw()` (rebuild the tree, relayout, reshape
        // every text run, paint) — real, measured cost on a screen with dozens of
        // text nodes — and Android can deliver move samples faster than that
        // completes. Without this, a fast real scroll queues up a growing
        // backlog of stale positions in `rx`, and the render thread — having no
        // way to know they're stale — dutifully processes every one in order:
        // visibly lagging behind the finger while the backlog grows, then
        // grinding through the whole queued gesture in fast-forward after the
        // finger lifts, long after there's anything left to actually show for
        // it. `try_recv` (non-blocking, drains only what's *already* queued —
        // never waits for more) collapses any such run to its last element, so
        // `redraw()` only ever runs against where the finger actually is *now*.
        // A non-`TouchMove` found mid-drain (a `TouchUp` right behind a flurry of
        // moves, say) must not be dropped — it's saved in `pending` and handled
        // next, in its original order relative to the coalesced move.
        //
        // `FrameReady` coalesces on exactly the same argument, arrived at from the
        // other end: a background thread can call `@store.update()` in a loop far
        // faster than a frame takes to paint, and every one of those carries a
        // *complete* tree. Only the newest can still be true, so the older ones are
        // dropped without ever being parsed — which is where their cost would be.
        let same_kind: Option<fn(&Command) -> bool> = match cmd {
            Command::TouchMove { .. } => Some(|c| matches!(c, Command::TouchMove { .. })),
            Command::FrameReady { .. } => Some(|c| matches!(c, Command::FrameReady { .. })),
            _ => None,
        };
        if let Some(is_same) = same_kind {
            let mut dropped = 0u32;
            loop {
                match rx.try_recv() {
                    Ok(newer) if is_same(&newer) => {
                        cmd = newer;
                        dropped += 1;
                    }
                    Ok(other) => {
                        pending = Some(other);
                        break;
                    }
                    Err(_) => break,
                }
            }
            // Only touch moves are counted (../PLAN.md §42): a coalesced
            // `FrameReady` run means a background thread outpaced the renderer,
            // which is §34's question, not this one.
            if matches!(cmd, Command::TouchMove { .. }) {
                state.note_touch_batch(dropped);
            }
        }

        match cmd {
            Command::Stop => {
                crate::logi!("zkite-render thread: Stop received");
                return;
            }
            cmd => state.handle(cmd),
        }
        // A command-driven frame can finish an animation too — a `TouchMove` during a
        // fade paints the frame on which it completes. Drained here as well as on the
        // timeout path so a completion is never held until the *next* wakeup, which on
        // an idle app is never (../PLAN.md §47).
        state.dispatch_animation_completions();
    }
}

/// A `RawNode` a real app should never produce, shown only if `zkite_pybridge::setup`
/// itself fails (the interpreter couldn't start at all, or `runtime.py`'s glue is
/// missing/broken) — `_zkite_setup_tree` already turns an ordinary app-level exception
/// (a bad `main.py`) into a real, on-device error tree (`zkite.errors.format_error_tree`),
/// so reaching this means Python never got that far.
fn boot_failed_tree(err: &str) -> RawNode {
    RawNode {
        tag: "div".into(),
        id: Some("__boot_failed".into()),
        classes: "flex items-center justify-center bg-black p-4 text-red-400 text-xs".into(),
        events: Vec::new(),
        attrs: serde_json::Map::new(),
        children: vec![RawNode {
            tag: "#text".into(),
            id: None,
            classes: String::new(),
            events: Vec::new(),
            attrs: serde_json::Map::new(),
            children: Vec::new(),
            text: Some(format!("boot failed: {err}")),
        }],
        text: None,
    }
}

/// Attaches the calling thread (the render thread) to `vm` as a daemon thread, for
/// the handful of plain (non-`GameTextInput`) JNI calls this crate makes off the UI
/// thread — `focusAndShowKeyboard`/`blurAndHideKeyboard` via
/// `call_focus_method`/`call_blur_method`. Deliberately separate from `GameTextInput_init` (see
/// text_input.rs's module doc, and this session's own crash log): `GameTextInput`'s
/// `FindClass`-by-name calls only resolve app classes correctly on a thread the JVM
/// itself called into — a raw attached thread's `FindClass` falls back to the boot
/// classloader and fails on any app-defined class, fatally. Ordinary instance-method
/// calls on an object this thread already holds a reference to
/// (`GetObjectClass`+`GetMethodID`+`CallVoidMethod`, not `FindClass`-by-name) have no
/// such restriction — they read the object's actual runtime type directly — so
/// `resolve_surface_view_methods`/`call_focus_method`/`call_blur_method` are safe to keep on this
/// thread's own attachment.
fn attach_current_thread(vm: *mut jni_sys::JavaVM) -> Option<*mut jni_sys::JNIEnv> {
    if vm.is_null() {
        crate::loge!("attach_current_thread: no JavaVM (GetJavaVM failed at boot?)");
        return None;
    }
    let mut env: *mut std::ffi::c_void = std::ptr::null_mut();
    let rc = unsafe {
        let vtable = **vm;
        let Some(attach) = vtable.AttachCurrentThreadAsDaemon else {
            crate::loge!("attach_current_thread: no AttachCurrentThreadAsDaemon");
            return None;
        };
        attach(vm, &mut env, std::ptr::null_mut())
    };
    if rc != 0 || env.is_null() {
        crate::loge!("attach_current_thread: AttachCurrentThreadAsDaemon failed: rc={rc}");
        return None;
    }
    Some(env as *mut jni_sys::JNIEnv)
}

/// Looks up `ZKiteSurfaceView.focusAndShowKeyboard(String)`
/// (`(Ljava/lang/String;)V` — see ../PLAN.md §5/§7.5, per-field input types) and
/// `.blurAndHideKeyboard()` (`()V`) once, via `env` (must be `State::render_env` —
/// see `attach_current_thread`'s doc comment on why calls through this crate stay
/// on one specific thread's env). `None` (logged) if either lookup fails, which
/// `call_focus_method`/`call_blur_method` treat as "focus/blur silently does
/// nothing" rather than a crash — the same posture as every other optional-
/// hardware-or-API-surface fallback in this file (`gpu`, `render_env` itself).
fn resolve_surface_view_methods(
    env: *mut jni_sys::JNIEnv,
    view: jni_sys::jobject,
) -> Option<(jni_sys::jmethodID, jni_sys::jmethodID)> {
    unsafe {
        let vtable = **env;
        let class = (vtable.GetObjectClass?)(env, view);
        let get_mid = vtable.GetMethodID?;
        let show_sig = std::ffi::CString::new("(Ljava/lang/String;)V").unwrap();
        let hide_sig = std::ffi::CString::new("()V").unwrap();
        let show_name = std::ffi::CString::new("focusAndShowKeyboard").unwrap();
        let hide_name = std::ffi::CString::new("blurAndHideKeyboard").unwrap();
        let show_mid = get_mid(env, class, show_name.as_ptr(), show_sig.as_ptr());
        let hide_mid = get_mid(env, class, hide_name.as_ptr(), hide_sig.as_ptr());
        if let Some(del) = vtable.DeleteLocalRef {
            del(env, class);
        }
        if show_mid.is_null() || hide_mid.is_null() {
            crate::loge!("resolve_surface_view_methods: GetMethodID failed (ZKiteSurfaceView missing focusAndShowKeyboard(String)/blurAndHideKeyboard?)");
            if let Some(clear) = vtable.ExceptionClear {
                clear(env);
            }
            return None;
        }
        Some((show_mid, hide_mid))
    }
}

/// Looks up `ZKiteSurfaceView.setSelection(String, int, int)` once — JNI type
/// descriptor `(Ljava/lang/String;II)V` in the grammar `GetMethodID` needs. Same
/// `GetObjectClass`+`GetMethodID` pattern, and the same "no `FindClass`-by-name
/// involved, so no thread restriction" reasoning, as `resolve_surface_view_methods`
/// — see ../PLAN.md §7.7.
fn resolve_set_selection_method(env: *mut jni_sys::JNIEnv, view: jni_sys::jobject) -> Option<jni_sys::jmethodID> {
    unsafe {
        let vtable = **env;
        let class = (vtable.GetObjectClass?)(env, view);
        let get_mid = vtable.GetMethodID?;
        let name = std::ffi::CString::new("setSelection").unwrap();
        let sig = std::ffi::CString::new("(Ljava/lang/String;II)V").unwrap();
        let mid = get_mid(env, class, name.as_ptr(), sig.as_ptr());
        if let Some(del) = vtable.DeleteLocalRef {
            del(env, class);
        }
        if mid.is_null() {
            crate::loge!(
                "resolve_set_selection_method: GetMethodID failed (ZKiteSurfaceView missing setSelection(String,int,int)?)"
            );
            if let Some(clear) = vtable.ExceptionClear {
                clear(env);
            }
            return None;
        }
        Some(mid)
    }
}

/// Looks up `ZKiteSurfaceView.finishActivity()` (`()V`) once — how this thread exits
/// the app after a Back press nothing in `zkite_py` wanted (../PLAN.md §16). Same
/// `GetObjectClass`+`GetMethodID` pattern and the same thread reasoning as
/// `resolve_surface_view_methods`; the Java side does the UI-thread hop, exactly as
/// `focusAndShowKeyboard` already does (§7.3 item 2).
fn resolve_finish_activity_method(
    env: *mut jni_sys::JNIEnv,
    view: jni_sys::jobject,
) -> Option<jni_sys::jmethodID> {
    unsafe {
        let vtable = **env;
        let class = (vtable.GetObjectClass?)(env, view);
        let get_mid = vtable.GetMethodID?;
        let name = std::ffi::CString::new("finishActivity").unwrap();
        let sig = std::ffi::CString::new("()V").unwrap();
        let mid = get_mid(env, class, name.as_ptr(), sig.as_ptr());
        if let Some(del) = vtable.DeleteLocalRef {
            del(env, class);
        }
        if mid.is_null() {
            crate::loge!("resolve_finish_activity_method: GetMethodID failed (ZKiteSurfaceView missing finishActivity()?)");
            if let Some(clear) = vtable.ExceptionClear {
                clear(env);
            }
            return None;
        }
        Some(mid)
    }
}

/// Looks up `ZKiteSurfaceView.showSelectionOverlay(...)`/`.hideSelectionOverlay()`
/// once (../PLAN.md §7.8) — same `GetObjectClass`+`GetMethodID` pattern as
/// `resolve_surface_view_methods`/`resolve_set_selection_method`. `show`'s type
/// descriptor (`(Ljava/lang/String;IIFFFFFIIZLjava/lang/String;)V`) must match `ZKiteSurfaceView.
/// showSelectionOverlay`'s real parameter list exactly, in order — text, start,
/// end, x, y, w, h, textSizePx, textColor, bgColor, isPassword (../PLAN.md §5/§7.5
/// — lets the overlay's own real `EditText` mask via a real `TYPE_TEXT_VARIATION_
/// PASSWORD`, see `show_selection_overlay`'s doc comment).
fn resolve_selection_overlay_methods(
    env: *mut jni_sys::JNIEnv,
    view: jni_sys::jobject,
) -> Option<(jni_sys::jmethodID, jni_sys::jmethodID)> {
    unsafe {
        let vtable = **env;
        let class = (vtable.GetObjectClass?)(env, view);
        let get_mid = vtable.GetMethodID?;
        let show_name = std::ffi::CString::new("showSelectionOverlay").unwrap();
        let show_sig = std::ffi::CString::new("(Ljava/lang/String;IIFFFFFIIZLjava/lang/String;)V").unwrap();
        let hide_name = std::ffi::CString::new("hideSelectionOverlay").unwrap();
        let hide_sig = std::ffi::CString::new("()V").unwrap();
        let show_mid = get_mid(env, class, show_name.as_ptr(), show_sig.as_ptr());
        let hide_mid = get_mid(env, class, hide_name.as_ptr(), hide_sig.as_ptr());
        if let Some(del) = vtable.DeleteLocalRef {
            del(env, class);
        }
        if show_mid.is_null() || hide_mid.is_null() {
            crate::loge!("resolve_selection_overlay_methods: GetMethodID failed (ZKiteSurfaceView missing showSelectionOverlay/hideSelectionOverlay?)");
            if let Some(clear) = vtable.ExceptionClear {
                clear(env);
            }
            return None;
        }
        Some((show_mid, hide_mid))
    }
}

/// Looks up `ZKiteSurfaceView.isAccessibilityEnabled()` (`()Z`) once — the guard
/// `push_a11y_update` must consult before handing anything to
/// `accesskit_android`, because raising an `AccessibilityEvent` while accessibility
/// is off is a hard process abort, not a no-op (../PLAN.md §8.7; the Java method's
/// own doc comment has the full account). Same `GetObjectClass`+`GetMethodID`
/// pattern, and the same "no `FindClass`-by-name, so no thread restriction",
/// reasoning as `resolve_surface_view_methods`.
fn resolve_is_accessibility_enabled_method(
    env: *mut jni_sys::JNIEnv,
    view: jni_sys::jobject,
) -> Option<jni_sys::jmethodID> {
    unsafe {
        let vtable = **env;
        let class = (vtable.GetObjectClass?)(env, view);
        let get_mid = vtable.GetMethodID?;
        let name = std::ffi::CString::new("isAccessibilityEnabled").unwrap();
        let sig = std::ffi::CString::new("()Z").unwrap();
        let mid = get_mid(env, class, name.as_ptr(), sig.as_ptr());
        if let Some(del) = vtable.DeleteLocalRef {
            del(env, class);
        }
        if mid.is_null() {
            crate::loge!(
                "resolve_is_accessibility_enabled_method: GetMethodID failed (ZKiteSurfaceView missing isAccessibilityEnabled()?)"
            );
            if let Some(clear) = vtable.ExceptionClear {
                clear(env);
            }
            return None;
        }
        Some(mid)
    }
}

/// Present when a GPU is available and initialized. `None` means every draw uses the
/// `vello_cpu` + `ANativeWindow_lock` fallback (see module doc).
/// One laid-out frame: the tree `zkite_pybridge` built and where every node in it
/// ended up. Exists so `redraw` can produce one, report its viewports to `zkite_py`,
/// and — if that changed which rows a virtualised `List` needs — produce another
/// before painting anything. See `State::layout_frame`.
struct LayoutFrame {
    built: BuiltTree,
    nodes: Vec<zkite_renderer::PositionedNode>,
    /// What this pass cost, per phase — accumulated across however many passes one
    /// `redraw` runs. See `FramePhases`.
    phases: FramePhases,
}

/// How many build+layout passes one `redraw` will spend converging with `zkite_py`
/// on which rows a virtualised list needs (../PLAN.md §11). Two is enough by
/// construction — a `List`'s content height doesn't depend on its window, so
/// reporting a viewport can change the tree but can't change that viewport — and
/// bounding it means a violation of that invariant shows up as one log line and a
/// slightly stale frame rather than as a hung render thread.
const MAX_VIEWPORT_SYNC_PASSES: usize = 2;

/// How many redraws go into one `redraw x60: avg …` log line. Roughly a second of
/// continuous scrolling, so a drag or fling produces a steady trickle of real
/// numbers while an idle app produces none.
const REDRAW_STATS_INTERVAL: u32 = 60;

/// A single redraw slower than this gets its own log line immediately, rather than
/// waiting to be averaged into the next `REDRAW_STATS_INTERVAL` summary.
///
/// Two reasons, both learned by trying to measure a 10,000-row list without it
/// (../PLAN.md §11.5). Diagnostically, an average hides exactly the frames worth
/// knowing about — one 400ms stall inside sixty 8ms frames reads as 14ms average.
/// Practically, a summary that only prints every 60 frames prints *nothing at all*
/// for an app slow enough that input coalescing collapses eight full swipes into
/// fewer than 60 redraws, which is precisely the app you most want numbers for.
/// 33ms is one missed frame at 30fps — comfortably above a healthy frame, well
/// below anything a user would call smooth.
const SLOW_REDRAW_THRESHOLD: Duration = Duration::from_millis(33);

/// Where a frame's time actually goes, accumulated over `REDRAW_STATS_INTERVAL`
/// redraws and logged alongside the total.
///
/// Added when the shaped-text cache (../PLAN.md §11.5/§11.9) turned out to buy far
/// less on the device than the host benchmark predicted — a host measurement of
/// `zkite_renderer` alone said shaping was 99% of *its* cost, which was true and still
/// left most of the real frame unaccounted for, because a real frame is also a
/// CPython call, a JSON parse, a tree build, an accessibility push and a full paint.
/// Attributing a frame by subtracting the one phase you happened to measure is how
/// that mistake gets made twice, so every phase is measured.
#[derive(Default, Clone, Copy)]
struct FramePhases {
    /// `zkite_pybridge::build` — the CPython `view()` call, its JSON, and the taffy
    /// tree built from it.
    build: Duration,
    /// `Renderer::layout` — taffy plus whatever text the shape cache didn't serve.
    layout: Duration,
    /// `Renderer::positioned` — the flatten/clone walk, run up to twice per pass.
    positioned: Duration,
    /// `sync_viewports` — reporting scroll position back to `zkite_py`, which on a
    /// scrolling frame means a real CPython call. ../PLAN.md §11.7 named this as the
    /// thing to measure on its own once the shaped-text cache landed; this is that
    /// measurement.
    ///
    /// **Kept as the total, with `sync_py` and `sync_parse` splitting it** (§43), for
    /// the same reason §39 split `acquire` out of `present` rather than replacing it:
    /// four sections of measurement are quoted against this number and have to stay
    /// comparable.
    sync: Duration,
    /// The CPython half of `sync` — `zkite_pybridge::scroll`, i.e. GIL acquire, the
    /// `_zkite_scroll_tree` call (`view()`, the row `builder`s, the `to_tree` walk and
    /// `json.dumps`), and the returned string crossing the FFI boundary.
    ///
    /// Split out because ../PLAN.md §42.5 priced the remaining fix off `sync` as a
    /// single number (`~0.38ms x rows`), and the fix it names — build only the rows
    /// that entered — can only remove work from *this* half. Whatever is in
    /// `sync_parse` is spent walking the whole window no matter which rows were
    /// freshly built, so a split that came out 50/50 would mean the predicted ~1.2ms
    /// is unreachable without also changing the transport. Measuring which it is
    /// before building the fix is §40.1's and §42.3's lesson applied a third time.
    sync_py: Duration,
    /// The same CPython call on the frames where it answers "nothing to do" — the
    /// common case during a scroll (~93% of scroll frames at `overscan=4`). Kept
    /// apart from `sync_py` so the `rebuild:` line can divide by rebuilds without
    /// inflating them by the polls, and because §40.4 names a large value here with
    /// `rebuilt 0` as its own distinct bug.
    sync_poll: Duration,
    /// The Rust half of `sync` — `RawNode::from_json` over the whole returned tree.
    sync_parse: Duration,
    /// `build + layout + positioned` for every `layout_frame()` pass *after the
    /// first* — i.e. the cost of laying the tree out a second time because
    /// `sync_viewports` produced a new one (§43).
    ///
    /// Not a new phase: it is a subset of `build`/`layout`/`positioned`, which
    /// already accumulate both passes. It is measured separately because those three
    /// are reported as an average over all frames, and the second pass happens only
    /// on rebuild frames — which §42.2 established are ~89% of the late ones. So the
    /// per-frame averages spread a cost that lands entirely on the frames that miss,
    /// and §42.5's headroom arithmetic (~1.2ms of `sync` into ~3.9ms) never counted
    /// it at all.
    relayout: Duration,
    /// Building the `accesskit::TreeUpdate` and handing it to Android.
    a11y: Duration,
    /// Walking `PositionedNode`s into a `vello_hybrid::Scene` (or the CPU
    /// equivalent) — CPU work, entirely ours.
    scene: Duration,
    /// `get_current_texture` alone — the blocking swapchain acquire, i.e. **waiting,
    /// not working**.
    ///
    /// Split out of `present` in §39 because that field's own doc comment already
    /// warned that "time here is not necessarily work done" and nothing had ever
    /// measured which half it was. It mattered: on a Moto G54 mid-scroll the combined
    /// number was 9.02ms of an 17.64ms frame, which reads as a catastrophically slow
    /// painter and is nothing of the kind.
    acquire: Duration,
    /// `vello_hybrid::render` + `submit` + `present` — real work, ours and the driver's.
    ///
    /// **Kept as the total, with `present_render`/`present_submit`/`present_swap`
    /// splitting it** (§46), for the reason §43 kept `sync` when it split it: §39, §42,
    /// §43 and §44 all quote this number and have to stay comparable.
    present: Duration,
    /// `create_view` + `create_command_encoder` + `vello_hybrid::Renderer::render` —
    /// the CPU-side strip generation this document has described as the whole of
    /// `present` since §39.5, plus the GPU command encoding that goes with it. The two
    /// setup calls are in here rather than in a fourth field because they are ours in
    /// the same sense `render` is; if this field is large they are where to look next.
    ///
    /// Split out because that description is a **claim about composition that nothing
    /// ever checked**, and it is the claim Queue H's last remaining item is priced off:
    /// "overdraw is unmeasured … `present` is CPU-side strip generation for the whole
    /// surface every frame" (§39.5, repeated in §42.8 and §44.7). `present` is one
    /// `Instant` around three unrelated things, which is exactly the shape §43 found
    /// `sync` in — one number covering four kinds of work, modelled as one, 44% of it a
    /// defect. Occlusion culling can only ever remove work from *this* field.
    present_render: Duration,
    /// `CommandEncoder::finish` alone — wgpu resolving the encoded stream into a
    /// command buffer (barrier and resource-tracking work, ours by proxy).
    ///
    /// Split from `present_submit` because the first version of this instrument lumped
    /// the two and would have reported "`submit` is 5.3ms" when half of it might have
    /// been `finish` — which is the exact mistake this section exists to report about
    /// `present`. Content-scaling already argued against it (`finish`'s cost tracks the
    /// encoded command count, which varied 5x with no movement), but arguing is what
    /// §39.5 did.
    present_finish: Duration,
    /// `Queue::submit` alone — handing the command buffer to the driver.
    present_submit: Duration,
    /// `SurfaceTexture::present` alone — the swapchain enqueue.
    ///
    /// §39 split `acquire` (`get_current_texture`) out of `present` and refuted the
    /// *dequeue* half of the blocking hypothesis: the swapchain was not throttling
    /// there. It never touched the **enqueue** half, and on a tiler with a full buffer
    /// queue that is where back-pressure moves to.
    ///
    /// **Read this beside `acquire`, not alone.** GPU work is asynchronous, so all
    /// three of these fields time CPU occupancy inside a call, not GPU execution; a
    /// frame that overruns the GPU can pay for it at the *next* frame's `acquire`
    /// rather than here. What the three do settle is which call this thread is sitting
    /// in, which is the question "is `present` ours to optimise" reduces to.
    present_swap: Duration,
}

impl FramePhases {
    /// Per-phase **maximum** over a window, rather than a sum.
    ///
    /// Averages hid the thing that actually matters for perceived smoothness: §39
    /// measured `avg 17.15ms max 45.39ms` frames on a Moto G54 and the per-phase
    /// averages added up to the *average*, saying nothing about where the 45ms went.
    /// A hitch is one frame in sixty; it cannot be seen in a mean of sixty.
    fn take_max(&mut self, other: &FramePhases) {
        self.build = self.build.max(other.build);
        self.layout = self.layout.max(other.layout);
        self.positioned = self.positioned.max(other.positioned);
        self.sync = self.sync.max(other.sync);
        self.sync_py = self.sync_py.max(other.sync_py);
        self.sync_poll = self.sync_poll.max(other.sync_poll);
        self.sync_parse = self.sync_parse.max(other.sync_parse);
        self.relayout = self.relayout.max(other.relayout);
        self.a11y = self.a11y.max(other.a11y);
        self.scene = self.scene.max(other.scene);
        self.acquire = self.acquire.max(other.acquire);
        self.present = self.present.max(other.present);
        self.present_render = self.present_render.max(other.present_render);
        self.present_finish = self.present_finish.max(other.present_finish);
        self.present_submit = self.present_submit.max(other.present_submit);
        self.present_swap = self.present_swap.max(other.present_swap);
    }

    fn add(&mut self, other: &FramePhases) {
        self.build += other.build;
        self.layout += other.layout;
        self.positioned += other.positioned;
        self.sync += other.sync;
        self.sync_py += other.sync_py;
        self.sync_poll += other.sync_poll;
        self.sync_parse += other.sync_parse;
        self.relayout += other.relayout;
        self.a11y += other.a11y;
        self.scene += other.scene;
        self.acquire += other.acquire;
        self.present += other.present;
        self.present_render += other.present_render;
        self.present_finish += other.present_finish;
        self.present_submit += other.present_submit;
        self.present_swap += other.present_swap;
    }
}

/// **Which** frames missed the refresh budget, and whether this thread had any slack
/// left when they did — accumulated over the same window as `FramePhases`.
///
/// Added by ../PLAN.md §42 for the reason §40.4 added `scroll:`: the existing lines
/// report an average interval and a worst interval, and `avg 17.48ms max 39.41ms` is
/// equally consistent with *"one frame in twenty takes two refreshes"* and with
/// *"most frames run slightly over"*. Those two have different fixes and neither
/// existing line can tell them apart, so the histogram is the measurement, not a
/// nicer presentation of one.
///
/// `idle` is the discriminator that actually decides what to do next. §39.5 proposed
/// pacing a finger-down drag against vsync the way §13.2 paces a fling — which can
/// only help if this thread is ever *waiting* during a drag. If `idle` is ~0 the
/// thread is saturated, every frame starts the instant its input arrives, and a
/// deadline would return `ZERO` every time: pacing would be a no-op and only making
/// the frame cheaper can help. That is the same shape of check §40.1 ran on
/// `overscan` before implementing it, and for the same reason.
#[derive(Default, Clone, Copy)]
struct BudgetStats {
    /// Frame intervals that fit inside one refresh period (with `LATE_TOLERANCE`
    /// of slack, since a frame delivered a hair late is still shown on that vsync).
    on_time: u32,
    /// Intervals of about two refresh periods — exactly one dropped frame.
    late_1: u32,
    /// Three periods or more.
    late_2plus: u32,
    /// Of the late intervals, how many ended on a frame that re-ran `view()`.
    /// Cross-tabulated with `State::scroll_stats` rather than derived from it:
    /// knowing 9% of frames rebuild and 22% of frames are late says nothing about
    /// whether they are the *same* frames, which is the entire question.
    late_rebuilt: u32,
    /// Wall-clock this thread spent blocked on `rx` with nothing to do, over the
    /// window. Waits longer than `IDLE_GAP` are excluded — those are an idle app,
    /// not slack inside a gesture, and are excluded from the interval stats by the
    /// same rule so both numbers describe the same frames.
    ///
    /// **Waits made while a fling is in flight are excluded too, and that exclusion
    /// is the difference between measuring something and manufacturing it.** During
    /// a fling `run()` waits on `next_frame_wait` *on purpose* (§13.2) — that idle
    /// is the pacing working, and counting it would report slack in the window
    /// precisely because the frames in it were already paced, which is the answer
    /// this number exists to test for. `fling_frames` says how much of the window
    /// was excluded on those grounds.
    idle: Duration,
    /// Frames in this window painted while a fling was in flight, i.e. the ones
    /// already paced by `next_frame_wait`. A window with `fling 0` is a pure
    /// finger-down drag and its `idle` describes only unpaced frames.
    fling_frames: u32,
    /// `TouchMove`s that arrived, including the ones never acted on.
    moves_in: u32,
    /// …and how many of those `run()`'s coalescing loop dropped unprocessed. A
    /// large number means Android is delivering move samples faster than a frame
    /// takes, which is the other way to be input-saturated.
    moves_dropped: u32,
}

/// A gap longer than this is an idle app, not a frame interval and not slack inside
/// a gesture. Well past any real frame and well under any real pause, so both
/// `State::interval_stats` and `BudgetStats::idle` use it to stay describing the
/// same frames.
const IDLE_GAP: Duration = Duration::from_millis(250);

struct Gpu {
    // Kept, and reused for every surface — an `Adapter` and any `Surface` it's used
    // with (via `get_capabilities`, or implicitly through `compatible_surface`) are
    // only meant to come from the *same* `Instance`; see `try_init_gpu`/
    // `configure_surface`'s comments for the bug this fixed.
    instance: wgpu::Instance,
    adapter: wgpu::Adapter,
    device: wgpu::Device,
    queue: wgpu::Queue,
    /// `'static`: created via `create_surface_unsafe` from a raw handle, not tied to
    /// a borrow — the actual lifetime contract (the `NativeWindow` must outlive the
    /// `Surface`) is upheld by hand, by always dropping `surface` before `window` (see
    /// `State::drop_window`), not by the type system. Reused across `NewWindow`s
    /// (recreated fresh each time) rather than across the whole `Gpu`'s life, since a
    /// `Surface` is tied to one specific `ANativeWindow`.
    surface: Option<wgpu::Surface<'static>>,
    hybrid_renderer: Option<vello_hybrid::Renderer>,
    resources: vello_hybrid::Resources,
}

struct State {
    renderer: Renderer,
    /// The last tree `zkite_pybridge` produced — from `setup` at boot, or from
    /// `dispatch` after a tap that hit something with a handler. The one piece of
    /// state that actually matters; everything else here is rendering plumbing around
    /// it.
    current: RawNode,
    /// `current`'s most recent `zkite_renderer` build — `None` only before the first
    /// `redraw()`. Rebuilt from `current` on every redraw (`Renderer` state is meant
    /// to be rebuilt per render, not mutated — see `zkite_renderer::Renderer::clear`'s
    /// doc comment), so this is really just `hit_test`/dispatch bookkeeping for the
    /// *next* touch, not a cache in the performance sense.
    built: Option<BuiltTree>,
    /// Current scroll position for every scrolled container, keyed by `zkite_py` node
    /// id (survives rebuilds — see the module doc). Absent entries mean "never
    /// scrolled," equivalent to `(0.0, 0.0)`, not tracked eagerly for every
    /// scrollable node up front.
    scroll_offsets: FxHashMap<Arc<str>, (f32, f32)>,
    /// The last `(scroll_y, viewport_height)` actually reported to `zkite_py` for each
    /// scrollable container (../PLAN.md §11) — so a redraw that didn't move anything
    /// (a tap, a focus change, an IME pan) costs no Python call at all. Keyed by
    /// `zkite_py` node id, and for the same reason `scroll_offsets` is: it has to
    /// survive the tree rebuild every redraw performs. See `sync_viewports`.
    reported_viewports: FxHashMap<Arc<str>, (f32, f32)>,
    /// The in-progress touch gesture, if any — `None` between touches (after
    /// `TouchUp`, before the next `TouchDown`).
    gesture: Option<Gesture>,
    /// The in-progress momentum-scroll animation, if any — see the module doc's
    /// "Momentum" paragraph. Mutually exclusive with `gesture` in practice:
    /// `TouchDown` always clears this before a new `gesture` starts, and this is
    /// only ever set from `maybe_start_fling`, called after `gesture` is taken
    /// (`Option::take`) at `TouchUp`.
    fling: Option<Fling>,
    /// In dp — see `AppPaths::touch_slop`.
    touch_slop: f32,
    /// One display refresh — see `AppPaths::frame_period`.
    frame_period: Duration,
    /// When the most recent `redraw()` *started*, or `None` if none has yet.
    ///
    /// This is what makes fling pacing a deadline rather than a delay (../PLAN.md
    /// §13.2). The loop used to `recv_timeout(FRAME_INTERVAL)` and *then* do a
    /// frame's work, so the delivered period was `interval + work` — 16 + ~13 = ~29ms
    /// for a ~13ms frame that had a 16.7ms budget. Measuring from the start of the
    /// last frame instead makes the wait absorb the work rather than follow it.
    last_frame_at: Option<Instant>,
    /// Device pixels per dp — see `AppPaths::scale` for the full coordinate-space
    /// contract. Read on every conversion rather than baked into derived fields, so
    /// there is exactly one authority for it.
    scale: f32,
    /// The surface's size in **device pixels**, exactly as Android reports it. Layout
    /// runs against `logical_size()` instead; these two stay in their own space so
    /// that swapchain configuration, `vello` canvas sizing and the `frame:` log line
    /// keep meaning real pixels.
    width: i32,
    height: i32,
    window: Option<NativeWindow>,
    window_generation: u64,
    gpu: Option<Gpu>,
    /// Set once, the first time `NewWindow` is handled, and never retried — matches
    /// plan1.md's own framing of the CPU path as the fallback for a GPU that isn't
    /// there, not a thing to keep re-probing for every frame.
    gpu_probed: bool,
    /// This thread's own attachment to the JVM (../PLAN.md §5, IME) — `None` only if
    /// attaching failed at `new()` (logged there; every focus/blur attempt after
    /// that is a no-op, same "documented gap, not a crash" pattern as `gpu`'s own
    /// `None` case). **Not** used for `GameTextInput` itself — see `text_input.rs`'s
    /// module doc for why that stays confined to the UI thread (`lib.rs`'s
    /// `TEXT_INPUT` static) while this env is only ever used for ordinary instance-
    /// method calls (`call_focus_method`/`call_blur_method`).
    render_env: Option<*mut jni_sys::JNIEnv>,
    /// `ZKiteSurfaceView`'s own object, set once via `SurfaceViewReady` — needed to
    /// call `focusAndShowKeyboard`/`blurAndHideKeyboard` on it.
    surface_view: Option<GlobalRef>,
    last_system_bars: Option<String>,
    /// Cached `(focusAndShowKeyboard, blurAndHideKeyboard)` method ids, resolved
    /// lazily the first time either is needed (not at `SurfaceViewReady` time, since
    /// resolving them needs `render_env`, and — in principle — either could arrive
    /// first).
    surface_view_methods: Option<(jni_sys::jmethodID, jni_sys::jmethodID)>,
    /// Cached `ZKiteSurfaceView.setSelection(String, int, int)` method id
    /// (../PLAN.md §7.7, tap-to-position-cursor) — resolved lazily the same way as
    /// `surface_view_methods`, kept separate since it's a different signature
    /// (`GetMethodID` needs the exact JNI type descriptor, not just a name).
    set_selection_method: Option<jni_sys::jmethodID>,
    /// Cached `ZKiteSurfaceView.finishActivity()` method id (../PLAN.md §16) — how
    /// this thread exits the app when a Back press reached Python and nothing
    /// wanted it. Resolved lazily like every other method id here.
    finish_activity_method: Option<jni_sys::jmethodID>,
    /// Cached `(showSelectionOverlay, hideSelectionOverlay)` method ids (../PLAN.md
    /// §7.8), resolved lazily the same way as `surface_view_methods`/
    /// `set_selection_method`.
    selection_overlay_methods: Option<(jni_sys::jmethodID, jni_sys::jmethodID)>,
    /// The `zkite_py` node id the selection overlay is currently showing for, if
    /// any — `None` whenever it's hidden. **Not** the same thing as `focused`:
    /// tapping a *different* field while this one's overlay is still up updates
    /// `focused` immediately (`focus_text_field`), but the overlay's own dismiss
    /// report for the field it was actually covering arrives later, asynchronously
    /// (see `Command::SelectionOverlayDismissed`'s own doc comment on the race this
    /// separate field exists to resolve correctly).
    selection_overlay_field: Option<Arc<str>>,
    /// The `zkite_py` node id of the currently focused `TextField`, if any — `None`
    /// means no field has focus (the common case, and the state right after
    /// `blur_text_field`).
    focused: Option<Arc<str>>,
    /// The focused field's live text, from the most recent `GameTextInputState` —
    /// what actually gets displayed while focused, overriding `RawNode.attrs["value"]`
    /// (see `zkite_pybridge::FocusedField`'s doc comment). Meaningless (and unread)
    /// while `focused` is `None`.
    focused_text: String,
    /// The focused field's current `[start, end)` selection, as UTF-8 byte offsets
    /// into `focused_text` — `start == end` is a plain cursor. Updated from two
    /// places: `Command::TextInputEvent` (the IME's own real report, after a
    /// commit/compose) and `set_cursor_position` (optimistically, right after a tap
    /// asks `GameTextInput` to move it — see that method's own doc comment on why
    /// no event follows to confirm it). Meaningless while `focused` is `None`, same
    /// as `focused_text`.
    focused_selection: (i32, i32),
    /// The IME's own last-reported bottom inset (../PLAN.md §5/§7.5) — how much of
    /// the bottom of the surface the keyboard currently covers, `0.0` when it's
    /// hidden. Updated by `Command::ImeInsetsChanged`; read fresh at the top of
    /// every `redraw()` to (re)compute `ime_pan_offset_y`, never used directly for
    /// painting/hit-testing itself.
    ime_inset_bottom: f32,
    /// How far the *whole tree* is currently panned up (positive = shifted up), so
    /// a focused field the keyboard would otherwise cover stays visible above it —
    /// see `redraw`'s own comment for the full mechanism (it's implemented as an
    /// ordinary scroll-offset entry keyed on the tree's own root, reusing
    /// `zkite_renderer`'s existing per-node offset machinery rather than a bespoke
    /// transform). Recomputed from scratch at the top of every `redraw()` (derived
    /// state, not something any caller sets directly); `node_scroll_offsets` reads
    /// the value `redraw` last computed so touch handling between redraws hit-tests
    /// against what's actually on screen right now, not an unpanned layout. `0.0`
    /// whenever nothing is focused or the keyboard isn't covering it.
    ime_pan_offset_y: f32,
    double_tap_slop: f32,
    double_tap_timeout: Duration,
    /// The most recent tap that focused-and-positioned-cursor on a `TextField`
    /// (../PLAN.md §5/§7.5, double-tap-to-select-word) — `(when, field, x, y)`.
    /// `None` after a double-tap fires (consumed, so a third rapid tap starts a
    /// fresh pair rather than chaining into a second double-tap) or after any tap
    /// that didn't land on a `TextField` at all (breaks pairing across an
    /// unrelated tap, the same way a real double-tap gesture only ever pairs two
    /// *consecutive* taps). Read by `is_double_tap`.
    last_tap: Option<(Instant, Arc<str>, f32, f32)>,
    /// The AccessKit adapter (../PLAN.md §5 item 2) — installed on `ZKiteSurfaceView`
    /// by `ensure_a11y_adapter` the first `redraw()` after both `render_env` and
    /// `surface_view` are available, and pushed a fresh tree on every `redraw()`
    /// after that (`push_a11y_update`). `None` until then, or permanently if
    /// construction ever fails (logged, not fatal — same "documented gap, not a
    /// crash" posture as `gpu`'s own fallible init).
    a11y_adapter: Option<accesskit_android::InjectingAdapter>,
    /// Cached `ZKiteSurfaceView.isAccessibilityEnabled()` method id (../PLAN.md
    /// §8.7), resolved lazily the same way as `surface_view_methods`.
    is_accessibility_enabled_method: Option<jni_sys::jmethodID>,
    /// The last `(width, height, paint path)` `redraw` actually logged — so that
    /// line is emitted on *change* only, not once per frame. See `redraw`'s own
    /// comment on why both of those are worth reporting at all (../PLAN.md §7.14).
    last_logged_frame_state: Option<(i32, i32, &'static str)>,
    /// Decoded bitmaps for every `Image` on screen, warm across frames (../PLAN.md
    /// §35). Rooted at `paths.app_dir` — the directory `ZKiteActivity` unpacks
    /// `app.zip` into, and the same one `zkite dev` pushes files into — so an
    /// `Image(src="logo.png")` resolves against the app's own files and nothing else
    /// (`zkite_imagestore` refuses a `src` that tries to leave it).
    ///
    /// `Box`ed and `Option`al only so `paint_gpu`/`paint_cpu_fallback` can take it as
    /// one borrow alongside `self`'s other fields; it is always `Some` after
    /// construction.
    images: Option<Box<zkite_imagestore::ImageStore>>,
    /// `(frames, total time, worst frame)` since the last summary log line — see
    /// `note_redraw_cost`.
    redraw_stats: (u32, Duration, Duration),
    /// Per-phase time over the same window as `redraw_stats` — see `FramePhases`.
    phase_stats: FramePhases,
    /// Per-phase worst case over the same window as `phase_stats` — see
    /// `FramePhases::take_max`.
    phase_worst: FramePhases,
    /// Total bytes of tree JSON returned by `sync_viewports` over the same window
    /// (§43) — zero on a window with no rebuilds.
    ///
    /// The denominator for `sync_py`/`sync_parse`. `phase_stats` divides by *frames*,
    /// which is the wrong unit for a cost that only some frames pay and that scales
    /// with how much tree crossed the boundary; dividing by the tree that actually
    /// crossed it gives a per-byte figure directly comparable to a host measurement
    /// of the same payload, which is what the ~55x host/device gap needs to be read
    /// against.
    rebuild_bytes: u64,
    /// `(frames whose `sync_viewports` reported a viewport, frames where that
    /// produced a whole new tree)` over the same window as `phase_stats`.
    ///
    /// The `sync` phase's *cause*, separated from its cost, because the two move
    /// independently and only one of them is actionable. `sync` time is dominated
    /// by re-entering CPython to run `view()` and re-parsing the JSON tree it
    /// returns; that happens only on the frames where a virtualised window
    /// genuinely moved, and the averages in `phase_stats` blend those frames with
    /// the cheap ones that reported a scroll and were told "nothing to do".
    ///
    /// Added by ../PLAN.md §40 for the same reason §39 added `worst:`: the
    /// mitigation being measured changes how *often* Python rebuilds, not how
    /// expensive one rebuild is, and no existing line could see a rate. Reading
    /// the two together also distinguishes the two ways `sync` can improve —
    /// fewer rebuilds (this) versus a cheaper rebuild (a smaller window).
    scroll_stats: (u32, u32),
    /// Where the dropped frames are, over the same window as `phase_stats` — see
    /// `BudgetStats`.
    budget_stats: BudgetStats,
    /// When this render thread started — i.e. immediately after `Native.boot`
    /// returned on the UI thread. The anchor for the cold-start half of ../PLAN.md
    /// §12.2 item 1: everything from here to the first painted frame is CPython
    /// init, font registration, the GPU/adapter probe, the first `view()` call, the
    /// first layout and the first paint. `ZKiteActivity` logs its own
    /// process-relative time at the same instant, so the two lines sum to
    /// process-start → first pixel without either side having to reason about the
    /// other's clock.
    thread_started: Instant,
    /// Set once the first frame has actually been painted, so the cold-start line is
    /// emitted exactly once rather than per frame.
    first_frame_logged: bool,
    retry_redraw: bool,
    /// End of the previous `redraw`, for the frame *interval* — which is a different
    /// number from the frame *cost* in `redraw_stats` and the only one directly
    /// comparable to the WebView backend (../PLAN.md §12.2 item 2).
    ///
    /// The distinction matters and is easy to collapse by accident: redraw cost is
    /// work this thread did, interval is wall-clock between frames reaching the
    /// screen. They coincide only when this thread is saturated. During a fling the
    /// loop deliberately waits on `FRAME_INTERVAL` between ticks, so the interval
    /// includes that idle time and the cost does not — reporting cost alone would
    /// claim a frame rate this backend does not actually deliver, and reporting
    /// interval alone would hide how much of it is ours to fix.
    last_frame_end: Option<Instant>,
    /// `(frames, total interval, worst interval)` over the same window as
    /// `redraw_stats`.
    interval_stats: (u32, Duration, Duration),
    /// When a tap started being acted on, for ../PLAN.md §12.2 item 3. Consumed by
    /// the first `redraw` that completes after it, which is the frame that shows the
    /// tap's result. `Option` rather than a plain `Instant` so a redraw triggered by
    /// anything else (a scroll, a fling tick, an IME pan) is never mistaken for one.
    tap_started: Option<Instant>,
    /// The animation driver (../PLAN.md §47, Queue F part 2) — layer 2 of §45.3's
    /// diagram. `zkite_anim` owns how a value moves through time and knows nothing
    /// about channels; this owns which node's which channel is moving, and when.
    animator: Animator,
}

/// The running animations, and the bookkeeping that makes a *declared* animation
/// start exactly once (../PLAN.md §47).
///
/// # Why this needs state `zkite_anim::Timeline` deliberately does not have
///
/// `Interrupt::Ignore` declines to restart an animation that is **still running**, and
/// documents that a *finished* entry does not block a new run — "a second open of the
/// same sheet animates again". That is the right rule for a spec that is sent, removed
/// and sent again. It is the wrong one here, because §45.3 principle 3 makes an
/// animation a *declaration* that stays in the tree: an entrance spec is still present
/// on the frame after it completes, so `Ignore` alone would start it over, forever.
///
/// So the gate is on the **declaration**, not on the running-ness: [`started`] records
/// what was last started for a key, and a key that has one is not started again unless
/// the declaration's target actually moved (`Trigger::Change`). `Ignore` stays as the
/// policy underneath it, as the second line of defence rather than the first.
///
/// [`started`]: Animator::started
#[derive(Default)]
struct Animator {
    timeline: zkite_anim::Timeline<AnimKey>,
    /// Per key: the `to` that was last *started*, and the trigger that started it.
    /// The `to` is what lets a `change` trigger tell "the target moved" from "the same
    /// declaration arrived again on the next of sixty frames"; the trigger is carried
    /// so the completion dispatched to Python can name it.
    started: FxHashMap<AnimKey, (f32, zkite_pybridge::Trigger)>,
    /// Completions collected during a frame and dispatched after it is painted.
    ///
    /// **Not dispatched where they are found.** `advance` runs inside `redraw`, and a
    /// Python handler is free to call `@store.update()`, which renders and re-enters
    /// this thread's frame path. Draining in the loop instead keeps `redraw`
    /// non-re-entrant, which every other dispatch site here already relies on.
    finished: Vec<(Arc<str>, zkite_renderer::Channel, zkite_pybridge::Trigger)>,
    /// The last malformed-declaration report, so a broken `anim` attribute — which is
    /// re-parsed on every rebuilt frame — is logged when it *changes* rather than
    /// sixty times a second. Same change-gating as the `frame:` line (../PLAN.md
    /// §7.14).
    last_error_report: Option<String>,
    /// Scratch reused across frames so the per-frame path allocates nothing once it
    /// has run once: the set of keys the current tree declares, used to evict entries
    /// for nodes that have left.
    live: FxHashSet<AnimKey>,
    /// Animations started since the last `ZKITEPERF` window, for the `anim:` line.
    stats_started: u32,
    /// Completions dispatched to Python since the last window.
    stats_completed: u32,
    /// Frames on which this driver, rather than a fling or a touch, is why the loop
    /// did not go idle. The number that says an animation was actually self-paced —
    /// §45's "the first workload this renderer has that is not input-paced".
    stats_driven_frames: u32,
}

/// A node's channel. `Arc<str>` rather than `NodeId` because taffy ids are minted
/// fresh by every `build` — the tree is rebuilt on every frame (`Renderer::clear`), so
/// a `NodeId` is valid for exactly one frame while a `zkite_py` node id is the thing
/// that persists across them. Every other cross-frame map here (`scroll_offsets`,
/// `reported_viewports`) is keyed the same way and for the same reason.
type AnimKey = (Arc<str>, zkite_renderer::Channel);

impl State {
    fn new(fonts: zkite_renderer::FontSet, paths: AppPaths) -> Result<Self, String> {
        // Taken before anything else this constructor does, because everything it
        // does — font registration, CPython init, the first `view()` call — is part
        // of the cold start being measured (../PLAN.md §12.2 item 1). Taking it in
        // the struct literal at the bottom instead would silently exclude the single
        // largest component of it.
        let thread_started = Instant::now();
        let renderer = Renderer::new(fonts)?;
        let touch_slop = paths.touch_slop;
        let scale = paths.scale;
        let frame_period = paths.frame_period;
        let log = |line: &str| crate::logi!("py: {line}");
        let current = match zkite_pybridge::setup(&paths.python_prefix, &paths.zkite_pkg_dir, &paths.app_dir, &log) {
            Ok(json) => match RawNode::from_json(&json) {
                Ok(tree) => tree,
                Err(e) => {
                    crate::loge!("pybridge setup returned malformed JSON: {e}");
                    boot_failed_tree(&e)
                }
            },
            Err(e) => {
                crate::loge!("pybridge setup failed: {e}");
                boot_failed_tree(&e)
            }
        };
        // Read once, here, while we are already on the thread that owns the
        // interpreter — `reload.rs`'s socket thread must never call into Python (its
        // module doc is explicit about why), and this value cannot change for the life
        // of the process. See ../PLAN.md §31.6.
        crate::set_framework_fingerprint(zkite_pybridge::framework_fingerprint(&paths.zkite_pkg_dir));
        // Attached here (not lazily on first focus) so a failure is visible in the
        // boot log rather than surfacing confusingly the first time a user taps a
        // `TextField`. A failure (no JavaVM, attach rejected) downgrades every
        // future focus/blur attempt to a no-op rather than aborting boot — the rest
        // of the app (everything not a `TextField`) works identically either way,
        // same "documented gap, not a crash" shape as `gpu`'s own fallible init just
        // below this in `handle`.
        let render_env = attach_current_thread(paths.java_vm as *mut jni_sys::JavaVM);
        if render_env.is_none() {
            crate::loge!("attach_current_thread failed; TextField focus/IME will be a no-op");
        }
        Ok(State {
            renderer,
            current,
            built: None,
            scroll_offsets: FxHashMap::default(),
            reported_viewports: FxHashMap::default(),
            gesture: None,
            fling: None,
            touch_slop,
            frame_period,
            last_frame_at: None,
            scale,
            width: 0,
            height: 0,
            window: None,
            window_generation: 0,
            gpu: None,
            gpu_probed: false,
            render_env,
            surface_view: None,
            last_system_bars: None,
            surface_view_methods: None,
            set_selection_method: None,
            finish_activity_method: None,
            selection_overlay_methods: None,
            selection_overlay_field: None,
            focused: None,
            focused_text: String::new(),
            focused_selection: (0, 0),
            ime_inset_bottom: 0.0,
            ime_pan_offset_y: 0.0,
            double_tap_slop: paths.double_tap_slop,
            double_tap_timeout: paths.double_tap_timeout,
            last_tap: None,
            a11y_adapter: None,
            is_accessibility_enabled_method: None,
            last_logged_frame_state: None,
            images: Some(Box::new(zkite_imagestore::ImageStore::new(&paths.app_dir))),
            redraw_stats: (0, Duration::ZERO, Duration::ZERO),
            thread_started,
            first_frame_logged: false,
            retry_redraw: false,
            last_frame_end: None,
            interval_stats: (0, Duration::ZERO, Duration::ZERO),
            tap_started: None,
            phase_stats: FramePhases::default(),
            phase_worst: FramePhases::default(),
            rebuild_bytes: 0,
            scroll_stats: (0, 0),
            budget_stats: BudgetStats::default(),
            animator: Animator::default(),
        })
    }

    /// `self.scroll_offsets` (persistent, zkite-id-keyed) resolved against `self`'s
    /// most recent build (ephemeral, taffy-`NodeId`-keyed), *plus* the current IME
    /// pan (../PLAN.md §5/§7.5) folded in as one more entry keyed on the tree's own
    /// root — what `positioned`/`hit_test` actually need to agree with whatever
    /// `redraw` last painted. Empty before the first `redraw()`. `ime_pan_offset_y`
    /// is derived state `redraw` recomputes every time (see that field's own doc
    /// comment); this just has to stay consistent with whatever `redraw` last set
    /// it to, so touch handling between redraws hit-tests the same panned layout
    /// that's actually on screen.
    fn node_scroll_offsets(&self) -> FxHashMap<NodeId, (f32, f32)> {
        match &self.built {
            Some(built) => {
                let mut offsets = built.scroll_offsets_for(&self.scroll_offsets);
                if self.ime_pan_offset_y != 0.0 {
                    offsets.insert(built.root, (0.0, self.ime_pan_offset_y));
                }
                offsets
            }
            None => FxHashMap::default(),
        }
    }

    fn handle(&mut self, cmd: Command) {
        match cmd {
            Command::Redraw => self.redraw(),
            Command::NewWindow { window, width, height, generation } => {
                if !crate::SURFACE_LIFECYCLE.is_current(generation) {
                    crate::logi!("NewWindow discarded: Android already replaced or destroyed it");
                    return;
                }
                self.width = width;
                self.height = height;
                if !self.gpu_probed {
                    self.gpu_probed = true;
                    match try_init_gpu() {
                        Ok(gpu) => {
                            crate::logi!("GPU backend initialized (wgpu/vello_hybrid)");
                            self.gpu = Some(gpu);
                        }
                        Err(e) => {
                            crate::logi!("GPU unavailable ({e}); using vello_cpu fallback");
                        }
                    }
                }
                // Initialization can outlive surfaceDestroyed's UI-thread wait.
                // Recheck without consuming any queued resize/input/destroy event.
                if !crate::SURFACE_LIFECYCLE.is_current(generation) {
                    crate::logi!("NewWindow expired during GPU initialization");
                    return;
                }
                crate::logi!("NewWindow {width}x{height}");
                if let Some(gpu) = &mut self.gpu {
                    // Deliberately clear any existing surface from an older window
                    // so we never attempt to bind or render to an expired surface.
                    gpu.surface = None;
                    gpu.hybrid_renderer = None;
                    ensure_surface(gpu, &window);
                    reconfigure(gpu, width, height, self.images.as_deref_mut());
                }
                self.window = Some(window);
                self.window_generation = generation;
                self.redraw();
            }
            Command::Resize { width, height } => {
                crate::logi!("Resize {width}x{height}");
                self.width = width;
                self.height = height;
                // A queued resize can precede DestroyWindow even though Android
                // has already invalidated the surface on the UI thread.
                if !crate::SURFACE_LIFECYCLE.is_current(self.window_generation) {
                    return;
                }
                // Deliberately *not* `ensure_surface` here too: the window from the
                // matching `NewWindow` is still live and its surface still bound to
                // it. A second `create_surface_unsafe` against the same
                // still-in-use `ANativeWindow` is what this session's second on-device
                // bug actually was (../PLAN.md §4) — Vulkan correctly refuses it
                // (`ERROR_NATIVE_WINDOW_IN_USE_KHR`), and the panic hook added for the
                // *first* bug this session is exactly what surfaced this one instead
                // of another silent black screen. `surfaceChanged` only ever means
                // "reconfigure," never "recreate."
                if let Some(gpu) = &mut self.gpu {
                    reconfigure(gpu, width, height, self.images.as_deref_mut());
                }
                self.redraw();
            }
            Command::Reload { ack } => {
                // Collect what the app printed *during* the reload rather than
                // logging it straight out, so `zkite dev`'s terminal gets it too —
                // plan1.md §6 wants a traceback in both places, not one.
                let collected = std::cell::RefCell::new(String::new());
                let log = |line: &str| {
                    crate::logi!("py: {line}");
                    collected.borrow_mut().push_str(line);
                    collected.borrow_mut().push('\n');
                };
                match zkite_pybridge::reload(&log) {
                    Ok(json) => match RawNode::from_json(&json) {
                        Ok(tree) => {
                            self.current = tree;
                            // Deliberately *not* clearing scroll offsets, focus, or
                            // the shape cache: keeping scroll position across a save
                            // is a phase-1 promise (plan1.md §6, "keyboard state and
                            // scroll position intact"), offsets are keyed by zkite_py
                            // node id so they survive a rebuilt taffy tree by
                            // construction (§6.2), and the shape cache is keyed by
                            // (content, style) so it self-heals — a changed string
                            // simply misses once (§11.9).
                            self.redraw();
                            crate::logi!("reload: applied, repainted");
                        }
                        Err(e) => crate::loge!("reload returned malformed JSON: {e}"),
                    },
                    Err(e) => crate::loge!("reload failed: {e}"),
                }
                let report = ReloadReport {
                    output: collected.into_inner(),
                    error: zkite_pybridge::last_error(),
                };
                // The socket thread may already have given up (10s timeout) or gone
                // away with the connection; a failed ack is not this thread's problem.
                let _ = ack.send(report);
            }
            Command::Back => {
                // One decision, made in exactly one place: `zkite_py`'s back chain
                // (../PLAN.md §16). If a navigator popped a screen it has already
                // re-rendered, so there is a fresh tree to paint; if nothing wanted
                // the press, the platform default is to exit, and this thread is the
                // one that has to say so because Java already consumed the event.
                let log = |line: &str| crate::logi!("py: {line}");
                match zkite_pybridge::back(&log) {
                    Some(json) => match RawNode::from_json(&json) {
                        Ok(tree) => {
                            crate::logi!("back: consumed by zkite_py, repainting");
                            self.current = tree;
                            // Blur first: a focused field on the screen being left
                            // would otherwise keep the keyboard up over the screen
                            // underneath, and keep `focused` pointing at a node id
                            // that no longer exists in the new tree.
                            if self.focused.is_some() {
                                self.blur_text_field();
                            }
                            self.redraw();
                        }
                        Err(e) => {
                            crate::loge!("back: bad tree JSON from zkite_py: {e}");
                        }
                    },
                    None => {
                        crate::logi!("back: not handled by zkite_py, finishing activity");
                        self.call_finish_activity();
                    }
                }
            }
            Command::DestroyWindow { ack } => {
                // Order matters: the wgpu `Surface` unsafely assumes the
                // `ANativeWindow` stays valid for its own lifetime, so it must be
                // dropped strictly before the `NativeWindow` that backs it (which, on
                // `Drop`, calls `ANativeWindow_release`). Android's own contract for
                // `surfaceDestroyed` requires exactly this: nothing may touch the
                // surface once the callback returns, so the ack is only sent after
                // both are gone.
                crate::logi!("DestroyWindow (had surface: {})", self.gpu.as_ref().is_some_and(|g| g.surface.is_some()));
                if let Some(gpu) = &mut self.gpu {
                    gpu.surface = None;
                    gpu.hybrid_renderer = None;
                }
                self.window = None;
                self.retry_redraw = false;
                // A fling ticking away with nothing on screen to show for it is
                // pure wasted wakeups (see `run()`'s recv_timeout switch) — the
                // window it was animating for is gone, and `redraw()` already no-ops
                // without one, so there's nothing left for it to accomplish.
                self.fling = None;
                let _ = ack.send(());
            }
            Command::TouchDown { x, y } => {
                let (x, y) = self.touch_to_dp(x, y);
                // A new touch always grabs whatever's currently in motion, same as
                // Android's own fling-cancel-on-ACTION_DOWN — a still-decelerating
                // list must stop dead under a landing finger, not keep coasting
                // underneath it.
                self.fling = None;
                let Some(built) = &self.built else { return };
                let offsets = self.node_scroll_offsets();
                // Resolve, once, everything this gesture might need: the deepest hit
                // node (for a tap, if it turns out to be one) and the nearest
                // scrollable ancestor above it (for a drag, if it turns out to be
                // one) — found the same way a tap target is, by walking up from the
                // deepest hit via `Renderer::parent` (see that method's doc comment).
                let hit = self.renderer.hit_test(built.root, x, y, &offsets);

                // A `Slider` under the finger claims the gesture outright, before the
                // scroll ancestor is even looked for — see `Gesture::slider` for why
                // this is a capture rather than a direction test. `positioned` is
                // needed for the slider's own rect and is only computed on that path,
                // so an ordinary touch pays nothing for this.
                let slider = hit
                    .and_then(|n| built.control_at(&self.renderer, n))
                    .and_then(|(zkite_id, control, node)| match control {
                        zkite_pybridge::Control::Slider { min, max, step, value } => {
                            let positioned = self.renderer.positioned(built.root, &offsets);
                            let rect = positioned.iter().find(|p| p.node == node)?.rect;
                            Some(SliderDrag { target: zkite_id, rect, min, max, step, last_value: value })
                        }
                        // Neither toggle captures the gesture: they commit on release
                        // via `dispatch_tap`, not on touch-down like a slider (§26.3).
                        zkite_pybridge::Control::Switch { .. } => None,
                        zkite_pybridge::Control::Radio { .. } => None,
                    });

                // Resolve a target per axis, not one target and an axis: from a touch
                // on a card inside a horizontal strip inside a vertical list, the two
                // answers are two different containers. `scrollable_ancestor` filters
                // on *room*, not on the `overflow` declaration — `overflow-auto` is
                // `Axis::Both`, so every `ScrollView` that has ever existed declares
                // itself x-scrollable and would otherwise swallow every horizontal
                // drag. See `Renderer::is_scrollable`'s doc comment.
                let (mut scroll_x, mut scroll_y) = (None, None);
                if slider.is_none() {
                    if let Some(hit) = hit {
                        let resolve = |axis| {
                            self.renderer.scrollable_ancestor(hit, axis).and_then(|(n, max)| {
                                built.zkite_id(n).map(|id| ScrollTarget { id: Arc::<str>::from(id), max })
                            })
                        };
                        scroll_x = resolve(zkite_renderer::ScrollAxis::X);
                        scroll_y = resolve(zkite_renderer::ScrollAxis::Y);
                    }
                }
                let now = Instant::now();
                self.gesture = Some(Gesture {
                    down: (x, y),
                    down_time: now,
                    last: (x, y),
                    dragging: false,
                    scroll_x,
                    scroll_y,
                    axis: None,
                    history: vec![(now, x, y)],
                    slider: slider.clone(),
                });
                // Touching a slider's track jumps to that position immediately,
                // before any movement — the same thing a real `<input type=range>`
                // and every native slider do, and what makes a plain *tap* on the
                // track work without a second code path for it.
                if slider.is_some() {
                    self.update_slider_from_touch(x);
                }
            }
            Command::TouchMove { x, y } => {
                let (x, y) = self.touch_to_dp(x, y);
                let Some(gesture) = &mut self.gesture else { return };
                let now = Instant::now();
                gesture.history.push((now, x, y));
                let cutoff = now - VELOCITY_WINDOW;
                gesture.history.retain(|(t, _, _)| *t >= cutoff);

                if !gesture.dragging {
                    let (dx, dy) = (x - gesture.down.0, y - gesture.down.1);
                    if dx.hypot(dy) > self.touch_slop {
                        gesture.dragging = true;
                        // Lock the axis at the same instant, from the movement that
                        // crossed slop — the first moment anything could move, and
                        // therefore the first moment the answer is both knowable and
                        // needed. See `Gesture::axis` for why this is deferred where
                        // `Gesture::slider` is captured.
                        //
                        // Ties go to Y. A tie is a perfect 45° drag, which is not a
                        // thing a finger produces on purpose; biasing to the axis that
                        // has always worked keeps a genuinely ambiguous gesture doing
                        // what it did before horizontal scrolling existed.
                        gesture.axis = Some(if dx.abs() > dy.abs() {
                            zkite_renderer::ScrollAxis::X
                        } else {
                            zkite_renderer::ScrollAxis::Y
                        });
                    }
                }
                // A captured slider tracks the finger from the first move, without
                // waiting for `touch_slop` — the position was already committed at
                // touch-down, so making the *next* pixel wait for slop would make the
                // control feel like it sticks. `scroll_target` is `None` here by
                // construction (see `Gesture::slider`), so nothing scrolls.
                if gesture.slider.is_some() {
                    gesture.last = (x, y);
                    self.update_slider_from_touch(x);
                    return;
                }
                if !gesture.dragging {
                    return;
                }
                let axis = gesture.axis.unwrap_or(zkite_renderer::ScrollAxis::Y);
                let target = match axis {
                    zkite_renderer::ScrollAxis::X => gesture.scroll_x.clone(),
                    zkite_renderer::ScrollAxis::Y => gesture.scroll_y.clone(),
                };
                let Some(target) = target else {
                    gesture.last = (x, y);
                    return;
                };
                // Dragging the finger up/left (the coordinate decreasing) reveals
                // content below/right of it, i.e. increases the scroll offset — the
                // standard "content follows the finger" touch-scroll convention, and
                // identical on both axes.
                let delta = match axis {
                    zkite_renderer::ScrollAxis::X => gesture.last.0 - x,
                    zkite_renderer::ScrollAxis::Y => gesture.last.1 - y,
                };
                let max = target.max;
                let entry = self.scroll_offsets.entry(target.id).or_insert((0.0, 0.0));
                let slot = match axis {
                    zkite_renderer::ScrollAxis::X => &mut entry.0,
                    zkite_renderer::ScrollAxis::Y => &mut entry.1,
                };
                *slot = (*slot + delta).clamp(0.0, max);
                gesture.last = (x, y);
                self.redraw();
            }
            Command::TouchUp { x, y } => {
                let (x, y) = self.touch_to_dp(x, y);
                if self.gesture.as_ref().is_some_and(|g| g.slider.is_some()) {
                    self.update_slider_from_touch(x);
                }
                let Some(gesture) = self.gesture.take() else { return };
                // A slider owned this gesture: the value is already committed, and
                // releasing must not also fling (there is nothing to fling — nothing
                // scrolled), long-press (a held thumb is not a text selection), or
                // tap (which would blur a focused field and dispatch to an ancestor).
                if gesture.slider.is_some() {
                    return;
                }
                if gesture.dragging {
                    self.maybe_start_fling(gesture);
                    return;
                }
                if gesture.down_time.elapsed() >= LONG_PRESS_THRESHOLD {
                    self.dispatch_long_press(x, y);
                    return;
                }
                self.dispatch_tap(x, y);
            }
            Command::TouchCancel => {
                // Explicitly not `maybe_start_fling` — `ACTION_CANCEL` means this
                // gesture didn't complete normally (see `Command::TouchCancel`'s own
                // doc comment), and a fling launched from a cancelled gesture would
                // be exactly the "maybe a tap" mistake that comment already warns
                // `TouchUp` itself must not make, one level up.
                self.gesture = None;
            }
            Command::SurfaceViewReady(view) => {
                self.surface_view = Some(view);
                self.last_system_bars = None;
                self.sync_system_bars();
            }
            Command::TextInputEvent(new_state) => {
                // `lib.rs`'s `Native_onTextInputEvent` already decoded this on the
                // UI thread (see `text_input.rs`'s module doc for why `GameTextInput`
                // itself never reaches this thread) — nothing JNI-flavored left to do
                // here, just ordinary state bookkeeping.
                let Some(zkite_id) = self.focused.clone() else { return };
                let text_changed = self.focused_text != new_state.text;
                let selection = (new_state.selection_start, new_state.selection_end);
                if !text_changed && self.focused_selection == selection {
                    return;
                }
                self.focused_text = new_state.text;
                self.focused_selection = selection;
                // IMEs also report selection/composition changes and echo setState.
                // Those must not re-run application change handlers for unchanged text.
                if text_changed {
                    let text = self.focused_text.clone();
                    self.dispatch_text_edit(&zkite_id, &text);
                }
                self.redraw();
            }
            Command::KeyboardDismissed => {
                // The system/user already dismissed it — just drop our own focus
                // bookkeeping and redraw (so the caret/placeholder-vs-value display
                // catches up), no `blurAndHideKeyboard` call: there is nothing left
                // to hide.
                if self.focused.take().is_some() {
                    self.redraw();
                }
            }
            Command::SelectionOverlayChanged { text, start, end } => {
                // Only meaningful while the overlay is actually showing — see
                // `selection_overlay_field`'s own doc comment. A stray/delayed event
                // after dismiss (shouldn't happen, but this thread has no way to
                // cancel an in-flight Java-side callback) is ignored rather than
                // corrupting whatever field is focused *now*.
                if self.selection_overlay_field.is_none() {
                    return;
                }
                if self.focused != self.selection_overlay_field { return; }
                let changed = self.focused_text != text;
                self.focused_text = text;
                self.focused_selection = (start, end);
                if changed {
                    let id = self.selection_overlay_field.clone().unwrap();
                    let value = self.focused_text.clone();
                    self.dispatch_text_edit(&id, &value);
                }
                self.redraw();
            }
            Command::SelectionOverlayDismissed { text, start, end } => {
                let Some(zkite_id) = self.selection_overlay_field.take() else { return };
                // Only apply to `focused_text`/`focused_selection` if focus hasn't
                // already moved on to a *different* field — see `show_selection_
                // overlay`'s doc comment on the race this guards: tapping field B
                // while field A's overlay was still up steals Android focus (which
                // is what triggers *this* dismiss), but `focus_text_field(B)` always
                // runs first and already owns those two fields by the time this
                // arrives. A's final edit must still reach Python either way.
                let changed = self.focused_text != text;
                if self.focused.as_deref() == Some(zkite_id.as_ref()) {
                    self.focused_text = text.clone();
                    self.focused_selection = (start, end);
                }
                if changed { self.dispatch_text_edit(&zkite_id, &text); }
                self.redraw();
            }
            Command::ImeInsetsChanged { bottom } => {
                // A no-op-ish update when nothing's focused (redraw's own pan
                // computation already yields 0.0 in that case) still redraws —
                // cheap, and simpler than tracking whether this actually needs to
                // change anything visible.
                //
                // Converted to dp on arrival (../PLAN.md §13): this is a real system
                // window inset, so it arrives in device px, and its only consumer
                // (`compute_ime_pan`) compares it against a dp layout and produces a
                // dp pan offset. Storing it in the space it is *used* in, at the
                // boundary it crosses, keeps that comparison honest by construction.
                self.ime_inset_bottom = bottom / self.scale;
                self.redraw();
            }
            Command::FrameReady { tree_json } => {
                match RawNode::from_json(&tree_json) {
                    Ok(tree) => {
                        // Exactly what `Command::Reload` does with a new tree, and for
                        // the same reasons: scroll offsets, IME focus and the shape
                        // cache are all keyed by zkite_py node id or by (content,
                        // style), so they survive a rebuilt tree by construction. A
                        // background fetch landing must not scroll the user's list
                        // back to the top or close their keyboard.
                        self.current = tree;
                        self.redraw();
                    }
                    Err(e) => crate::loge!("pushed frame was malformed JSON: {e}"),
                }
                // A background thread's `print()` has been sitting in `runtime.py`'s
                // stdout tee with nothing to flush it — every other path drains as
                // part of the call it made, and this path never made one. Draining
                // here is what makes a `print()` inside a worker thread reach logcat
                // at all, which is the only debugging instrument such a thread has.
                let log = |line: &str| crate::logi!("py: {line}");
                zkite_pybridge::drain_log(&log);
            }
            Command::A11yAction(request) => self.handle_a11y_action(request),
            Command::Stop => unreachable!("handled in run()"),
        }
    }

    /// A `TextField`'s text just changed — tell Python, as both `"input"` and
    /// `"change"`, in that order (../PLAN.md §34.1).
    ///
    /// **Why two events for one edit, and why this order.** `on_input` was registered
    /// by `_register_events` on both backends and dispatched by neither on native
    /// (§26.7), so a developer writing a search-as-you-type field had to register
    /// `on_change` for native and `on_input` for the WebView and hope. Driven against
    /// real Chromium rather than reasoned about (`test/probe_input_change.mjs`), the
    /// contract that backend actually has is: typing `Ken` into a field emits
    /// **`input` three times and `change` not at all**, and `change` arrives once, on
    /// blur. `input` before `change` is the DOM's own order for any single value
    /// transition, so a handler bound to both sees them in the same sequence here.
    ///
    /// **What this deliberately does not do is retime `change`.** Native has fired
    /// `change` per keystroke since §7, WIDGETS.md documents `on_change` as the way to
    /// get live typing on this backend, and §33.6's own typeahead is built on it — so
    /// moving it to blur would silently break working apps on the default backend to
    /// fix a divergence that has never bitten anyone. `input` is purely additive: a
    /// field that never registered one dispatches exactly what it dispatched before.
    /// The residual timing divergence is real and now named rather than latent —
    /// §15.1, with the contract question written up in §34.7 for the owner.
    ///
    /// Each dispatch is skipped unless the field actually declared that handler; an
    /// unhandled dispatch is the normal case (see `zkite_pybridge::dispatch`'s own doc
    /// comment), not an error, and the live text is on screen either way (see
    /// `zkite_pybridge::FocusedField`).
    fn dispatch_text_edit(&mut self, zkite_id: &str, text: &str) {
        for event in ["input", "change"] {
            if !self.built.as_ref().is_some_and(|b| b.handles(zkite_id, event)) {
                continue;
            }
            let log = |line: &str| crate::logi!("py: {line}");
            if let Some(json) = zkite_pybridge::dispatch(zkite_id, event, text, &log) {
                match RawNode::from_json(&json) {
                    Ok(tree) => self.current = tree,
                    Err(e) => crate::loge!("dispatch({zkite_id}, {event}) returned malformed JSON: {e}"),
                }
            }
        }
    }

    /// Focuses `zkite_id` (a `TextField`'s own node id) and asks
    /// `ZKiteSurfaceView` to request view focus + show the keyboard — a no-op if it's
    /// already the focused field (repeated taps on the same field shouldn't restart
    /// an edit or re-seed live text from a possibly-stale `RawNode` value). See
    /// ../PLAN.md §5.
    fn focus_text_field(&mut self, zkite_id: &str) {
        if self.focused.as_deref() == Some(zkite_id) {
            return;
        }
        self.focused = Some(Arc::from(zkite_id));
        self.focused_text = self.current_text_field_value(zkite_id);
        // End-of-text default — overwritten immediately after by `dispatch_tap`
        // calling `set_cursor_position` with the actual tap offset for every real
        // focus (a tap is the only way to focus a field at all right now). Kept as
        // a sane fallback rather than `(0, 0)` in case that ever isn't true.
        let len = self.focused_text.len() as i32;
        self.focused_selection = (len, len);
        let input_type = self.current_text_field_input_type(zkite_id);
        self.call_focus_method(&input_type);
        self.redraw();
    }

    /// Clears focus (if any) and asks `ZKiteSurfaceView` to hide the keyboard + drop
    /// view focus. Safe to call when nothing is focused (a tap that lands on empty
    /// space always calls this — see `dispatch_tap`).
    fn blur_text_field(&mut self) {
        if self.focused.take().is_some() {
            // Always called, not gated on `selection_overlay_field` — `hide()` is
            // idempotent (a no-op if nothing's showing, see `TextSelectionOverlay`),
            // and this is the only path a blank-space tap has to dismiss an overlay
            // left open on a field that tap didn't land on (a tap *inside* the
            // overlay's own bounds never reaches this thread at all — Android routes
            // it straight to the overlay, the topmost view there — so this is only
            // ever reached for a tap genuinely outside it).
            self.call_hide_selection_overlay_method();
            self.call_blur_method();
            self.redraw();
        }
    }

    /// After a tap has already focused (or re-tapped) a `TextField` (see
    /// `dispatch_tap`), hit-tests the *same* tap point against that field's own
    /// positioned text node and asks `GameTextInput` to move the cursor there — the
    /// tap-to-position-cursor slice of ../PLAN.md §7.7. Re-derives `positioned()`
    /// rather than reusing `redraw()`'s own (already-discarded-by-return) copy —
    /// a second tree walk over already-computed layout, not a re-shape of
    /// anything, and simpler than threading `nodes` out of `redraw()` for what's
    /// still a single, infrequent, user-gesture-driven call. A no-op if the
    /// field's text node isn't in the current build for any reason (shouldn't
    /// happen immediately after `focus_text_field`'s own `redraw()`, but cheap to
    /// guard rather than assume).
    fn position_cursor_from_tap(&mut self, x: f32, y: f32) {
        let Some(built) = &self.built else { return };
        let Some(text_node) = built.focused_text_node else { return };
        let offsets = self.node_scroll_offsets();
        let nodes = self.renderer.positioned(built.root, &offsets);
        let Some(n) = nodes.iter().find(|n| n.node == text_node) else { return };
        let (node_x, node_y, _w, _h) = n.rect;
        let style = n.style.text.clone();
        let content = self.focused_display_text();
        let offset = self.renderer.hit_test_text_offset(&style, &content, x - node_x, y - node_y);
        self.set_cursor_position(offset);
    }

    /// Pushes a new `[offset, offset)` selection into the focused field's live
    /// `Editable` (via `ZKiteSurfaceView.setSelection`, which always carries the
    /// field's real current text along — see that method's own doc comment) and
    /// updates this thread's own idea of the caret immediately, since no
    /// `TextInputEvent` follows `GameTextInput_setState` to confirm it (see
    /// `text_input.rs`'s module doc). A no-op if nothing is focused.
    fn set_cursor_position(&mut self, offset: usize) {
        if self.focused.is_none() {
            return;
        }
        let offset = offset.min(self.focused_text.len()) as i32;
        self.focused_selection = (offset, offset);
        self.call_set_selection_method();
        self.redraw();
    }

    /// The JNI half of `set_cursor_position` — `ZKiteSurfaceView.setSelection`'s
    /// `(Ljava/lang/String;II)V` signature needs a real Java `String` built from
    /// `self.focused_text`, a different signature from either `call_focus_method`'s
    /// (a single `input_type` string) or `call_blur_method`'s (zero-arg), so this
    /// doesn't reuse either. Same posture as
    /// `call_focus_method`/`call_blur_method` on every failure mode (missing env/view/method
    /// lookup, a pending exception after the call): logged, not fatal — a tap that
    /// can't reposition the cursor leaves the field exactly as usable as it was
    /// before the tap, never crashes.
    fn call_set_selection_method(&mut self) {
        let Some(env) = self.render_env else {
            crate::loge!("call_set_selection_method: no attached JNIEnv (see boot log)");
            return;
        };
        let Some(view) = self.surface_view else {
            crate::loge!("call_set_selection_method: ZKiteSurfaceView not ready yet");
            return;
        };
        if self.set_selection_method.is_none() {
            self.set_selection_method = resolve_set_selection_method(env, view.0);
        }
        let Some(mid) = self.set_selection_method else {
            crate::loge!("call_set_selection_method: method lookup failed (see log above)");
            return;
        };
        unsafe {
            let vtable = **env;
            let Some(new_string_utf) = vtable.NewStringUTF else {
                crate::loge!("call_set_selection_method: no NewStringUTF in vtable");
                return;
            };
            let Ok(c_text) = std::ffi::CString::new(cesu8::to_java_cesu8(&self.focused_text).as_ref()) else {
                // Only fails on an embedded NUL byte, which no real IME-produced
                // text can contain — defensive, not expected to ever trigger.
                crate::loge!("call_set_selection_method: focused_text has an embedded NUL");
                return;
            };
            let jtext = new_string_utf(env, c_text.as_ptr());
            if jtext.is_null() {
                crate::loge!("call_set_selection_method: NewStringUTF failed");
                if let Some(clear) = vtable.ExceptionClear {
                    clear(env);
                }
                return;
            }
            let (start, end) = self.focused_selection;
            let start = zkite_pybridge::utf8_to_utf16_offset(&self.focused_text, start);
            let end = zkite_pybridge::utf8_to_utf16_offset(&self.focused_text, end);
            if let Some(call) = vtable.CallVoidMethodA {
                let args =
                    [jni_sys::jvalue { l: jtext }, jni_sys::jvalue { i: start }, jni_sys::jvalue { i: end }];
                call(env, view.0, mid, args.as_ptr());
            }
            if let Some(del) = vtable.DeleteLocalRef {
                del(env, jtext);
            }
            if let Some(check) = vtable.ExceptionCheck {
                if check(env) != 0 {
                    crate::loge!("call_set_selection_method: pending exception after call");
                    if let Some(describe) = vtable.ExceptionDescribe {
                        describe(env);
                    }
                    if let Some(clear) = vtable.ExceptionClear {
                        clear(env);
                    }
                }
            }
        }
    }

    /// A long-press (§7.8's own `LONG_PRESS_THRESHOLD` check, in `TouchUp`) that
    /// lands on a `TextField` shows the native selection overlay, seeded with the
    /// word touching the press point — same initial-selection behavior every native
    /// text view gives for free. A long-press anywhere else (a button, blank space)
    /// has no special meaning here and falls back to plain-tap handling, same as an
    /// ordinary Android view with no long-click listener registered treats one.
    fn dispatch_long_press(&mut self, x: f32, y: f32) {
        let Some(built) = &self.built else { return };
        let offsets = self.node_scroll_offsets();
        let mut node = self.renderer.hit_test(built.root, x, y, &offsets);
        let mut text_field: Option<String> = None;
        while let Some(n) = node {
            if let Some(zkite_id) = built.zkite_id(n) {
                if built.is_text_field(zkite_id) {
                    text_field = Some(zkite_id.to_string());
                }
                break;
            }
            node = self.renderer.parent(n);
        }
        let Some(field_id) = text_field else {
            self.dispatch_tap(x, y);
            return;
        };
        crate::logi!("Touch ({x}, {y}) -> long-press on text field {field_id}");
        self.focus_text_field(&field_id);
        self.select_word_at(x, y);
        // A long-press is its own gesture, not a tap — it must not leave a stale
        // `last_tap` behind to pair with some future unrelated tap into an
        // unintended double-tap.
        self.last_tap = None;
    }

    /// Hit-tests `(x, y)` against the *already-focused* field's text and shows the
    /// selection overlay seeded with the word touching that point — the shared
    /// tail of both `dispatch_long_press` (../PLAN.md §7.8) and double-tap-to-
    /// select (`dispatch_tap`, ../PLAN.md §5/§7.5 — the gesture §7.7/§7.8's own
    /// known-gaps notes flagged as missing even though the hit-testing primitive
    /// it needed already existed). Caller must already have called
    /// `focus_text_field` for whatever field `(x, y)` landed on — this only reads
    /// `built.focused_text_node`, it doesn't itself resolve which field was
    /// touched.
    fn select_word_at(&mut self, x: f32, y: f32) {
        let Some(built) = &self.built else { return };
        let Some(text_node) = built.focused_text_node else { return };
        let offsets = self.node_scroll_offsets();
        let nodes = self.renderer.positioned(built.root, &offsets);
        let Some(n) = nodes.iter().find(|n| n.node == text_node) else { return };
        let (node_x, node_y, _w, _h) = n.rect;
        let style = n.style.text.clone();
        // Masked for a `secure=True` field (../PLAN.md §5/§7.5), same as every
        // other geometry call in this file — see `focused_display_text`'s doc
        // comment. `word_range_at` degrades to a zero-width selection at the tap
        // point on masked content (`*` isn't a "word" byte), which is an accepted,
        // documented outcome for a password field, not a bug: there's no
        // meaningful notion of a "word" inside a run of mask characters, and the
        // resulting selection is still expandable via the overlay's own drag
        // handles once shown.
        let content = self.focused_display_text();
        let offset = self.renderer.hit_test_text_offset(&style, &content, x - node_x, y - node_y);
        let (start, end) = word_range_at(&content, offset);
        self.show_selection_overlay(start as i32, end as i32);
    }

    /// Whether a tap landing on `field_id` at `(x, y)` pairs with `self.last_tap`
    /// into a double-tap (../PLAN.md §5/§7.5) — both real platform constants
    /// (`double_tap_slop`/`double_tap_timeout`), not guessed ones, same as
    /// `touch_slop`. Same field *and* within both thresholds — a fast second tap
    /// on a *different* field is two independent single taps, not a double-tap on
    /// either.
    fn is_double_tap(&self, field_id: &str, x: f32, y: f32) -> bool {
        let Some((t, last_field, lx, ly)) = &self.last_tap else { return false };
        if last_field.as_ref() != field_id {
            return false;
        }
        if t.elapsed() > self.double_tap_timeout {
            return false;
        }
        (x - lx).hypot(y - ly) <= self.double_tap_slop
    }

    /// Shows the native-`EditText` selection overlay (../PLAN.md §7.8) over the
    /// focused field's own screen rect, seeded with `[start, end)` (already
    /// word-expanded by `dispatch_long_press`) — Android's own selection handles and
    /// copy/paste toolbar take over from there until the overlay is dismissed
    /// (`Command::SelectionOverlayDismissed`). Reads the field's *container* node's
    /// rect/background for positioning — not `focused_text_node`'s own rect, which
    /// (see `zkite_pybridge::build_node`'s `is_text_field` branch) is only the tight
    /// box around the shaped glyphs, sized and placed identically to `caret_rect`'s
    /// own local-coordinate math but with no padding/background of its own; the
    /// overlay needs to visually cover the *whole* rounded field box the user
    /// actually sees, which lives one level up, on the text node's own parent
    /// container. A no-op if nothing is focused or either node isn't in the current
    /// build (mirrors `position_cursor_from_tap`'s own guards).
    ///
    /// **Passes the real, unmasked text** to the overlay (unlike every glyph-
    /// geometry call in this file — see `focused_display_text`'s doc comment) —
    /// deliberately: the overlay is a real `android.widget.EditText`, and a
    /// `secure=True` field's masking belongs on *it*, the same way it would on any
    /// other password `EditText`, via a real `TYPE_TEXT_VARIATION_PASSWORD`
    /// (`call_show_selection_overlay_method`'s `is_password` argument) rather than
    /// by feeding it pre-masked `*`s — the latter would make "Select All" + "Copy"
    /// copy literal asterisks to the clipboard instead of the real password, and
    /// would forgo Android's own default of hiding Cut/Copy for a password
    /// `EditText` in the first place.
    fn show_selection_overlay(&mut self, start: i32, end: i32) {
        if self.focused.is_none() {
            return;
        }
        let Some(built) = &self.built else { return };
        let Some(text_node) = built.focused_text_node else { return };
        let Some(container_node) = self.renderer.parent(text_node) else { return };
        let offsets = self.node_scroll_offsets();
        let nodes = self.renderer.positioned(built.root, &offsets);
        let Some(text_n) = nodes.iter().find(|n| n.node == text_node) else { return };
        let Some(container_n) = nodes.iter().find(|n| n.node == container_node) else { return };
        let (x, y, w, h) = container_n.rect;
        let text_size = text_n.style.text.size;
        let text_color = text_n.style.text.color.unwrap_or(zkite_renderer::Color::WHITE);
        // A field with no declared background would otherwise leave the overlay
        // transparent, showing whatever this thread's own GPU paint left underneath
        // through it — fine for ordinary paint (`None` there just means "skip the
        // background rect," never seen as a gap), wrong here, where the overlay is
        // meant to visually *replace* the field entirely while shown. The fallback
        // is this app's own dark theme background (matching every screenshot in
        // ../PLAN.md's IME sections), not an arbitrary color.
        let bg_color = container_n.style.paint.background.unwrap_or(zkite_renderer::Color { r: 15, g: 23, b: 42, a: 255 });
        self.focused_selection = (start, end);
        self.selection_overlay_field = self.focused.clone();
        let is_password = self
            .focused
            .as_deref()
            .is_some_and(|zkite_id| self.current_text_field_input_type(zkite_id) == "password");
        // Back to device pixels (../PLAN.md §13). The overlay is a real
        // `android.widget.EditText` positioned by `FrameLayout.LayoutParams` margins
        // in its parent's own pixel space, and `setTextSize(COMPLEX_UNIT_PX, ..)`
        // takes pixels too — none of which know anything about this renderer's dp
        // layout. Left in dp it would appear at `1/density` size and position, in the
        // top-left corner region of the field it is supposed to cover exactly (§7.8's
        // whole design premise is that occlusion is exact, not approximate).
        let s = self.scale;
        let radius = container_n.style.paint.radius;
        let baseline = text_n.text.as_ref().and_then(|runs| runs.first())
            .and_then(|run| run.glyphs.first()).map(|g| g.y).unwrap_or(text_size);
        let geometry = serde_json::json!({
            "left": (text_n.rect.0 - x) * s,
            "baseline": (text_n.rect.1 - y + baseline) * s,
            "right": (x + w - text_n.rect.0 - text_n.rect.2).max(0.0) * s,
            "font": format!("{:?}", text_n.style.text.family).to_lowercase(),
            "bold": text_n.style.text.weight >= 600,
            "italic": text_n.style.text.italic,
            "spacing": text_n.style.text.letter_spacing,
            "radii": [radius.top_left*s, radius.top_right*s, radius.bottom_right*s, radius.bottom_left*s]
        }).to_string();
        self.call_show_selection_overlay_method(
            x * s,
            y * s,
            w * s,
            h * s,
            text_size * s,
            text_color,
            bg_color,
            start,
            end,
            is_password,
            &geometry,
        );
    }

    /// The JNI half of `show_selection_overlay` — `ZKiteSurfaceView.
    /// showSelectionOverlay`'s eleven-parameter signature needs a real Java
    /// `String` (same `NewStringUTF` dance as `call_set_selection_method`) plus a
    /// packed `jvalue` array mixing `jobject`/`jint`/`jfloat`/`jboolean`. Same
    /// failure posture as every other JNI call in this file: logged, never fatal.
    #[allow(clippy::too_many_arguments)]
    fn call_show_selection_overlay_method(
        &mut self,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        text_size_px: f32,
        text_color: zkite_renderer::Color,
        bg_color: zkite_renderer::Color,
        start: i32,
        end: i32,
        is_password: bool,
        geometry: &str,
    ) {
        let Some(env) = self.render_env else {
            crate::loge!("call_show_selection_overlay_method: no attached JNIEnv (see boot log)");
            return;
        };
        let Some(view) = self.surface_view else {
            crate::loge!("call_show_selection_overlay_method: ZKiteSurfaceView not ready yet");
            return;
        };
        if self.selection_overlay_methods.is_none() {
            self.selection_overlay_methods = resolve_selection_overlay_methods(env, view.0);
        }
        let Some((show_mid, _)) = self.selection_overlay_methods else {
            crate::loge!("call_show_selection_overlay_method: method lookup failed (see log above)");
            return;
        };
        unsafe {
            let vtable = **env;
            let Some(new_string_utf) = vtable.NewStringUTF else {
                crate::loge!("call_show_selection_overlay_method: no NewStringUTF in vtable");
                return;
            };
            let Ok(c_text) = std::ffi::CString::new(cesu8::to_java_cesu8(&self.focused_text).as_ref()) else {
                crate::loge!("call_show_selection_overlay_method: focused_text has an embedded NUL");
                return;
            };
            let jtext = new_string_utf(env, c_text.as_ptr());
            if jtext.is_null() {
                crate::loge!("call_show_selection_overlay_method: NewStringUTF failed");
                if let Some(clear) = vtable.ExceptionClear {
                    clear(env);
                }
                return;
            }
            let c_geometry = std::ffi::CString::new(geometry).unwrap();
            let jgeometry = new_string_utf(env, c_geometry.as_ptr());
            let start = zkite_pybridge::utf8_to_utf16_offset(&self.focused_text, start);
            let end = zkite_pybridge::utf8_to_utf16_offset(&self.focused_text, end);
            if let Some(call) = vtable.CallVoidMethodA {
                let args = [
                    jni_sys::jvalue { l: jtext },
                    jni_sys::jvalue { i: start },
                    jni_sys::jvalue { i: end },
                    jni_sys::jvalue { f: x },
                    jni_sys::jvalue { f: y },
                    jni_sys::jvalue { f: w },
                    jni_sys::jvalue { f: h },
                    jni_sys::jvalue { f: text_size_px },
                    jni_sys::jvalue { i: pack_argb(text_color) },
                    jni_sys::jvalue { i: pack_argb(bg_color) },
                    jni_sys::jvalue { z: if is_password { jni_sys::JNI_TRUE } else { jni_sys::JNI_FALSE } },
                    jni_sys::jvalue { l: jgeometry },
                ];
                call(env, view.0, show_mid, args.as_ptr());
            }
            if let Some(del) = vtable.DeleteLocalRef {
                del(env, jtext);
                del(env, jgeometry);
            }
            if let Some(check) = vtable.ExceptionCheck {
                if check(env) != 0 {
                    crate::loge!("call_show_selection_overlay_method: pending exception after call");
                    if let Some(describe) = vtable.ExceptionDescribe {
                        describe(env);
                    }
                    if let Some(clear) = vtable.ExceptionClear {
                        clear(env);
                    }
                }
            }
        }
    }

    /// The JNI half of `blur_text_field`'s overlay-dismiss — zero-arg, same posture
    /// as `call_focus_method`/`call_blur_method`. Safe to call whether or not the overlay is
    /// currently showing (`TextSelectionOverlay.hide` is idempotent).
    fn call_hide_selection_overlay_method(&mut self) {
        let Some(env) = self.render_env else { return };
        let Some(view) = self.surface_view else { return };
        if self.selection_overlay_methods.is_none() {
            self.selection_overlay_methods = resolve_selection_overlay_methods(env, view.0);
        }
        let Some((_, hide_mid)) = self.selection_overlay_methods else { return };
        unsafe {
            let vtable = **env;
            if let Some(call) = vtable.CallVoidMethodA {
                call(env, view.0, hide_mid, std::ptr::null());
            }
            if let Some(check) = vtable.ExceptionCheck {
                if check(env) != 0 {
                    crate::loge!("call_hide_selection_overlay_method: pending exception after call");
                    if let Some(describe) = vtable.ExceptionDescribe {
                        describe(env);
                    }
                    if let Some(clear) = vtable.ExceptionClear {
                        clear(env);
                    }
                }
            }
        }
    }

    /// `zkite_id`'s `value` attr in the *current* tree — seeds `focused_text` when a
    /// field first gains focus, so typing continues from whatever `main.py`'s last
    /// render actually showed rather than starting blank. A plain recursive search
    /// (no id index on `RawNode` — trees are small, and this only runs once per
    /// focus, not per frame).
    fn current_text_field_value(&self, zkite_id: &str) -> String {
        fn find<'a>(node: &'a RawNode, zkite_id: &str) -> Option<&'a RawNode> {
            if node.id.as_deref() == Some(zkite_id) {
                return Some(node);
            }
            node.children.iter().find_map(|c| find(c, zkite_id))
        }
        find(&self.current, zkite_id)
            .and_then(|n| n.attrs.get("value"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string()
    }

    /// `zkite_id`'s `input_type` attr in the *current* tree (../PLAN.md §5/§7.5) —
    /// `"password"`/`"email"`/... or plain `"text"`, exactly what `TextField.
    /// _tree_attrs` set (`phase_1/zkite_py/src/zkite/widgets.py`). Fed to
    /// `call_focus_method` on every focus so `ZKiteSurfaceView` can build the right
    /// `EditorInfo`. Same recursive-search shape as `current_text_field_value` —
    /// falls back to `"text"` (not empty string) so a lookup miss degrades to the
    /// same plain-text behavior every field had before this mapping existed, rather
    /// than an `androidInputTypeFor` call on the Java side having to also handle
    /// an empty spec.
    fn current_text_field_input_type(&self, zkite_id: &str) -> String {
        fn find<'a>(node: &'a RawNode, zkite_id: &str) -> Option<&'a RawNode> {
            if node.id.as_deref() == Some(zkite_id) {
                return Some(node);
            }
            node.children.iter().find_map(|c| find(c, zkite_id))
        }
        find(&self.current, zkite_id)
            .and_then(|n| n.attrs.get("input_type"))
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .unwrap_or("text")
            .to_string()
    }

    /// `self.focused_text`, masked via `zkite_pybridge::mask_password` if the
    /// focused field is `secure=True` — what every glyph-geometry call in this
    /// file (`hit_test_text_offset`/`text_caret_geometry`, via `caret_rect`/
    /// `position_cursor_from_tap`/`dispatch_long_press`) must shape, not the real
    /// text, since `zkite_pybridge::build_node` paints the masked run of `*`s for
    /// such a field (../PLAN.md §5/§7.5) — shaping the real characters instead
    /// would compute caret/tap positions against glyph widths that were never
    /// actually on screen. **Not** for `GameTextInput`/`zkite_py`/the selection
    /// overlay's own `EditText` — those still carry the real text; see `show_
    /// selection_overlay`'s own doc comment for why the overlay masks a different
    /// way (a real `TYPE_TEXT_VARIATION_PASSWORD`, not this transform).
    fn focused_display_text(&self) -> String {
        let is_password = self
            .focused
            .as_deref()
            .is_some_and(|zkite_id| self.current_text_field_input_type(zkite_id) == "password");
        if is_password {
            zkite_pybridge::mask_password(&self.focused_text)
        } else {
            self.focused_text.clone()
        }
    }

    /// Calls `ZKiteSurfaceView.focusAndShowKeyboard(String)` (../PLAN.md §5/§7.5)
    /// through this thread's own attached env (see `attach_current_thread`'s doc
    /// comment on why it has to be this specific env). A no-op, logged once, if
    /// either `render_env` or `surface_view` isn't ready yet — both arrive
    /// asynchronously relative to boot (`SurfaceViewReady`/`TextInput::new`), and a
    /// tap landing on a `TextField` before either has is a real (if narrow) race,
    /// not something to crash over. `input_type` needs a real Java `String` (same
    /// `NewStringUTF` dance as `call_set_selection_method`), unlike `call_blur_
    /// method`'s zero-arg call, so the two no longer share one JNI helper the way
    /// they did before per-field input types existed.
    fn call_focus_method(&mut self, input_type: &str) {
        let Some(env) = self.render_env else {
            crate::loge!("call_focus_method: no attached JNIEnv (see boot log)");
            return;
        };
        let Some(view) = self.surface_view else {
            crate::loge!("call_focus_method: ZKiteSurfaceView not ready yet");
            return;
        };
        if self.surface_view_methods.is_none() {
            self.surface_view_methods = resolve_surface_view_methods(env, view.0);
        }
        let Some((show_mid, _)) = self.surface_view_methods else {
            crate::loge!("call_focus_method: method lookup failed (see log above)");
            return;
        };
        unsafe {
            let vtable = **env;
            let Some(new_string_utf) = vtable.NewStringUTF else {
                crate::loge!("call_focus_method: no NewStringUTF in vtable");
                return;
            };
            let Ok(c_input_type) = std::ffi::CString::new(input_type) else {
                crate::loge!("call_focus_method: input_type has an embedded NUL");
                return;
            };
            let jinput_type = new_string_utf(env, c_input_type.as_ptr());
            if jinput_type.is_null() {
                crate::loge!("call_focus_method: NewStringUTF failed");
                if let Some(clear) = vtable.ExceptionClear {
                    clear(env);
                }
                return;
            }
            if let Some(call) = vtable.CallVoidMethodA {
                let args = [jni_sys::jvalue { l: jinput_type }];
                call(env, view.0, show_mid, args.as_ptr());
            } else {
                crate::loge!("call_focus_method: no CallVoidMethodA in vtable");
            }
            if let Some(del) = vtable.DeleteLocalRef {
                del(env, jinput_type);
            }
            if let Some(check) = vtable.ExceptionCheck {
                if check(env) != 0 {
                    crate::loge!("call_focus_method: pending exception after call");
                    if let Some(describe) = vtable.ExceptionDescribe {
                        describe(env);
                    }
                    if let Some(clear) = vtable.ExceptionClear {
                        clear(env);
                    }
                }
            }
        }
    }

    /// Calls `ZKiteSurfaceView.blurAndHideKeyboard()` (`()V`) through this thread's
    /// own attached env — same posture as `call_focus_method` on every failure
    /// mode, just no string argument to marshal.
    /// Exit the app, from this thread, after a Back press nothing wanted.
    ///
    /// The Java side posts itself to the UI thread before touching the Activity, the
    /// same hop `focusAndShowKeyboard` makes and for the same reason (§7.3 item 2:
    /// `View`/`Activity` methods assert the UI thread, and a JNI call from here is
    /// still a background-thread call as far as they are concerned).
    fn call_finish_activity(&mut self) {
        let Some(env) = self.render_env else {
            crate::loge!("call_finish_activity: no attached JNIEnv (see boot log)");
            return;
        };
        let Some(view) = self.surface_view else {
            crate::loge!("call_finish_activity: ZKiteSurfaceView not ready yet");
            return;
        };
        if self.finish_activity_method.is_none() {
            self.finish_activity_method = resolve_finish_activity_method(env, view.0);
        }
        let Some(mid) = self.finish_activity_method else {
            crate::loge!("call_finish_activity: method lookup failed (see log above)");
            return;
        };
        unsafe {
            let vtable = **env;
            if let Some(call) = vtable.CallVoidMethodA {
                call(env, view.0, mid, std::ptr::null());
            } else {
                crate::loge!("call_finish_activity: no CallVoidMethodA in vtable");
            }
            if let Some(check) = vtable.ExceptionCheck {
                if check(env) != 0 {
                    crate::loge!("call_finish_activity: pending exception after call");
                    if let Some(describe) = vtable.ExceptionDescribe {
                        describe(env);
                    }
                    if let Some(clear) = vtable.ExceptionClear {
                        clear(env);
                    }
                }
            }
        }
    }

    fn call_blur_method(&mut self) {
        let Some(env) = self.render_env else {
            crate::loge!("call_blur_method: no attached JNIEnv (see boot log)");
            return;
        };
        let Some(view) = self.surface_view else {
            crate::loge!("call_blur_method: ZKiteSurfaceView not ready yet");
            return;
        };
        if self.surface_view_methods.is_none() {
            self.surface_view_methods = resolve_surface_view_methods(env, view.0);
        }
        let Some((_, hide_mid)) = self.surface_view_methods else {
            crate::loge!("call_blur_method: method lookup failed (see log above)");
            return;
        };
        unsafe {
            let vtable = **env;
            if let Some(call) = vtable.CallVoidMethodA {
                call(env, view.0, hide_mid, std::ptr::null());
            } else {
                crate::loge!("call_blur_method: no CallVoidMethodA in vtable");
            }
            if let Some(check) = vtable.ExceptionCheck {
                if check(env) != 0 {
                    crate::loge!("call_blur_method: pending exception after call");
                    if let Some(describe) = vtable.ExceptionDescribe {
                        describe(env);
                    }
                    if let Some(clear) = vtable.ExceptionClear {
                        clear(env);
                    }
                }
            }
        }
    }

    /// `hit_test` finds the deepest node under a tap — typically some text label or
    /// icon several levels inside whatever's actually interactive — so walk back up
    /// looking for the nearest ancestor with a `tap` handler registered. Same
    /// target-vs-currentTarget reasoning as DOM event bubbling; see
    /// `Renderer::parent`'s doc comment. Generalized from this file's original
    /// single-hardcoded-button version: any real app's arbitrarily nested interactive
    /// nodes go through the exact same walk now.
    fn dispatch_tap(&mut self, x: f32, y: f32) {
        let Some(built) = &self.built else { return };
        let offsets = self.node_scroll_offsets();
        let hit = self.renderer.hit_test(built.root, x, y, &offsets);

        // A `Switch` under the tap toggles (../PLAN.md §26). Resolved *before* the
        // tap walk below and deliberately **without stopping it**: in the DOM a click
        // on a checkbox fires the checkbox's `change` *and* bubbles to any ancestor
        // click handler, and `zkite-runtime.js` reproduces that exactly —
        // `findBoundAncestor(target, "tap")` walks straight past the input to the
        // nearest `data-zkite-on="tap"` ancestor. So a `Switch` inside a tappable row
        // fires both on the WebView backend, and native firing only the row was the
        // whole of the visible bug: the row's handler ran, the switch's did not, and
        // it read as the wrong thing responding.
        //
        // `Slider` is absent here on purpose — it captured the gesture at touch-down
        // and already committed its value, so this is never reached for one.
        // The payload is resolved here rather than at the dispatch site below, because
        // it is the *only* thing that differs between the three checkbox-shaped
        // controls: a `Switch` or `Checkbox` toggles, a `Radio` always sets `true`
        // (../PLAN.md §30 — a real `<input type=radio>` cannot be unchecked by
        // clicking it, and a group with nothing selected is usually not a state its
        // own store field can represent).
        let switch_target = hit.and_then(|n| built.control_at(&self.renderer, n)).and_then(|(zkite_id, control, _)| {
            match control {
                zkite_pybridge::Control::Switch { checked } => Some((zkite_id, checked, !checked)),
                zkite_pybridge::Control::Radio { checked } => Some((zkite_id, checked, true)),
                zkite_pybridge::Control::Slider { .. } => None,
            }
        });

        let mut node = hit;
        // Single walk up from the hit point, same target-vs-currentTarget reasoning
        // as `Renderer::parent`'s own doc comment — the first interactive ancestor
        // found (`TextField` *or* a `tap` handler, whichever is deeper) wins;
        // ../PLAN.md §5 doesn't have a `zkite_py` widget that's both at once, so there
        // is no real ordering question here yet, just "stop at the nearest match."
        let mut target: Option<String> = None;
        let mut text_field: Option<String> = None;
        while let Some(n) = node {
            if let Some(zkite_id) = built.zkite_id(n) {
                if built.is_text_field(zkite_id) {
                    text_field = Some(zkite_id.to_string());
                    break;
                }
                if built.handles(zkite_id, "tap") {
                    target = Some(zkite_id.to_string());
                    break;
                }
            }
            node = self.renderer.parent(n);
        }
        if let Some(field_id) = text_field {
            // Checked *before* `focus_text_field` (which is a no-op on a field
            // that's already focused, but doesn't touch `last_tap` either way) —
            // ../PLAN.md §5/§7.5, double-tap-to-select-word.
            let is_double_tap = self.is_double_tap(&field_id, x, y);
            self.focus_text_field(&field_id);
            if is_double_tap {
                crate::logi!("Touch ({x}, {y}) -> double-tap, selecting word on {field_id}");
                self.select_word_at(x, y);
                // Consumed — a third rapid tap starts a fresh pair rather than
                // this double-tap chaining into another one.
                self.last_tap = None;
            } else {
                crate::logi!("Touch ({x}, {y}) -> focusing text field {field_id}");
                self.position_cursor_from_tap(x, y);
                self.last_tap = Some((Instant::now(), Arc::from(field_id.as_str()), x, y));
            }
            return;
        }
        // Any tap that isn't on a `TextField` blurs whatever *was* focused — the
        // WebView backend gets this for free from Blink's own focus model; there's
        // no DOM to inherit it from here, so it's explicit. `blur_text_field` itself
        // already no-ops when nothing is focused. Also breaks any pending
        // double-tap pairing (../PLAN.md §5/§7.5) — a tap elsewhere in between
        // means the next tap on a `TextField` starts a fresh pair, not a stale one.
        self.last_tap = None;
        self.blur_text_field();

        // **An ancestor's bubbled `tap` runs before the control's own `change`, and
        // that ordering is now a decided contract rather than an accident** —
        // ../PLAN.md §15.1 divergence 4b, §27.6, §32.3. The two backends fired these
        // in opposite orders: `zkite-runtime.js` listens in the *capture* phase, so
        // Blink hands it the row before the control, while this function used to run
        // the control first. Both always fired; the order only mattered when both
        // handlers wrote the same `@store` field, where the last writer wins and was a
        // different one per backend. §30.8 measured that costing a real user — the
        // most natural way to make a checkbox label tappable netted `False` here and
        // `True` there from identical code. The owner's call is that **native matches
        // the WebView**, so the WebView backend is unchanged and this is the side that
        // moved.
        //
        // **The payload is deliberately still computed from the pre-tap build**
        // (`switch_target` was resolved above, before either dispatch), and that is
        // what makes this faithful rather than merely reordered: on the WebView the
        // `change` payload is `el.checked` read out of a DOM that Blink toggled at
        // gesture time and that no re-render has reached yet, because the shell
        // applies new HTML asynchronously. So both backends now report the value the
        // control had when the finger landed, in the same order, whatever an
        // ancestor's handler did in between. `checked` is this build's state, so the
        // new value is simply its inverse — nothing here tracks switch state of its
        // own, which is what keeps the widget controlled (see `dispatch_change_to`).
        //
        // Spelled `"true"`/`"false"` because that is what `zkite_py`'s `_as_bool`
        // parses and what `String(el.checked)` sends from `zkite-runtime.js` — one
        // wire spelling, both backends.
        if let Some(zkite_id) = target {
            crate::logi!("Touch ({x}, {y}) -> dispatching tap to {zkite_id}");
            self.dispatch_tap_to(&zkite_id);
        }

        if let Some((zkite_id, checked, next)) = switch_target {
            crate::logi!("Touch ({x}, {y}) -> toggling switch {zkite_id}: {checked} -> {next}");
            self.dispatch_change_to(&zkite_id.clone(), if next { "true" } else { "false" }, true);
        }
    }

    /// Moves the captured slider to whatever value the touch at dp `x` selects, and
    /// dispatches one `"change"` if that is a different value than last time
    /// (../PLAN.md §26).
    ///
    /// The value comes from `zkite_renderer::slider_value_at`, which is where the
    /// geometry lives precisely because this crate cannot be tested: `build.rs`
    /// compiles gametextinput's C++ against NDK headers, so `gpu_harness` does not
    /// build for the host at all (../PLAN.md §15.5). Everything arithmetic is on the
    /// other side of that call; what is left here is plumbing.
    fn update_slider_from_touch(&mut self, x: f32) {
        let Some(gesture) = &self.gesture else { return };
        let Some(slider) = &gesture.slider else { return };
        let value = zkite_renderer::slider_value_at(slider.rect, x, slider.min, slider.max, slider.step);
        if value == slider.last_value {
            return;
        }
        let target = slider.target.clone();
        // Written back before dispatching: `dispatch_change_to` re-enters CPython and
        // redraws, and a `TouchMove` arriving during that would otherwise compare
        // against a stale `last_value` and re-dispatch the same value.
        if let Some(g) = &mut self.gesture {
            if let Some(s) = &mut g.slider {
                s.last_value = value;
            }
        }
        self.dispatch_change_to(&target, &format_slider_value(value), false);
    }

    /// Dispatches a `"change"` event carrying `payload` to `zkite_id`, if it has a
    /// handler, and repaints (../PLAN.md §26).
    ///
    /// The `handles` guard is not an optimisation — a `Switch` with no `on_change` is
    /// ordinary, correct user code (a read-only indicator), and `dispatch` treats an
    /// unhandled event as normal rather than as an error. Repainting happens either
    /// way, because both `Switch` and `Slider` are **controlled**: their on-screen
    /// state is whatever the last tree said, so a control with no handler must visibly
    /// *not* move, and one whose handler declines to update the store must snap back.
    /// That is the same contract `<input type=checkbox>` has under React, and it is
    /// why nothing here keeps a local "pending" value.
    /// `measure_latency` gates the `tap-to-paint` stat (../PLAN.md §12.2 item 3):
    /// `true` for a `Switch`, whose tap is exactly the discrete tap-to-visible-change
    /// that measurement is about and directly comparable to a `Button`'s, and `false`
    /// for a `Slider` drag, where every one of dozens of moves would otherwise be
    /// logged as a separate "tap" and drag the recorded distribution around.
    fn dispatch_change_to(&mut self, zkite_id: &str, payload: &str, measure_latency: bool) {
        if !self.built.as_ref().is_some_and(|b| b.handles(zkite_id, "change")) {
            crate::logi!("change on {zkite_id} (no handler — controlled, so nothing moves)");
            return;
        }
        if measure_latency {
            self.tap_started = Some(Instant::now());
        }
        let log = |line: &str| crate::logi!("py: {line}");
        match zkite_pybridge::dispatch(zkite_id, "change", payload, &log) {
            Some(json) => match RawNode::from_json(&json) {
                Ok(tree) => {
                    self.current = tree;
                    self.redraw();
                }
                Err(e) => crate::loge!("dispatch({zkite_id}, change) returned malformed JSON: {e}"),
            },
            None => crate::logi!("dispatch({zkite_id}, change) unexpectedly reported unhandled"),
        }
    }

    /// Dispatches a `"tap"` event to `zkite_id` and, if handled, updates `self.current`
    /// and redraws — the actual Python round-trip shared by a real touch
    /// (`dispatch_tap`, above) and an AccessKit `Action::Click`
    /// (`handle_a11y_action`, ../PLAN.md §5 item 2): both mean the same thing to
    /// `zkite_py` ("this node's tap handler fired"), so both go through the exact same
    /// call instead of two copies that could silently drift apart.
    fn dispatch_tap_to(&mut self, zkite_id: &str) {
        // ../PLAN.md §12.2 item 3, and the span is chosen so the WebView backend can
        // report the identical one: from this backend first acting on the tap to the
        // resulting frame being presented. Deliberately *not* from the raw
        // `ACTION_UP`, because the WebView side's equivalent entry point is its own
        // delegated DOM handler — starting the clock at different depths on the two
        // sides would charge one of them for platform dispatch the other never sees.
        self.tap_started = Some(Instant::now());
        let log = |line: &str| crate::logi!("py: {line}");
        match zkite_pybridge::dispatch(zkite_id, "tap", "", &log) {
            Some(json) => match RawNode::from_json(&json) {
                Ok(tree) => {
                    self.current = tree;
                    self.redraw();
                }
                Err(e) => crate::loge!("dispatch({zkite_id}) returned malformed JSON: {e}"),
            },
            // Shouldn't happen — `zkite_id` came from the event map, so
            // `runtime.py`'s `_zkite_dispatch_tree` should have found the same
            // handler `to_tree` registered it under. Not treated as fatal: the app
            // can keep responding to other taps.
            None => crate::logi!("dispatch({zkite_id}, tap) unexpectedly reported unhandled"),
        }
    }

    /// Handles an AccessKit action request (../PLAN.md §5 item 2) — `Action::Click`
    /// (a TalkBack double-tap-to-activate, or a `performAction(ACTION_CLICK)` call
    /// from any assistive technology) reuses the exact same dispatch a real touch
    /// uses (`dispatch_tap_to`) for a node with a `tap` handler, or focuses a
    /// `TextField` the same way a real tap on it does (`accesskit_android::adapter`'s
    /// own `perform_simple_action` already promotes a focusable-but-not-clickable
    /// node's `Click` request to a plain `Focus` one before it ever reaches here —
    /// see that crate's `ACTION_CLICK` arm — so a `TextField`, which this crate marks
    /// focusable but not clickable, only ever arrives as `Action::Focus`, never
    /// `Action::Click`; both arms handle it identically regardless, since nothing
    /// here depends on which of the two actually fired). Nothing else is wired up
    /// yet — see ../PLAN.md §5 item 2's own known-gaps note (no scroll actions, no
    /// text-selection/traversal actions).
    fn handle_a11y_action(&mut self, request: ActionRequest) {
        let Some(built) = &self.built else { return };
        let Some(zkite_id) = built.zkite_id_for_a11y(request.target_node).map(|s| s.to_string()) else {
            crate::logi!("handle_a11y_action: {:?} on unknown node {} (stale build?)", request.action, request.target_node.0);
            return;
        };
        crate::logi!("handle_a11y_action: {:?} on {zkite_id}", request.action);
        match request.action {
            Action::Click if built.is_text_field(&zkite_id) => self.focus_text_field(&zkite_id),
            Action::Click if built.handles(&zkite_id, "tap") => self.dispatch_tap_to(&zkite_id),
            Action::Focus if built.is_text_field(&zkite_id) => self.focus_text_field(&zkite_id),
            _ => crate::logi!("handle_a11y_action: {:?} on {zkite_id} has no handler", request.action),
        }
    }

    /// Installs the AccessKit adapter on `ZKiteSurfaceView` (../PLAN.md §5 item 2), the
    /// first time both a render-thread JNI attachment (`render_env`) and the
    /// `ZKiteSurfaceView` global ref (`surface_view`) are available — mirrors
    /// `call_focus_method`'s own "resolve lazily, cache, no-op if unavailable"
    /// posture. Idempotent: a no-op once `a11y_adapter` is `Some` (constructing a
    /// second `InjectingAdapter` against the same still-live `ZKiteSurfaceView` would
    /// panic — see `accesskit_android::inject`'s own "host already has an
    /// accessibility delegate" check), and safe to call from the render thread
    /// specifically — see `a11y.rs`'s own module doc, finding 1, for why this
    /// *doesn't* need the UI-thread hop `text_input::TextInput::new` needs.
    fn ensure_a11y_adapter(&mut self) {
        if self.a11y_adapter.is_some() {
            return;
        }
        let (Some(raw_env), Some(surface_view)) = (self.render_env, self.surface_view) else { return };
        let env = unsafe { crate::a11y::bridge_env(raw_env) };
        let mut env = match env {
            Ok(env) => env,
            Err(e) => {
                crate::loge!("ensure_a11y_adapter: JNIEnv::from_raw failed: {e}");
                return;
            }
        };
        let host = unsafe { crate::a11y::bridge_jobject(surface_view.0) };
        self.a11y_adapter = Some(crate::a11y::install(&mut env, &host));
        crate::logi!("ensure_a11y_adapter: InjectingAdapter installed on ZKiteSurfaceView");
    }

    /// Pushes a fresh `accesskit::TreeUpdate` (../PLAN.md §5 item 2) built from
    /// `self.built`'s own bounds-less nodes plus `nodes`' real, just-painted bounds
    /// (`zkite_pybridge::BuiltTree::accessibility_tree_update` — see that method's own
    /// doc for why bounds can only be filled in this late). Called from `redraw()`
    /// after `self.built` is set, so `self.built`'s reference here is always this
    /// frame's own build, never a stale one. A no-op (cheaply) whenever
    /// `ensure_a11y_adapter` hasn't managed to install an adapter yet.
    fn push_a11y_update(&mut self, nodes: &[zkite_renderer::PositionedNode]) {
        self.ensure_a11y_adapter();
        let Some(built) = &self.built else { return };
        let update = built.accessibility_tree_update(nodes, self.focused.as_deref(), self.scale);
        // Always kept current, even while accessibility is off — this is what
        // `a11y::Activation::request_initial_tree` hands back when the framework
        // (re)attaches later, so a screen reader started mid-session sees the real
        // tree immediately rather than a stale or placeholder one. Cheap, and
        // deliberately *not* behind the guard below.
        crate::a11y::set_latest_tree(update.clone());
        // The guard (../PLAN.md §8.7). `update_if_active` raises real
        // `AccessibilityEvent`s once AccessKit's adapter has gone `Active`, and
        // doing that while Android's accessibility framework is disabled throws
        // `IllegalStateException` from `AccessibilityManager.sendAccessibilityEvent`
        // — which `accesskit_android` `.unwrap()`s, aborting the whole process. The
        // adapter has no way to know accessibility was switched off underneath it
        // (nothing in its API observes that), so the check has to happen here, on
        // ZKite's side, before every push. See `ZKiteSurfaceView.
        // isAccessibilityEnabled`'s own doc comment for why this fires far more
        // often than "the user runs a screen reader" would suggest.
        if !self.accessibility_enabled() {
            return;
        }
        if let Some(adapter) = &mut self.a11y_adapter {
            adapter.update_if_active(|| update);
        }
    }

    /// Whether Android's accessibility framework is enabled *right now*, via
    /// `ZKiteSurfaceView.isAccessibilityEnabled()` (../PLAN.md §8.7). Queried fresh
    /// on every push rather than cached: accessibility can be toggled at any moment
    /// by a screen reader starting/stopping or by a transient client like
    /// `uiautomator dump`, and a stale `true` here is precisely the process-abort
    /// this guard exists to prevent. One `CallBooleanMethod` per redraw — the same
    /// order of cost as the JNI calls `redraw` already makes.
    ///
    /// Fails **closed** (returns `false`) if the env, the view, or the method lookup
    /// isn't available: skipping an accessibility update degrades assistive-tech
    /// support for that frame, while wrongly proceeding crashes the app outright —
    /// an easy call between the two.
    fn accessibility_enabled(&mut self) -> bool {
        let Some(env) = self.render_env else { return false };
        let Some(view) = self.surface_view else { return false };
        if self.is_accessibility_enabled_method.is_none() {
            self.is_accessibility_enabled_method = resolve_is_accessibility_enabled_method(env, view.0);
        }
        let Some(mid) = self.is_accessibility_enabled_method else { return false };
        unsafe {
            let vtable = **env;
            let Some(call) = vtable.CallBooleanMethodA else { return false };
            let enabled = call(env, view.0, mid, std::ptr::null());
            if let Some(check) = vtable.ExceptionCheck {
                if check(env) != 0 {
                    crate::loge!("accessibility_enabled: pending exception after call");
                    if let Some(describe) = vtable.ExceptionDescribe {
                        describe(env);
                    }
                    if let Some(clear) = vtable.ExceptionClear {
                        clear(env);
                    }
                    return false;
                }
            }
            enabled != 0
        }
    }

    /// Turns a just-ended drag into a fling, if its release velocity clears
    /// `MIN_FLING_START_VELOCITY` — called from `TouchUp` for any gesture that
    /// dragged, whether or not it ever actually moved a scroll offset (a drag with
    /// no `scroll_target` bails out below, same as it silently no-ops in
    /// `TouchMove`). Velocity is estimated from `gesture.history`'s oldest and
    /// newest surviving sample (already trimmed to `VELOCITY_WINDOW` by every
    /// `TouchMove`) rather than just the last two points, so one slow final sample
    /// right at lift-off doesn't read as "the drag stopped" when the finger was
    /// actually still moving fast up to that instant.
    fn maybe_start_fling(&mut self, gesture: Gesture) {
        // A fling belongs to the axis its drag locked to — it is that drag's tail, not
        // a fresh decision. `axis` is always `Some` here (`TouchUp` only calls this for
        // a gesture that dragged, and `dragging` and `axis` are set together), but
        // defaulting to Y rather than unwrapping keeps a future reordering from
        // panicking on the render thread.
        let axis = gesture.axis.unwrap_or(zkite_renderer::ScrollAxis::Y);
        let target = match axis {
            zkite_renderer::ScrollAxis::X => gesture.scroll_x,
            zkite_renderer::ScrollAxis::Y => gesture.scroll_y,
        };
        let Some(target) = target else { return };
        let (Some(&(first_t, first_x, first_y)), Some(&(last_t, last_x, last_y))) =
            (gesture.history.first(), gesture.history.last())
        else {
            return;
        };
        let dt = last_t.duration_since(first_t).as_secs_f32();
        if dt < MIN_VELOCITY_SAMPLE_DT {
            return;
        }
        // Same sign convention as `TouchMove`'s own per-move `delta`: positive means
        // the offset is increasing. Summing that convention over the whole window
        // telescopes to exactly `first - last` on the locked axis.
        let velocity = match axis {
            zkite_renderer::ScrollAxis::X => (first_x - last_x) / dt,
            zkite_renderer::ScrollAxis::Y => (first_y - last_y) / dt,
        };
        if velocity.abs() < MIN_FLING_START_VELOCITY {
            return;
        }
        let tau = fling_tau(velocity);
        crate::logi!(
            "fling: launching at {velocity:.0}dp/s on {axis:?} (tau {tau:.3}s, travel {:.0}dp, max_scroll {})",
            velocity.abs() * tau,
            target.max
        );
        self.fling = Some(Fling {
            target: target.id,
            axis,
            velocity,
            max_scroll: target.max,
            last_tick: Instant::now(),
            tau,
        });
    }

    /// Advances `self.fling` by one animation step and redraws — called from `run()`
    /// once per `FRAME_INTERVAL` while a fling is in flight. Uses the exact integral
    /// of exponential decay over the real elapsed `dt` (not a fixed per-frame
    /// increment), so the animation's total travel and duration are correct even if
    /// a tick is late (e.g. a `Resize` briefly held up the render thread) rather than
    /// silently losing or gaining distance to however long that tick actually took.
    fn tick_fling(&mut self) {
        let Some(fling) = self.fling.as_mut() else { return };
        let now = Instant::now();
        let dt = now.duration_since(fling.last_tick).as_secs_f32();
        fling.last_tick = now;
        if dt <= 0.0 {
            // Two ticks with no measurable time between them (clock granularity) —
            // nothing to integrate yet; wait for the next one rather than dividing
            // by/decaying over a zero interval.
            return;
        }
        let decay = (-dt / fling.tau).exp();
        let displacement = fling.velocity * fling.tau * (1.0 - decay);
        fling.velocity *= decay;
        let target = fling.target.clone();
        let axis = fling.axis;
        let max_scroll = fling.max_scroll;
        let remaining_velocity = fling.velocity;

        let entry = self.scroll_offsets.entry(target).or_insert((0.0, 0.0));
        let slot = match axis {
            zkite_renderer::ScrollAxis::X => &mut entry.0,
            zkite_renderer::ScrollAxis::Y => &mut entry.1,
        };
        let unclamped = *slot + displacement;
        let clamped = unclamped.clamp(0.0, max_scroll);
        // Reached a scroll bound: stop dead rather than bouncing — this renderer has
        // no overscroll/rubber-band effect yet (see ../PLAN.md §5), so continuing to
        // tick with the offset pinned at a bound would just be wasted wakeups with
        // nothing left to animate.
        let hit_bound = clamped != unclamped;
        *slot = clamped;

        if hit_bound || remaining_velocity.abs() < MIN_FLING_STOP_VELOCITY {
            self.fling = None;
        }
        self.redraw();
    }

    /// Rebuilds `zkite_renderer`'s tree from `self.current` and paints it. Called
    /// after every state change (`current` updated by `dispatch`) *and* every size
    /// change (`current` unchanged, only `width`/`height` are new) — the tree is
    /// cheap to rebuild and correctness (a stale layout at the old size) matters
    /// more than skipping the rebuild would save.
    ///
    /// This used to be able to say "never re-invokes Python," and that was the whole
    /// reason scrolling was cheap. Virtualised lists (../PLAN.md §11) ended that: a
    /// `List` cannot know which rows to build without knowing where it is scrolled
    /// to, so scroll position has to reach `zkite_py` before the tree is built. What
    /// survives of the old property is the part that mattered — Python is *asked*
    /// once per scroll frame but only *re-runs `view()`* when a materialised window
    /// actually moved, so the common case is still a rebuild-and-paint with no user
    /// code in it. See `sync_viewports`.
    fn sync_system_bars(&mut self) {
        let style = self.current.attrs.get("system_bars")
            .map(serde_json::Value::to_string).unwrap_or_else(|| "{}".into());
        if self.last_system_bars.as_ref() == Some(&style) { return; }
        let (Some(raw_env), Some(view)) = (self.render_env, self.surface_view) else { return };
        // Same attached env/global-reference bridge used by the accessibility adapter.
        let Ok(mut env) = (unsafe { crate::a11y::bridge_env(raw_env) }) else { return };
        let host = unsafe { crate::a11y::bridge_jobject(view.0) };
        let result = env.with_local_frame(4, |env| -> Result<(), accesskit_android::jni::errors::Error> {
            let json = env.new_string(&style)?;
            env.call_method(&host, "applySystemBars", "(Ljava/lang/String;)V", &[(&json).into()])?;
            Ok(())
        });
        match result {
            Ok(()) => self.last_system_bars = Some(style),
            Err(error) => {
                crate::loge!("system bars: {error}");
                let _ = env.exception_clear();
            }
        }
    }

    fn redraw(&mut self) {
        self.sync_system_bars();
        if self.window.is_none() || self.width <= 0 || self.height <= 0
            || !crate::SURFACE_LIFECYCLE.is_current(self.window_generation) {
            return;
        }

        // Lay out, then tell `zkite_py` where every scrollable container ended up, so
        // a virtualised `List` can decide which rows it now needs (../PLAN.md §11).
        // If that changed the tree, lay out again — the second pass is what actually
        // gets painted, so a window change is never visible as a frame of missing
        // rows. Bounded rather than looped to a fixpoint: a `List`'s content height
        // is window-invariant by construction (the spacers see to that), so its own
        // viewport height can't change as a result of its window changing, and a
        // second pass that still wants a third is a bug worth reporting rather than
        // spinning on.
        let started = Instant::now();
        // Which half of the window this frame belongs to (../PLAN.md §42) — see
        // `BudgetStats::fling_frames`. Read here rather than in `note_redraw_cost`
        // because `tick_fling` can clear `self.fling` on the very tick it paints.
        if self.fling.is_some() {
            self.budget_stats.fling_frames += 1;
        }
        // Recorded at the *start* of the frame, not the end — `next_frame_wait`
        // subtracts this frame's own work from the next frame's wait, which only
        // works if the mark is the frame boundary rather than the moment the work
        // finished. Set for every redraw, not just fling ticks, so a fling that
        // begins right after a touch-driven frame doesn't immediately render a
        // second one.
        self.last_frame_at = Some(started);

        // ../PLAN.md §47. Sampled once, at the top, against **one** timestamp for the
        // whole frame — `layout_frame` can run twice (the viewport sync pass) and two
        // different `now`s across one frame would sample the same animation at two
        // different points and paint the second, which is a subtle way to make an
        // animation's duration depend on whether the list happened to rebuild.
        //
        // Completions are collected, not dispatched: a Python handler may
        // `@store.update()`, which re-enters this thread's frame path. The loop drains
        // them after the frame is on screen.
        let advance = self.animator.timeline.advance(started);
        for key in advance.finished {
            let trigger = self.animator.started.get(&key).map(|(_, t)| *t);
            // A key with no `started` entry finished on a frame where its declaration
            // had already been withdrawn; there is nobody left to tell.
            if let Some(trigger) = trigger {
                self.animator.finished.push((key.0, key.1, trigger));
            }
        }

        let mut frame = self.layout_frame(started);
        let mut phases = frame.phases;
        let mut rebuilt = false;
        for pass in 0..MAX_VIEWPORT_SYNC_PASSES {
            let t = Instant::now();
            let changed = self.sync_viewports(&frame, &mut phases);
            phases.sync += t.elapsed();
            // Counted on the first pass only: this is "did this *frame* rebuild",
            // not "how many passes did it take", and a second pass is already
            // reported separately as the `viewport sync did not settle` line.
            if pass == 0 {
                self.scroll_stats.0 += 1;
                if changed {
                    self.scroll_stats.1 += 1;
                    rebuilt = true;
                }
            }
            if !changed {
                break;
            }
            frame = self.layout_frame(started);
            phases.add(&frame.phases);
            // The same three phases the first pass already paid, paid again because
            // the window moved (§43). `add` has folded them into the running
            // `build`/`layout`/`positioned` totals; this records how much of those
            // totals only exists on a rebuild frame.
            phases.relayout += frame.phases.build + frame.phases.layout + frame.phases.positioned;
            if pass + 1 == MAX_VIEWPORT_SYNC_PASSES {
                crate::logi!("viewport sync did not settle in {MAX_VIEWPORT_SYNC_PASSES} passes");
            }
        }
        let LayoutFrame { built, nodes, .. } = frame;

        // Caret sits at the selection's *end* — matches every real text editor's
        // convention for a plain (non-selecting) cursor, where start == end anyway;
        // see ../PLAN.md §7.7's own known-gaps note on why a real *range* selection
        // still renders as a single caret here rather than a highlighted span.
        // `focused_display_text` (not `self.focused_text` directly), so the caret
        // shapes against whatever `zkite_pybridge::build_node` actually painted —
        // the masked run of `*`s for a `secure=True` field (../PLAN.md §5/§7.5),
        // the real text otherwise.
        let display_text = self.focused_display_text();
        let caret = built.focused_text_node.and_then(|node| {
            let offset = self.focused_selection.1.max(0) as usize;
            caret_rect(&mut self.renderer, &nodes, node, &display_text, offset)
        });
        self.built = Some(built);
        let t = Instant::now();
        self.push_a11y_update(&nodes);
        phases.a11y = t.elapsed();

        if !crate::SURFACE_LIFECYCLE.is_current(self.window_generation) {
            return;
        }
        let mut outcome = if let Some(gpu) = &mut self.gpu {
            paint_gpu(gpu, self.width, self.height, self.scale, &nodes, caret, &mut phases, &mut self.images)
        } else {
            PaintOutcome::Fallback // no GPU backend at all
        };

        // A lost surface is rebuilt here rather than in `paint_gpu`, because recreating
        // one needs the `NativeWindow` and only `State` holds it.
        if outcome == PaintOutcome::Recreate {
            if !crate::SURFACE_LIFECYCLE.is_current(self.window_generation) {
                return;
            }
            if let (Some(gpu), Some(window)) = (self.gpu.as_mut(), self.window.as_ref()) {
                ensure_surface(gpu, window);
                reconfigure(gpu, self.width, self.height, self.images.as_deref_mut());
                // Immediately retry painting on the fresh surface so the frame is not dropped,
                // preventing the screen from staying black if no subsequent event triggers a redraw.
                outcome = paint_gpu(gpu, self.width, self.height, self.scale, &nodes, caret, &mut phases, &mut self.images);
            }
        }

        // Logged only when it *changes*, never per frame (../PLAN.md §7.14). Both
        // halves of this were invisible during the long §7.6 investigation and both
        // turned out to matter: a surface whose size silently changed and never
        // changed back was the actual root cause, and a silent GPU→CPU fall-through
        // would have been mistaken for one. Change-only keeps that signal without
        // putting a log line on the hot path.
        let path = match outcome {
            PaintOutcome::Painted => "gpu",
            PaintOutcome::Skip => "gpu (frame skipped)",
            PaintOutcome::Recreate => "gpu (surface recreated)",
            PaintOutcome::Fallback => "cpu-fallback",
        };
        if self.last_logged_frame_state != Some((self.width, self.height, path)) {
            self.last_logged_frame_state = Some((self.width, self.height, path));
            crate::logi!("frame: {}x{} via {path}", self.width, self.height);
        }

        // **Only when there is genuinely no GPU path.** Before §39 this ran for any
        // non-`Some` return, which is how one transient swapchain error turned into a
        // permanently CPU-rendered app. There is a second reason to keep the CPU
        // rasteriser away from a live wgpu surface, and it is the likelier explanation
        // for why the old behaviour was *self-sustaining* rather than merely slow:
        // `paint_cpu_fallback` locks the `ANativeWindow` and writes pixels into it
        // directly, so for that frame the CPU is the buffer producer for a window whose
        // swapchain wgpu still owns. Named as the hypothesis it is — §39 did not isolate
        // it, and the recovery above is what fixes the symptom either way.
        let mut presented = outcome == PaintOutcome::Painted;
        if outcome == PaintOutcome::Fallback {
            if let Some(window) = &self.window {
                let t = Instant::now();
                presented = paint_cpu_fallback(window, self.width, self.height, self.scale, &nodes, caret, &mut self.images);
                phases.scene += t.elapsed();
            }
        }

        // The cold-start half of ../PLAN.md §12.2 item 1. Emitted from here rather
        // than from the `frame:` line above because that one is change-gated and
        // would go quiet if a frame ever painted before this point; this has to fire
        // exactly once, on the first frame that actually reached the screen.
        // `ZKiteActivity`'s own `boot-call process_rel=` line carries the other half.
        self.retry_redraw = !presented;
        if presented && !self.first_frame_logged {
            self.first_frame_logged = true;
            if let (Some(raw_env), Some(view)) = (self.render_env, self.surface_view) {
                if let Ok(mut env) = unsafe { crate::a11y::bridge_env(raw_env) } {
                    let host = unsafe { crate::a11y::bridge_jobject(view.0) };
                    if let Err(error) = env.call_method(&host, "onFirstFrame", "()V", &[]) {
                        crate::loge!("first frame callback: {error}");
                        let _ = env.exception_clear();
                    }
                }
            }
            crate::logi!(
                "ZKITEPERF first-frame since_boot_call={:.0}ms nodes={} via={path}",
                self.thread_started.elapsed().as_secs_f32() * 1000.0,
                nodes.len()
            );
        }

        // ../PLAN.md §12.2 item 3. Taken here, after `present`, because the question
        // is when the user *sees* the change, not when this thread stopped working on
        // it. Note the honest limit, which applies equally to the WebView backend's
        // matching line: `present` returning is this backend handing the frame to the
        // compositor, not the panel lighting up — both numbers exclude the same final
        // hop, so they compare fairly even though neither is the whole latency.
        if let Some(tapped) = self.tap_started.take().filter(|_| presented) {
            crate::logi!("ZKITEPERF tap-to-paint={:.1}ms", tapped.elapsed().as_secs_f32() * 1000.0);
        }

        self.note_redraw_cost(started.elapsed(), nodes.len(), &phases, rebuilt);
    }

    /// How long `run()` sat blocked on `rx` before the command that drove this
    /// frame. See `BudgetStats::idle`.
    ///
    /// Called from the loop rather than measured here because the wait happens
    /// *outside* `redraw()` by construction — it is the time this thread had
    /// available and did not use, which is exactly what no existing phase measures.
    ///
    /// `paced` is whether a fling was in flight when the wait began, in which case
    /// the wait was `next_frame_wait`'s deliberate one and is not slack.
    fn note_idle(&mut self, waited: Duration, paced: bool) {
        if !paced && waited < IDLE_GAP {
            self.budget_stats.idle += waited;
        }
    }

    /// One batch of `TouchMove`s pulled off `rx`: `dropped` is how many `run()`'s
    /// coalescing loop discarded in favour of the newest. See
    /// `BudgetStats::moves_dropped`.
    fn note_touch_batch(&mut self, dropped: u32) {
        self.budget_stats.moves_in += dropped + 1;
        self.budget_stats.moves_dropped += dropped;
    }

    /// Accumulates redraw cost and logs a summary every `REDRAW_STATS_INTERVAL`
    /// frames (../PLAN.md §11).
    ///
    /// Permanent rather than a temporary measurement hack, for the reason §7.14
    /// records at length: the diagnostics that would have found this phase's worst
    /// bug cheaply didn't exist until someone went looking, and "how long does a
    /// frame actually take on the real device" is precisely the question a
    /// virtualised list exists to answer. One `Instant::now()` and an integer add
    /// per frame, one log line per 60 — cheap enough to leave on, which is the whole
    /// point of leaving it on.
    fn note_redraw_cost(&mut self, elapsed: Duration, node_count: usize, phases: &FramePhases, rebuilt: bool) {
        if elapsed >= SLOW_REDRAW_THRESHOLD {
            crate::logi!("redraw SLOW: {:.1}ms ({node_count} nodes)", elapsed.as_secs_f32() * 1000.0);
        }
        // Frame *interval*, distinct from the frame cost above — see
        // `last_frame_end`'s doc comment. Gated on a recent previous frame so the
        // gap across an idle app (which can be minutes) is never counted as a frame
        // interval; `IDLE_GAP` is well past any real frame and well under any real
        // pause.
        let now = Instant::now();
        if let Some(prev) = self.last_frame_end {
            let interval = now.duration_since(prev);
            if interval < IDLE_GAP {
                let (ref mut n, ref mut total, ref mut worst) = self.interval_stats;
                *n += 1;
                *total += interval;
                *worst = (*worst).max(interval);
                // …and the same interval, bucketed rather than averaged
                // (../PLAN.md §42). `frame_periods` is how many refreshes this
                // frame actually occupied: 1 is on time, 2 means exactly one frame
                // was dropped. It takes the panel's own period (§13.2) rather than
                // a hardcoded 16ms, so it stays right on a 90/120Hz device — see
                // `zkite_frametiming` for why the rounding is what it is, and for
                // the tests that pin it.
                let b = &mut self.budget_stats;
                match frame_periods(interval, self.frame_period) {
                    1 => b.on_time += 1,
                    n => {
                        if n == 2 {
                            b.late_1 += 1;
                        } else {
                            b.late_2plus += 1;
                        }
                        if rebuilt {
                            b.late_rebuilt += 1;
                        }
                    }
                }
            }
        }
        self.last_frame_end = Some(now);
        self.phase_stats.add(phases);
        self.phase_worst.take_max(phases);
        let (ref mut count, ref mut total, ref mut worst) = self.redraw_stats;
        *count += 1;
        *total += elapsed;
        *worst = (*worst).max(elapsed);
        if *count < REDRAW_STATS_INTERVAL {
            return;
        }
        let avg = *total / *count as u32;
        crate::logi!(
            "redraw x{}: avg {:.2}ms max {:.2}ms ({} nodes)",
            *count,
            avg.as_secs_f32() * 1000.0,
            worst.as_secs_f32() * 1000.0,
            node_count
        );
        let n = *count;
        self.redraw_stats = (0, Duration::ZERO, Duration::ZERO);

        // The one number directly comparable to the WebView backend's own
        // `scroll-frames` line (../PLAN.md §12.2 item 2) — hence the shared
        // `ZKITEPERF` prefix, so one logcat grep reads both backends.
        let (ivn, ivtotal, ivworst) = self.interval_stats;
        if ivn > 0 {
            crate::logi!(
                "ZKITEPERF frame-interval n={ivn} avg {:.2}ms max {:.2}ms ({node_count} nodes)",
                ivtotal.as_secs_f32() * 1000.0 / ivn as f32,
                ivworst.as_secs_f32() * 1000.0
            );
        }
        self.interval_stats = (0, Duration::ZERO, Duration::ZERO);

        // Per-phase, because "the frame is 23ms" on its own points nowhere. Averaged
        // over the same window as the total above, so the five numbers add up to it
        // (minus the small unattributed remainder — caret geometry, the viewport-sync
        // Python call, the `Instant`s themselves).
        let p = self.phase_stats;
        let ms = |d: Duration| d.as_secs_f32() * 1000.0 / n as f32;
        crate::logi!(
            "  phases: build {:.2}  layout {:.2}  positioned {:.2}  sync {:.2}  a11y {:.2}  scene {:.2}  acquire {:.2}  present {:.2}  (ms)",
            ms(p.build),
            ms(p.layout),
            ms(p.positioned),
            ms(p.sync),
            ms(p.a11y),
            ms(p.scene),
            ms(p.acquire),
            ms(p.present)
        );
        // The worst single frame in the window, phase by phase. This is the line that
        // names a hitch's cause; the averages above only describe the steady state.
        let w = self.phase_worst;
        let mx = |d: Duration| d.as_secs_f32() * 1000.0;
        crate::logi!(
            "  worst:  build {:.2}  layout {:.2}  positioned {:.2}  sync {:.2}  a11y {:.2}  scene {:.2}  acquire {:.2}  present {:.2}  (ms)",
            mx(w.build),
            mx(w.layout),
            mx(w.positioned),
            mx(w.sync),
            mx(w.a11y),
            mx(w.scene),
            mx(w.acquire),
            mx(w.present)
        );
        // `present`, split into the three unrelated calls it has always been (§46).
        // Its own line rather than three more columns on `phases:`/`worst:`, so those
        // two stay byte-comparable with the four sections that quote them (§39, §42,
        // §43, §44) — the same call §43 made when it split `sync` and kept the total.
        //
        // `render` is the only one of the three that occlusion culling could shrink,
        // so this line is what says whether Queue H's last item has anything to remove.
        // `swap` is read against `acquire` on the line above: §39 proved the swapchain
        // does not block on *dequeue*, and enqueue is the half it never measured.
        crate::logi!(
            "  present: render {:.2}  finish {:.2}  submit {:.2}  swap {:.2}  (of {:.2} total, ms) | worst: render {:.2}  finish {:.2}  submit {:.2}  swap {:.2}",
            ms(p.present_render),
            ms(p.present_finish),
            ms(p.present_submit),
            ms(p.present_swap),
            ms(p.present),
            mx(w.present_render),
            mx(w.present_finish),
            mx(w.present_submit),
            mx(w.present_swap)
        );
        self.phase_stats = FramePhases::default();
        self.phase_worst = FramePhases::default();

        // The animation driver's own behaviour (../PLAN.md §47) — **counts, not
        // time**, deliberately. Its per-frame cost lives inside `build` above and is
        // bounded by the number of animating nodes rather than by the tree, so a
        // seventh time column would be reporting noise. What no existing line can see
        // is whether the driver is *working*: `started` and `done` must balance for
        // every non-repeating animation, and `driven` is the number of frames this
        // thread produced because an animation asked for them rather than because a
        // finger or a fling did.
        //
        // `driven` is the one that matters beyond debugging. Every measurement in this
        // document since §42.3 has been pinned near the input cadence, which is why
        // §43's 7ms win moved the frame-interval average by 0.12ms — a non-zero
        // `driven` is the first workload here that is not input-paced, and it is what
        // makes `budget:`'s late counts attributable to this renderer rather than to
        // how fast Android happened to deliver touch samples (§45's "free instrument").
        let a = &self.animator;
        if a.stats_started > 0 || a.stats_completed > 0 || a.stats_driven_frames > 0 {
            crate::logi!(
                "  anim: started {} done {} driven-frames {} running {} ({:.0}% of frames self-paced)",
                a.stats_started,
                a.stats_completed,
                a.stats_driven_frames,
                a.timeline.len(),
                a.stats_driven_frames as f32 * 100.0 / n as f32
            );
        }
        self.animator.stats_started = 0;
        self.animator.stats_completed = 0;
        self.animator.stats_driven_frames = 0;

        // The `sync` phase's cause. `rebuilt` is the number of frames in this window
        // that re-entered CPython for a whole new tree; `sync`'s average and worst
        // above are what those cost. A window with `rebuilt 0` whose `sync` is still
        // large would mean the cheap "nothing to do" path is not cheap, which is a
        // different bug from the one ../PLAN.md §39.5 filed.
        let (sframes, srebuilt) = self.scroll_stats;
        if sframes > 0 {
            crate::logi!(
                "  scroll: {srebuilt} tree rebuilds / {sframes} frames ({:.0}% of frames re-ran view())",
                srebuilt as f32 * 100.0 / sframes as f32
            );
        }

        // What one rebuild costs, split into the half a smaller rebuild could avoid
        // and the half it could not (§43). Averaged over *rebuilds*, not frames:
        // `phases:` above already divides by frames, which understates a rebuild's
        // own cost by the rebuild rate (~15x at §42's 6.7%) and is why ../PLAN.md
        // §42.5 had to reconstruct it from `worst:` across three `overscan` arms.
        //
        // `/KB` is the unit that makes this comparable to a host run of the same
        // payload: the tree that crosses the boundary is the same bytes on both, so a
        // ratio between them is a property of the interpreter and the CPU rather than
        // of the app's row count.
        if sframes > 0 {
            let per = |d: Duration, k: u32| d.as_secs_f32() * 1000.0 / k.max(1) as f32;
            let kb = self.rebuild_bytes as f32 / 1024.0;
            crate::logi!(
                "  rebuild: {srebuilt} x {:.1}KB  py {:.2}ms  parse {:.2}ms  relayout {:.2}ms  ({:.0}us/KB py, {:.0}us/KB parse)  poll {:.3}ms x{}",
                kb / srebuilt.max(1) as f32,
                per(p.sync_py, srebuilt),
                per(p.sync_parse, srebuilt),
                per(p.relayout, srebuilt),
                p.sync_py.as_secs_f32() * 1_000_000.0 / kb.max(0.001),
                p.sync_parse.as_secs_f32() * 1_000_000.0 / kb.max(0.001),
                per(p.sync_poll, sframes - srebuilt),
                sframes - srebuilt,
            );
        }
        self.scroll_stats = (0, 0);
        self.rebuild_bytes = 0;

        // Where the dropped frames are, and whether this thread had room to avoid
        // them — see `BudgetStats`. Two lines because they answer two questions:
        // the first is *how bad and whose fault*, the second is *is there any slack
        // to pace into at all*, which is what decides whether ../PLAN.md §39.5's
        // drag-pacing proposal can do anything.
        let b = self.budget_stats;
        let late = b.late_1 + b.late_2plus;
        let bucketed = b.on_time + late;
        if bucketed > 0 {
            crate::logi!(
                "  budget: {} on-time / {} late-1 / {} late-2+ ({:.0}% late, {} of {late} late frames rebuilt)",
                b.on_time,
                b.late_1,
                b.late_2plus,
                late as f32 * 100.0 / bucketed as f32,
                b.late_rebuilt
            );
            // Idle as a fraction of the window's **elapsed** time, not of its
            // nominal budget (`frames x frame_period`). The budget form was written
            // first and is wrong in the one regime that matters: on a run whose
            // frames took 235ms it reported "23% of budget" idle — reading as a
            // fifth of a frame spare — for a thread that was 98% busy, because
            // 3.81ms against a *notional* 16.67ms is a large fraction and against
            // the 235ms actually elapsed is nothing. Overstating slack is precisely
            // the misreading this line exists to prevent, since slack is what
            // decides whether pacing a drag can do anything at all. Against elapsed
            // the two agree whenever frames are near budget and diverge correctly
            // when they are not.
            //
            // Paced (fling) frames are excluded from both halves of the fraction —
            // see `BudgetStats::idle`. Their share of elapsed time is removed by
            // prorating rather than by being measured separately, which assumes the
            // two classes of frame take a similar time each; in a window that is
            // all one or all the other (`fling 0`, the case this is read in) the
            // assumption does not bind at all.
            let unpaced = bucketed.saturating_sub(b.fling_frames);
            let elapsed = ivtotal.as_secs_f32() * unpaced as f32 / bucketed.max(1) as f32;
            crate::logi!(
                "  slack:  idle {:.2}ms/frame = {:.0}% of the {} unpaced frames' elapsed time; fling {}; moves {} in / {} coalesced away",
                b.idle.as_secs_f32() * 1000.0 / unpaced.max(1) as f32,
                b.idle.as_secs_f32() * 100.0 / elapsed.max(f32::EPSILON),
                unpaced,
                b.fling_frames,
                b.moves_in,
                b.moves_dropped
            );
        }
        self.budget_stats = BudgetStats::default();

        // Reported alongside the frame cost because it is the *reason* for it
        // (../PLAN.md §11.5): before the shaped-text cache existed, every text leaf
        // was re-shaped through parley on every frame. A cache that silently stopped
        // hitting — a style that changes identity per frame, a key that grew a field
        // — would look exactly like a slow device and nothing else in the frame would
        // say otherwise. `entries` is here for the other half: it is the working set,
        // and a number that climbs with scroll distance rather than settling is the
        // symptom of rotation not evicting.
        let cache = self.renderer.shape_cache_stats();
        crate::logi!(
            "  shape cache: {:.1}% hit ({} hits / {} misses), {} entries",
            cache.hit_rate() * 100.0,
            cache.hits,
            cache.misses,
            cache.entries
        );
    }

    /// One build+layout+position pass over `self.current`. Split out of `redraw` so
    /// it can run twice in one frame when reporting viewports to `zkite_py` changes
    /// the tree (see `redraw`/`sync_viewports`); everything it touches is either
    /// rebuilt from scratch here (`Renderer`'s tree — see `zkite_renderer::Renderer::
    /// clear`) or derived state `redraw` recomputes anyway (`ime_pan_offset_y`), so
    /// running it twice costs a second layout and nothing else.
    /// The surface's size in **dp** — what layout runs against (../PLAN.md §13).
    ///
    /// This division is the entire density fix on the output side: `twcore` resolves
    /// Tailwind's `px-6` to 24, taffy lays out in whatever unit it is handed, and
    /// handing it a *device*-pixel viewport made 24 mean 24 device px — 1/density of
    /// what the same class means on the WebView backend, which resolves it against a
    /// `initial-scale=1` viewport in CSS px. Dividing here (and scaling once at paint)
    /// makes the two agree without `zkite_renderer` needing to know density exists.
    fn logical_size(&self) -> (f32, f32) {
        (self.width as f32 / self.scale, self.height as f32 / self.scale)
    }

    /// How long to wait before the next fling frame, measured from when the **last
    /// frame started** rather than from now (../PLAN.md §13.2).
    ///
    /// The old code waited a fixed `FRAME_INTERVAL` and then spent ~13ms rendering,
    /// so it delivered a frame every `interval + work` ≈ 29ms — which is exactly the
    /// gap §12.6 measured between native's frame *cost* (12.4–13.4ms, inside budget)
    /// and its delivered *interval* (25ms). Subtracting the work already done makes
    /// the period `max(frame_period, work)` instead of `frame_period + work`.
    ///
    /// Returning `ZERO` when the frame period has already elapsed is deliberate and
    /// safe: it means "render now, we are behind," and the swapchain — configured
    /// `PresentMode::Fifo` — is what actually blocks until the display is ready. This
    /// paces *to* vsync rather than replacing it; the timer's only job is to wake a
    /// thread that would otherwise sleep forever, and it should never be the thing
    /// that decides when a frame is shown.
    fn next_frame_wait(&self) -> Duration {
        match self.last_frame_at {
            Some(t) => self.frame_period.saturating_sub(t.elapsed()),
            None => Duration::ZERO,
        }
    }

    /// How long the loop may wait for a command, or `None` to block indefinitely
    /// (../PLAN.md §47) — `next_frame_wait` generalised from "is a fling running" to
    /// "does anything need the next frame".
    ///
    /// Three cases, and the middle one is the whole reason this is not
    /// `min(fling, animator)`:
    ///
    /// - **Something is moving** — a fling, or a track past its delay. Wait
    ///   `next_frame_wait()`, i.e. paced against the display exactly as a fling
    ///   already was. `Timeline::next_deadline` returns `ZERO` here, and `ZERO` means
    ///   *"a frame is wanted"*, not *"wake immediately"* — its own doc is explicit
    ///   that pacing is the render loop's business. Taking a literal `min` with it
    ///   would spin this thread at 100% CPU for the length of every animation.
    /// - **Everything is still inside its delay** — a 300ms-delayed toast. Wait the
    ///   real delay, not a frame period: eighteen wakeups that paint nothing is
    ///   precisely what `next_deadline` exists to avoid.
    /// - **Nothing at all.** Block, as before.
    fn next_wake(&self) -> Option<Duration> {
        if self.window.is_none() || !crate::SURFACE_LIFECYCLE.is_current(self.window_generation) {
            return None;
        }
        let animation = zkite_frametiming::next_wake(
            self.fling.is_some(),
            self.animator.timeline.next_deadline(Instant::now()),
            self.next_frame_wait(),
        );
        // Retry an unpresented idle frame without spinning on an occluded surface.
        if self.retry_redraw {
            let retry = Duration::from_millis(100);
            Some(animation.map_or(retry, |wait| wait.min(retry)))
        } else {
            animation
        }
    }

    /// One animation step for everything that self-paces, and a frame if anything did.
    ///
    /// Replaces the loop's old `tick_fling()` call. A fling and the animator are both
    /// "something moved without an input event"; keeping one entry point is what stops
    /// a frame being painted twice when both are running, which is the common case for
    /// a sheet that springs shut while the list behind it is still coasting.
    fn tick_animation_frame(&mut self) {
        // `tick_fling` redraws by itself (and clears `self.fling` on the tick it
        // stops), so when there is a fling it already produces this frame — and the
        // animator is sampled inside that same `redraw`.
        if self.fling.is_some() {
            self.tick_fling();
            return;
        }
        if self.retry_redraw || self.animator.timeline.is_active() {
            self.animator.stats_driven_frames += 1;
            self.redraw();
        }
    }

    /// Hand each finished animation to Python, exactly once.
    ///
    /// Drained by the loop after the frame is painted rather than inside `redraw`,
    /// because a handler may `@store.update()` and re-enter the frame path — see
    /// `Animator::finished`.
    ///
    /// The event is an ordinary `dispatch`, which is §45.3 principle 4: a completion
    /// is not a new bridge surface, it is the same call a tap makes. The payload names
    /// the channel and the trigger, because `zkite_py` routes an `exit`'s completion
    /// (drop the node) differently from an `enter`'s (nothing to do but tell the app).
    fn dispatch_animation_completions(&mut self) {
        if self.animator.finished.is_empty() {
            return;
        }
        let finished = std::mem::take(&mut self.animator.finished);
        let log = |line: &str| crate::logi!("py: {line}");
        let mut new_tree = None;
        for (zkite_id, channel, trigger) in finished {
            self.animator.stats_completed += 1;
            // Only nodes that registered a handler are dispatched to. Every animation
            // completes, most of them are decoration, and asking Python about each one
            // would put a CPython call on the end of every transition for nothing —
            // §45.2's whole constraint is that Python hears back *once*, and this is
            // where "once" can still mean "never" if the app did not ask.
            let handled = self
                .built
                .as_ref()
                .is_some_and(|b| b.handles(&zkite_id, ANIM_END_EVENT));
            if !handled {
                continue;
            }
            let payload = format!(r#"{{"channel":"{}","trigger":"{}"}}"#, channel.name(), trigger.name());
            match zkite_pybridge::dispatch(&zkite_id, ANIM_END_EVENT, &payload, &log) {
                Some(json) => match RawNode::from_json(&json) {
                    Ok(tree) => new_tree = Some(tree),
                    Err(e) => crate::loge!("animend({zkite_id}) returned malformed JSON: {e}"),
                },
                None => crate::logi!("animend({zkite_id}) unexpectedly reported unhandled"),
            }
        }
        // One redraw for the batch, from the last tree returned. Redrawing per
        // completion would paint a frame per animation for a staggered list, and every
        // dispatch above already ran against the same pre-batch tree.
        if let Some(tree) = new_tree {
            self.current = tree;
            self.redraw();
        }
    }

    /// A `MotionEvent` coordinate (device px, straight off the surface view) in dp —
    /// the input half of ../PLAN.md §13's fix, and the half most likely to be missed,
    /// because getting it wrong is *invisible in a screenshot*.
    ///
    /// Everything a touch is compared against after this — `hit_test`'s node rects,
    /// `hit_test_text_offset`'s glyph positions, `caret_rect`, `word_range_at`'s
    /// tap point — comes out of a dp layout. Leaving touches in device px would
    /// leave taps landing at `density` times their true offset: near the top-left
    /// corner it looks approximately right, and it gets worse the further from the
    /// origin the finger is, which reads as "hit-testing is flaky" rather than as a
    /// unit error. §9.7 already documented this exact failure mode for the text path
    /// ("only reproducible under a finger"), which is why the on-device verification
    /// for this change tests taps and caret placement, not just a screenshot.
    fn touch_to_dp(&self, x: f32, y: f32) -> (f32, f32) {
        (x / self.scale, y / self.scale)
    }

    /// Start whatever this tree newly declares, then write the driver's current sample
    /// over the class-resolved styles (../PLAN.md §47).
    ///
    /// **Runs after `build` and before `layout`, which is where §45.3's layer diagram
    /// said "overrides applied in `zkite_pybridge::build`" and is not the same place.**
    /// Applying *inside* the build is circular: an animation whose `from` is unstated
    /// starts from the node's resolved class value, and that value does not exist
    /// until the node has been built. Doing both here breaks the cycle with no extra
    /// tree walk — `build` collected the declarations on the pass it was already
    /// making, and neither channel affects layout, so nothing downstream can tell.
    ///
    /// Called once per `layout_frame`, i.e. **twice on a rebuild frame** (the viewport
    /// sync pass, §11.3). That is idempotent by construction rather than by luck: the
    /// second call sees `started` already populated for every key the first call
    /// started, and both calls share one `now`.
    fn drive_animations(&mut self, built: &BuiltTree, now: Instant) {
        // A tree that declares nothing is the overwhelmingly common case — every app
        // that does not animate, and every frame of one that does while it is idle.
        // Bail before touching any map so that costs nothing.
        if built.animations().is_empty() {
            if !self.animator.timeline.is_empty() {
                self.animator.timeline.retain(|_| false);
                self.animator.started.clear();
            }
            self.report_animation_errors(built);
            return;
        }
        self.report_animation_errors(built);

        self.animator.live.clear();
        for decl in built.animations() {
            for spec in &decl.specs {
                let key: AnimKey = (decl.zkite_id.clone(), spec.channel);
                self.animator.live.insert(key.clone());

                // Whether to start is a decision with a host test behind it
                // (`zkite_pybridge::anim::should_start`) rather than three lines
                // written here, for this file's standing reason: `gpu_harness` does
                // not build for the host (../PLAN.md §15.5), so logic that lives here
                // can only ever be checked on a device. The property it carries is the
                // one most likely to be wrong in a way that looks like a rendering
                // bug — a declared entrance restarting itself forever.
                let previous = self.animator.started.get(&key).map(|(to, _)| *to);
                if !zkite_pybridge::should_start(previous, spec) {
                    continue;
                }

                // The `from` an unstated declaration gets: this node's own resolved
                // class value, which only exists now that the node is built.
                let class_value = self.renderer.channel_value(decl.node, spec.channel).unwrap_or(0.0);
                let track = spec.track(class_value);
                if self.animator.timeline.start(key.clone(), track, now, spec.trigger.interrupt()) {
                    self.animator.stats_started += 1;
                }
                self.animator.started.insert(key, (spec.to, spec.trigger));
            }
        }

        // Evict everything the tree no longer declares. Without this a `List` that
        // animates its rows accumulates one held entry per row ever scrolled past —
        // `Timeline::retain`'s own doc names that case — and a node whose declaration
        // was removed would keep its final value forever instead of returning to what
        // its class string says.
        let live = &self.animator.live;
        self.animator.timeline.retain(|k| live.contains(k));
        self.animator.started.retain(|k, _| live.contains(k));

        // Write this frame's values over the class-resolved styles. Only the nodes
        // that declared something are visited, and only the channels they declared.
        for decl in built.animations() {
            for spec in &decl.specs {
                let key: AnimKey = (decl.zkite_id.clone(), spec.channel);
                if let Some(v) = self.animator.timeline.value(&key) {
                    self.renderer.animate(decl.node, spec.channel, v);
                }
            }
        }
    }

    /// Log malformed `anim` declarations, once per distinct report.
    ///
    /// A broken declaration is re-parsed on every rebuilt frame, so logging it from
    /// the parse site would print it sixty times a second during a scroll — the same
    /// reason the `frame:` line is change-gated (../PLAN.md §7.14). Reported through
    /// `loge!` rather than being swallowed, because a silently-dropped animation is
    /// exactly the §15.1 divergence this feature's strictness exists to avoid.
    fn report_animation_errors(&mut self, built: &BuiltTree) {
        let report = if built.animation_errors().is_empty() {
            None
        } else {
            Some(built.animation_errors().join("; "))
        };
        if report != self.animator.last_error_report {
            if let Some(r) = &report {
                crate::loge!("ZKITEANIM bad declaration: {r}");
            }
            self.animator.last_error_report = report;
        }
    }

    fn update_selection_overlay_rect(&self, rect: (f32, f32, f32, f32)) {
        let (Some(env), Some(view)) = (self.render_env, self.surface_view) else { return };
        unsafe {
            let vt = **env;
            let class = vt.GetObjectClass.unwrap()(env, view.0);
            let method = vt.GetMethodID.unwrap()(env, class,
                c"updateSelectionOverlayRect".as_ptr(), c"(FFFF)V".as_ptr());
            if !method.is_null() {
                let s = self.scale;
                let args = [jni_sys::jvalue { f: rect.0*s }, jni_sys::jvalue { f: rect.1*s },
                    jni_sys::jvalue { f: rect.2*s }, jni_sys::jvalue { f: rect.3*s }];
                vt.CallVoidMethodA.unwrap()(env, view.0, method, args.as_ptr());
            }
            vt.DeleteLocalRef.unwrap()(env, class);
            if vt.ExceptionCheck.unwrap()(env) != 0 {
                vt.ExceptionDescribe.unwrap()(env);
                vt.ExceptionClear.unwrap()(env);
            }
        }
    }

    fn layout_frame(&mut self, now: Instant) -> LayoutFrame {
        let mut phases = FramePhases::default();
        let focused = self.focused.as_ref().map(|zkite_id| FocusedField { zkite_id, text: &self.focused_text });
        let t = Instant::now();
        let built = zkite_pybridge::build(&mut self.renderer, &self.current, focused.as_ref());
        phases.build = t.elapsed();
        // Navigation or a conditional view can remove the focused field without a tap.
        if self.focused.as_deref().is_some_and(|id| !built.is_text_field(id)) {
            self.focused = None;
            self.selection_overlay_field = None;
            self.call_hide_selection_overlay_method();
            self.call_blur_method();
        }
        // Inside `build`'s phase rather than its own column: it is part of turning the
        // tree into a laid-out frame, it is bounded by the number of *animating* nodes
        // rather than by the tree, and §46's lesson is that a `ZKITEPERF` column
        // covering more than one kind of work eventually gets modelled as one. The
        // `anim:` line reports the driver's own behaviour instead — counts, not time.
        let t_anim = Instant::now();
        self.drive_animations(&built, now);
        phases.build += t_anim.elapsed();
        let t = Instant::now();
        let (logical_w, logical_h) = self.logical_size();
        self.renderer.layout(built.root, logical_w, logical_h);
        phases.layout = t.elapsed();
        built.clamp_scroll_offsets(&self.renderer, &mut self.scroll_offsets);
        let base_offsets = built.scroll_offsets_for(&self.scroll_offsets);
        let t = Instant::now();
        let unpanned_nodes = self.renderer.positioned(built.root, &base_offsets);
        phases.positioned = t.elapsed();

        // IME pan-into-view (../PLAN.md §5/§7.5): computed fresh every redraw from
        // this frame's *unpanned* layout (never accumulated), then applied as one
        // more scroll-offset entry keyed on the tree's own root — `positioned`/
        // `hit_test` already know how to shift any node's descendants by an
        // arbitrary offset without moving the node itself (that's the exact
        // mechanism an ordinary scrolled container uses), so reusing it here for a
        // synthetic "root scroll" gets consistent painting *and* touch hit-testing
        // for free (see `node_scroll_offsets`) instead of a bespoke transform
        // threaded through every touch handler by hand.
        self.ime_pan_offset_y = self.compute_ime_pan(&built, &unpanned_nodes);
        let nodes = if self.ime_pan_offset_y != 0.0 {
            let mut offsets = base_offsets;
            offsets.insert(built.root, (0.0, self.ime_pan_offset_y));
            let t = Instant::now();
            let panned = self.renderer.positioned(built.root, &offsets);
            phases.positioned += t.elapsed();
            panned
        } else {
            unpanned_nodes
        };
        if self.selection_overlay_field.is_some() {
            if let Some(container) = built.focused_text_node.and_then(|n| self.renderer.parent(n)) {
                if let Some(n) = nodes.iter().find(|n| n.node == container) {
                    self.update_selection_overlay_rect(n.rect);
                }
            }
        }
        LayoutFrame { built, nodes, phases }
    }

    /// Reports every scrollable container's current `(scroll_y, viewport_height)` to
    /// `zkite_py` and returns whether that produced a new tree (../PLAN.md §11).
    ///
    /// This is the one place scroll position flows *backwards*, from the renderer
    /// into the widget tree — a virtualised `List` can't choose which rows to build
    /// without it. Every scrollable container is reported uniformly rather than only
    /// the virtualised ones: this side genuinely cannot tell them apart without
    /// duplicating `List`'s own bookkeeping, and the Python side already answers
    /// "nothing to do" for a plain `ScrollView` in two dict lookups
    /// (`runtime.py`'s `_zkite_scroll_tree`).
    ///
    /// `reported_viewports` skips the call entirely when neither number moved, which
    /// is every redraw that isn't a scroll — a tap, a focus change, an IME pan. A
    /// drag or fling *does* reach Python once per frame; that cost is measured in
    /// ../PLAN.md §11 rather than assumed away.
    ///
    /// **Deliberately still y-only, after ../PLAN.md §27 added horizontal scrolling.**
    /// The only consumer of this channel is `List`, which is vertically virtualised
    /// and has no horizontal form, so there is nothing on the Python side that could
    /// use an x report. A horizontal strip declares `overflow-x-auto` and *not*
    /// `overflow-y-*`, so `is_scrollable_y` skips it and it costs nothing per frame.
    /// The consequence to know about: a horizontally scrolled container's position is
    /// never visible to Python, so there is no horizontal virtualisation — a
    /// horizontal strip builds all of its children, every frame.
    fn sync_viewports(&mut self, frame: &LayoutFrame, phases: &mut FramePhases) -> bool {
        let mut changed = false;
        for pn in &frame.nodes {
            if !self.renderer.is_scrollable_y(pn.node) {
                continue;
            }
            let Some(zkite_id) = frame.built.zkite_id(pn.node) else { continue };
            let scroll_y = self.scroll_offsets.get(zkite_id).map(|o| o.1).unwrap_or(0.0);
            let viewport_h = pn.rect.3;
            let zkite_id: Arc<str> = Arc::from(zkite_id);
            if self.reported_viewports.get(&zkite_id) == Some(&(scroll_y, viewport_h)) {
                continue;
            }
            self.reported_viewports.insert(zkite_id.clone(), (scroll_y, viewport_h));

            let log = |line: &str| crate::logi!("py: {line}");
            // Timed separately from the parse below (§43). The two are quoted as one
            // number by ../PLAN.md §42.5, and only this one is work a smaller rebuild
            // could avoid — the parse walks whatever JSON arrives.
            //
            // **Charged to `sync_py` only when a tree came back.** The same call on a
            // scroll that moved no window returns `None` after two dict lookups
            // (`runtime.py`'s `_zkite_scroll_tree`) and happens on ~93% of scroll
            // frames at `overscan=4`; folding those into the same field and then
            // dividing by the rebuild count — which is what the `rebuild:` line does —
            // would multiply their cost by ~15 and attribute it to the rebuild.
            let t = Instant::now();
            let scrolled = zkite_pybridge::scroll(&zkite_id, scroll_y, viewport_h, &log);
            let py = t.elapsed();
            let Some(json) = scrolled else {
                // The "nothing to do" path, measured rather than assumed cheap. §40.4
                // names this exact signature as a distinct bug — a window with
                // `rebuilt 0` and a large `sync` means the cheap path is not cheap —
                // and until now nothing could see it separately from the rebuild.
                phases.sync_poll += py;
                continue;
            };
            phases.sync_py += py;
            // Recorded per rebuild, not per frame: it is the denominator that turns
            // the two timings above into a per-byte (and so per-row) cost without
            // needing a second A/B over `overscan` to establish the slope.
            self.rebuild_bytes += json.len() as u64;
            let t = Instant::now();
            let parsed = RawNode::from_json(&json);
            phases.sync_parse += t.elapsed();
            match parsed {
                Ok(tree) => {
                    self.current = tree;
                    changed = true;
                }
                Err(e) => crate::loge!("scroll({zkite_id}) returned malformed JSON: {e}"),
            }
        }
        changed
    }

    /// How far to pan the whole tree up so the focused field's own container isn't
    /// covered by the keyboard (../PLAN.md §5/§7.5) — `0.0` if nothing's focused,
    /// the keyboard isn't showing (`ime_inset_bottom <= 0.0`), or the field isn't
    /// actually covered. `nodes` must be `built`'s *unpanned* positioned tree (this
    /// frame's, from `redraw`, before any pan offset is applied) — feeding it an
    /// already-panned tree would double-count the shift. Uses the field's
    /// *container* rect, the same node `show_selection_overlay` positions the
    /// selection overlay against (see that method's own doc comment on why: the
    /// text leaf itself carries no padding/background, so it under-reports how much
    /// of the field is actually visible). Clamped two ways: never more than
    /// `ime_inset_bottom` itself (panning further would reveal empty space below
    /// the tree that the keyboard isn't actually covering), and never more than the
    /// field's own unpanned top (so a keyboard taller than the space above the
    /// field pans it as far as it can go, flush to the top, rather than pushing it
    /// — and everything above it — off-screen chasing an unreachable target).
    fn compute_ime_pan(&self, built: &BuiltTree, nodes: &[zkite_renderer::PositionedNode]) -> f32 {
        if self.focused.is_none() || self.ime_inset_bottom <= 0.0 {
            return 0.0;
        }
        let Some(text_node) = built.focused_text_node else { return 0.0 };
        let Some(container_node) = self.renderer.parent(text_node) else { return 0.0 };
        let Some(container) = nodes.iter().find(|n| n.node == container_node) else { return 0.0 };
        let (_, field_top, _, field_h) = container.rect;
        let field_bottom = field_top + field_h;
        // Both terms in dp: the container rect comes from a dp layout, and
        // `ime_inset_bottom` was converted on arrival (see `Command::ImeInsetsChanged`).
        // Mixing the two spaces here would pan by roughly `density` times too much and
        // is exactly the kind of error a screenshot makes look like a layout bug.
        let (_, logical_h) = self.logical_size();
        let visible_bottom = logical_h - self.ime_inset_bottom;
        let needed = field_bottom - visible_bottom;
        if needed <= 0.0 {
            return 0.0;
        }
        needed.min(self.ime_inset_bottom).min(field_top.max(0.0))
    }
}

/// The caret's rect for the focused field's own (already-positioned) text node, at a
/// real byte offset into `content` — via `zkite_renderer::Renderer::
/// text_caret_geometry` (../PLAN.md §7.7), which shapes `content` fresh through
/// `parley::Cursor` rather than approximating from already-shaped glyph positions.
/// Replaces an earlier end-of-text-only approximation (see git history / phase_3/
/// PLAN.md §7.6's write-up if you're looking for why that approach was limited).
fn caret_rect(
    renderer: &mut Renderer,
    nodes: &[zkite_renderer::PositionedNode],
    text_node: NodeId,
    content: &str,
    byte_offset: usize,
) -> Option<(f32, f32, f32, f32)> {
    let n = nodes.iter().find(|n| n.node == text_node)?;
    let (x, y, _w, _h) = n.rect;
    // `text_caret_geometry` shapes `content` itself, independent of the byte length
    // of whatever was last rendered — clamp defensively rather than let a stale
    // offset (e.g. from a selection captured just before a backspace) index past
    // the end of a *shorter* current string.
    let byte_offset = byte_offset.min(content.len());
    let (local_x, local_y, w, h) = renderer.text_caret_geometry(&n.style.text, content, byte_offset, 2.0);
    Some((x + local_x, y + local_y, w, h))
}

/// Expands a byte offset into `text` to the boundaries of the "word" touching it —
/// the initial selection a long-press seeds (../PLAN.md §7.8), same default
/// behavior every native text view gives for free. Real Unicode word segmentation
/// (UAX #29, via `unicode-segmentation`'s `split_word_bound_indices`), not the
/// original ASCII-alphanumeric-run approximation this replaced — that version fell
/// back to a zero-width selection for CJK or any other non-ASCII-alphanumeric
/// script's own text, a real, documented gap fixed here rather than carried
/// forward. `segment.chars().any(char::is_alphanumeric)` mirrors exactly the
/// predicate the crate's own `unicode_words()` uses internally to tell a "word"
/// segment from a whitespace/punctuation one (checked directly against that
/// method's source, since the crate exposes the *filter*, `unicode_words()`, but
/// not a byte-offset-preserving version of it — `split_word_bound_indices` is the
/// one that keeps offsets, so the filter is reapplied here instead of on top of
/// `unicode_words()` itself). Iterates in text order and returns the first segment
/// containing `offset` at either edge, matching the original implementation's own
/// left-biased behavior at a word/non-word boundary (confirmed by hand against
/// both boundary directions, not assumed). A tap that lands only in whitespace/
/// punctuation still degrades to a zero-width selection at the tap point — no
/// script produces a meaningful "word" there either way.
/// A slider value on the wire (../PLAN.md §26).
///
/// Plain decimal, no exponent and no trailing noise, because the far end is
/// `zkite_py`'s `_as_float` calling `float(str)` — which accepts far more than this
/// emits, but the value also lands in a user's `@store` and gets rendered
/// (`f"Volume: {v}"`), so `40` reading back as `40` rather than `40.00000762939453`
/// is worth one line here. Rust's `{}` for `f32` already prints the shortest
/// round-tripping form; this only strips the `.0` that an integral step would
/// otherwise leave, matching what `<input type=range>` puts in its own `value`.
fn format_slider_value(value: f32) -> String {
    let s = format!("{value}");
    s.strip_suffix(".0").map(str::to_string).unwrap_or(s)
}

fn word_range_at(text: &str, offset: usize) -> (usize, usize) {
    use unicode_segmentation::UnicodeSegmentation;
    let offset = offset.min(text.len());
    for (start, segment) in text.split_word_bound_indices() {
        let end = start + segment.len();
        if offset >= start && offset <= end && segment.chars().any(char::is_alphanumeric) {
            return (start, end);
        }
    }
    (offset, offset)
}

/// `zkite_renderer::Color`'s four separate `u8` fields -> Android's own packed `0xAARRGGBB`
/// `int` (`android.graphics.Color`'s convention) — what `ZKiteSurfaceView.
/// showSelectionOverlay` expects for `textColor`/`bgColor` (../PLAN.md §7.8).
fn pack_argb(c: zkite_renderer::Color) -> i32 {
    ((c.a as i32) << 24) | ((c.r as i32) << 16) | ((c.g as i32) << 8) | (c.b as i32)
}

// ---------------------------------------------------------------------- GPU backend

/// The `wgpu::MemoryHints` the GPU device is requested with.
///
/// **This is the fix for ../PLAN.md §14.7's unexplained 272 MB of `Graphics`.** Left
/// to `..Default::default()` this is `MemoryHints::Performance`, which sets
/// `gpu-allocator`'s suballocation block sizes to a *minimum* of 128 MB device-local
/// and 64 MB host-visible (wgpu-hal `AllocationSizes::from_memory_hints`). Those
/// minimums are reserved on first use regardless of how much of them a scene needs,
/// which is why the cost was flat at ~214 MB across a 5-node app, a 6-node app and a
/// 152-node list — it was never sized by the content, or by the screen, but by a
/// constant written for desktop GPUs.
///
/// Measured on the Moto G54, one line differing between three builds (§20.2/§20.3),
/// stock counter / 10,000-row list, screen-on and foregrounded:
///
/// | `memory_hints` | `GL mtrack` | total PSS | scroll frame interval |
/// |---|---|---|---|
/// | `Performance` (wgpu default) | 214 MB | 334 / 314 MB | 16.90 – 17.53 ms |
/// | `MemoryUsage` | 79 MB | 195 / 179 MB | 16.84 – 17.36 ms |
/// | **`Manual{4..8 MB}`** (this) | **36 MB** | **155 / 135 MB** | 16.84 – 17.55 ms |
///
/// wgpu names this a memory-versus-performance tradeoff, so the performance side was
/// measured rather than assumed; on this workload it does not move at any setting,
/// because ZKite allocates almost nothing per frame (§11.9's shape cache and §18.3's
/// icon-path cache are why) and so block size has nothing to bite on.
///
/// `MemoryUsage` is the more conservative choice — a named preset wgpu maintains
/// rather than two constants pinned here — and costs 44 MB more on the list. Recorded
/// so the choice can be revisited rather than rediscovered.
///
/// Kept as a named constant so that A/B stays one line, the way §11.9's shape-cache
/// bypass and §13.2's pacing A/B were. **Re-measure if `Image` ever lands** — a real
/// image atlas allocates far more GPU memory than anything ZKite draws today, and small
/// blocks mean more `VkDeviceMemory` allocations. Oversized resources get dedicated
/// allocations so there is no correctness risk, only a tuning one (§20.5).
const ZKITE_MEMORY_HINT: wgpu::MemoryHints = wgpu::MemoryHints::Manual {
    suballocated_device_memory_block_size: (4 * 1024 * 1024)..(8 * 1024 * 1024),
};

fn try_init_gpu() -> Result<Gpu, String> {
    if std::env::var("ZKITE_FORCE_CPU_RENDERER").as_deref() == Ok("1") {
        return Err("ZKITE_FORCE_CPU_RENDERER=1".to_string());
    }
    let instance = wgpu::Instance::default();
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::LowPower,
        force_fallback_adapter: false,
        compatible_surface: None,
    }))
    .map_err(|e| format!("request_adapter: {e}"))?;
    let info = adapter.get_info();
    crate::logi!("wgpu adapter: {} ({:?}, backend {:?})", info.name, info.device_type, info.backend);
    // `required_limits: Limits::default()` (via `..Default::default()`) is a
    // desktop-shaped baseline, and it's a *floor* the adapter must clear, not a
    // request it downgrades to meet. This session's real-device run (Moto G54 5G,
    // PowerVR BXM-8-256) failed here: `max_inter_stage_shader_variables` value 16 is
    // better than allowed 15" — this actual mobile GPU supports *less* than wgpu's
    // default asks for, one limit, by one. `adapter.limits()` reports what this
    // specific adapter actually supports (already downward-clamped from the
    // implementation's true capability by wgpu-hal), so requesting exactly that
    // — rather than a fixed baseline — is what should make the *real* GPU path
    // reachable on real, current, mid-range hardware instead of only on
    // desktop-class ones. See ../PLAN.md §4 for whether re-running on the same
    // device after this fix actually confirmed that.
    // ../PLAN.md §20's concrete first step, which §14.7 named and no session had
    // taken: this adapter's real limits have never been logged, and several of
    // `vello_hybrid`'s textures are sized `max_texture_dimension_2d` wide. Permanent
    // rather than temporary — it is two lines, it is the input to every
    // texture-size calculation in the renderer below us, and not having it is what
    // made §14.7's search take four sessions to start.
    let lim = adapter.limits();
    crate::logi!(
        "ZKITEMEM limits max_texture_dimension_2d={} max_texture_array_layers={} \
         max_buffer_size={} max_storage_buffer_binding_size={}",
        lim.max_texture_dimension_2d,
        lim.max_texture_array_layers,
        lim.max_buffer_size,
        lim.max_storage_buffer_binding_size,
    );
    crate::logi!("ZKITEMEM memory_hint={:?}", ZKITE_MEMORY_HINT);

    // `memory_hints` is set explicitly rather than inherited from
    // `..Default::default()` — see `ZKITE_MEMORY_HINT`'s own doc comment for the
    // measurement and why the default is the wrong one on a phone.
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("zkite-gpu-harness"),
        required_features: wgpu::Features::empty(),
        required_limits: lim,
        memory_hints: ZKITE_MEMORY_HINT,
        ..Default::default()
    }))
    .map_err(|e| format!("request_device: {e}"))?;

    // **Say what wgpu is complaining about.** Without this, a validation error is
    // reported by `get_current_texture` as the bare word `Validation` and its message
    // goes nowhere at all — there is no `log` bridge in this crate, so wgpu's own
    // `log::error!` output is not in logcat either. §39 spent a measurement pass on a
    // permanent GPU→CPU fallback whose cause was one string this handler would have
    // printed. Costs nothing when no error is raised.
    device.on_uncaptured_error(std::sync::Arc::new(|e: wgpu::Error| {
        // `{e:?}`, not `{e}`: `Display` for `wgpu::Error::Validation` prints the bare
        // words "Validation Error" and drops the description, which is the one thing
        // worth logging. Measured on the device — the first version of this line printed
        // exactly that and no more.
        crate::loge!("wgpu uncaptured error: {e:?}");
    }));

    Ok(Gpu {
        instance,
        adapter,
        device,
        queue,
        surface: None,
        hybrid_renderer: None,
        resources: vello_hybrid::Resources::new(),
    })
}

/// Creates the `wgpu::Surface` for `window` if one doesn't already exist. Idempotent
/// on purpose — a `NewWindow` for a window that already has a live surface (shouldn't
/// happen given the state machine, but cheap to make impossible rather than assumed)
/// and, more importantly, a `Resize` on the *same* still-live window must never reach
/// `create_surface_unsafe` a second time. That second call, against a still-in-use
/// `ANativeWindow`, is exactly this session's second real on-device bug — Vulkan
/// correctly refuses it (`ERROR_NATIVE_WINDOW_IN_USE_KHR`), caught only because the
/// panic hook fixed alongside this session's *first* bug made it visible at all
/// instead of another silent black screen. See `State::handle`'s `Resize` arm.
fn ensure_surface(gpu: &mut Gpu, window: &NativeWindow) {
    if gpu.surface.is_some() {
        crate::logi!("ensure_surface: reusing existing surface");
        return;
    }
    crate::logi!("ensure_surface: creating a new surface");
    // Uses `gpu.instance` — *not* a fresh `wgpu::Instance::default()` — because an
    // `Adapter` and any `Surface` it's queried/used with must come from the same
    // `Instance`. A fresh second instance here was this session's *first* real
    // on-device bug: it silently panicked inside `create_surface_unsafe`/
    // `get_capabilities` with no visible error until the panic hook (added in the
    // same fix) made panics on this thread visible in logcat at all. See
    // ../PLAN.md §4 for the full account.
    let raw_window_handle = raw_window_handle::RawWindowHandle::AndroidNdk(
        raw_window_handle::AndroidNdkWindowHandle::new(window.ptr().cast()),
    );
    let raw_display_handle =
        raw_window_handle::RawDisplayHandle::Android(raw_window_handle::AndroidDisplayHandle::new());
    // SAFETY: `window` (and the `ANativeWindow` it wraps) is kept alive by `State` for
    // at least as long as this surface — see `Gpu::surface`'s doc comment and
    // `State::handle`'s `DestroyWindow` arm, which drops the surface first.
    match unsafe {
        gpu.instance.create_surface_unsafe(wgpu::SurfaceTargetUnsafe::RawHandle {
            raw_display_handle: Some(raw_display_handle),
            raw_window_handle,
        })
    } {
        Ok(s) => gpu.surface = Some(s),
        Err(e) => crate::loge!("create_surface_unsafe failed: {e}"),
    }
}

/// (Re)configures the current surface for `width`x`height` and rebuilds the
/// `vello_hybrid::Renderer` to match (its `RenderTargetConfig` is fixed-size at
/// construction, so a resize needs a new one, not a mutation). Safe to call
/// repeatedly on the same still-live surface — this is the `Resize` path as well as
/// the tail end of `NewWindow`'s.
fn reconfigure(
    gpu: &mut Gpu,
    width: i32,
    height: i32,
    images: Option<&mut zkite_imagestore::ImageStore>,
) {
    if width <= 0 || height <= 0 {
        crate::logi!("reconfigure: skipping, size not known yet ({width}x{height})");
        return;
    }
    let Some(surface) = gpu.surface.as_ref() else {
        crate::loge!("reconfigure: no surface yet");
        return;
    };

    let caps = surface.get_capabilities(&gpu.adapter);
    // Prefer a plain (non-sRGB) 8-bit format over whatever happens to be first: this
    // session's first real frame (emulator, format defaulted to the first-listed
    // `Rgba8UnormSrgb`) rendered visibly washed-out — `slate-950` (rgb 2,6,23, nearly
    // black) came out a clearly lighter navy. `Rgba8UnormSrgb` tells the hardware "the
    // values written are linear; convert to sRGB gamma on write," but `vello_hybrid`'s
    // own `render_to_file.rs` example targets plain `Rgba8Unorm` — its pipeline
    // already produces sRGB-encoded bytes matching `twcore::Color`'s own values
    // (straight 0-255 display values, per that type's doc comment), so asking the
    // hardware to gamma-encode them a second time is exactly a double-correction bug,
    // not a stylistic choice. Falls back to whatever's first if no plain format is
    // offered at all, rather than failing outright.
    let format = caps
        .formats
        .iter()
        .copied()
        .find(|f| matches!(f, wgpu::TextureFormat::Rgba8Unorm | wgpu::TextureFormat::Bgra8Unorm))
        .or_else(|| caps.formats.first().copied());
    let Some(format) = format else {
        crate::loge!("surface reports no supported formats at all");
        gpu.hybrid_renderer = None;
        return;
    };
    // `Opaque` is the natural choice (this is a full-screen app with no transparency
    // anywhere), but "natural" isn't "supported": this session's on-device run hit
    // `Requested alpha mode Opaque is not in the list of supported alpha modes:
    // [Inherit]` (SwiftShader on the emulator; see ../PLAN.md §4). Same principle as
    // `format` just above — query what the surface actually offers rather than
    // asserting what it "should" support, since Android's alpha-mode support is
    // compositor/driver-dependent, not just spec-guaranteed.
    let alpha_mode = if caps.alpha_modes.contains(&wgpu::CompositeAlphaMode::Opaque) {
        wgpu::CompositeAlphaMode::Opaque
    } else {
        caps.alpha_modes[0]
    };
    crate::logi!(
        "surface format: {format:?} (of {} supported), alpha mode: {alpha_mode:?} (of {:?})",
        caps.formats.len(),
        caps.alpha_modes
    );

    surface.configure(
        &gpu.device,
        &wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: width as u32,
            height: height as u32,
            present_mode: wgpu::PresentMode::Fifo,
            desired_maximum_frame_latency: 2,
            alpha_mode,
            view_formats: vec![],
        },
    );

    // **Every uploaded image dies with the renderer below, so give the slots back and
    // forget the handles** (../PLAN.md §35.5). The atlas texture lives inside
    // `Renderer`; the residency bookkeeping lives in `ImageStore` and in the shared
    // `Resources`. Replacing the renderer without this leaves `ImageStore` certain
    // that every image is already uploaded, so nothing re-uploads and every `Image`
    // paints an **opaque black rectangle** — the slot resolves, and nothing ever wrote
    // it. That is what the phone showed while the identical code was pixel-perfect
    // against Chromium on the host, because the host harness renders one frame and
    // never resizes. And this is not an edge case: `reconfigure` runs *twice on every
    // launch* (see the note below), so it was every image, every time.
    //
    // Done here, before the assignment, because `destroy_image` needs the renderer
    // that owns the slots — a moment later it is gone, and the slots are allocated in
    // `Resources`, which is not, so they would leak once per image per resize.
    if let Some(store) = images {
        let stale = store.drain_gpu_ids();
        if !stale.is_empty() {
            if let Some(old) = gpu.hybrid_renderer.as_mut() {
                let mut enc = gpu
                    .device
                    .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("zkite-image-evict") });
                for id in stale {
                    old.destroy_image(&mut gpu.resources, &mut enc, id);
                }
                gpu.queue.submit([enc.finish()]);
            }
        }
    }

    // Note: assigning `Some(Renderer::new(..))` keeps the *old* renderer alive for the
    // duration of the new one's construction (Rust evaluates the RHS first), and
    // `reconfigure` runs twice on every launch — `NewWindow` then the `Resize` Android
    // always delivers straight after. That looked like a clean explanation for §13.3's
    // 266 MB of `Graphics`, since half of 272 is 136. **Measured on the Moto G54 and
    // refuted:** dropping the old renderer first (`hybrid_renderer = None;` on its own
    // line, ahead of this) moved GL mtrack from 215035 KB to 215024 KB — an 11 KB
    // difference on a 210 MB allocation, i.e. nothing. Reverted rather than kept, since
    // a line that reads like a memory fix and isn't one is worse than its absence. See
    // ../PLAN.md §14.7 for what that does and doesn't rule out.
    gpu.hybrid_renderer = Some(vello_hybrid::Renderer::new(
        &gpu.device,
        &vello_hybrid::RenderTargetConfig { format, width: width as u32, height: height as u32 },
    ));
}

/// What one `paint_gpu` attempt did, and what the caller should do about it.
///
/// Replaces an `Option<()>` whose `None` meant two incompatible things: "this device has
/// no working GPU path, paint on the CPU instead" (which is what plan1.md §5/§9's CPU
/// fallback is *for*) and "the swapchain hiccuped on this one frame". §39 measured what
/// conflating them costs: **one transient `get_current_texture: Validation` mid-scroll
/// downgraded the app to the CPU rasteriser permanently** — ~23-29ms per frame against
/// the GPU path's ~11 — because nothing ever reconfigured the surface afterwards and
/// every later frame hit the same error. The comment on the old code said recovery
/// happened "on the next `Resize`/`NewWindow`"; during a scroll neither ever arrives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PaintOutcome {
    /// A frame reached the screen.
    Painted,
    /// Nothing was painted and **nothing should be**: the surface is momentarily
    /// unavailable (timeout, occluded) or has just been reconfigured. The last good
    /// frame stays on screen, which is correct and is what a dropped frame *is*; the
    /// next redraw paints normally. Deliberately not the CPU fallback — see `Fallback`.
    Skip,
    /// The surface is gone and must be recreated from the window, which only the caller
    /// holds. Skips this frame too.
    Recreate,
    /// There is no usable GPU path at all — paint on the CPU, as plan1.md §5 intends.
    Fallback,
}

fn paint_gpu(
    gpu: &mut Gpu,
    width: i32,
    height: i32,
    scale: f32,
    nodes: &[zkite_renderer::PositionedNode],
    caret: Option<(f32, f32, f32, f32)>,
    phases: &mut FramePhases,
    images: &mut Option<Box<zkite_imagestore::ImageStore>>,
) -> PaintOutcome {
    // Say *why* the GPU path bailed, rather than `?`-ing out silently: a fall-through
    // to `paint_cpu_fallback` is invisible on screen (both paths render the same
    // picture) but means something is wrong with the surface, and the un-instrumented
    // early-returns this replaces gave no signal at all that it was happening. See
    // ../PLAN.md §7.14 — a silent fall-through was one of the wrong turns the §7.6
    // investigation had to rule out by hand for want of exactly this line.
    if gpu.surface.is_none() {
        crate::logi!("paint_gpu: no surface yet, falling through to CPU");
        return PaintOutcome::Fallback;
    }
    if gpu.hybrid_renderer.is_none() {
        crate::logi!("paint_gpu: no hybrid_renderer yet, falling through to CPU");
        return PaintOutcome::Fallback;
    }

    // The acquire happens **before** the uploads and the scene, which is a change §39
    // had to make rather than chose: recovering from a stale surface means calling
    // `reconfigure`, and that replaces the `vello_hybrid::Renderer` and with it the image
    // atlas (§35.5). Uploading first and reconfiguring second would paint every `Image`
    // as an opaque black rectangle — the exact bug §35.5 documents. Acquiring first
    // keeps the recovery ahead of everything that depends on the renderer's identity.
    let acquire_started = Instant::now();
    let mut frame = None;
    for attempt in 0..2 {
        let Some(surface) = gpu.surface.as_ref() else { return PaintOutcome::Fallback };
        match surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(t) => {
                frame = Some(t);
                break;
            }
            // Still presentable this frame. wgpu's own doc recommends reconfiguring for
            // *optimal performance*, not for correctness, so this paints and lets the
            // next `Resize` deal with it — unchanged from before §39.
            wgpu::CurrentSurfaceTexture::Suboptimal(t) => {
                frame = Some(t);
                break;
            }
            // Nothing is wrong with the surface; there is just no buffer right now.
            // Skipping is what wgpu's own doc says to do, and it is *much* cheaper than
            // repainting the whole screen on the CPU.
            status @ (wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded) => {
                crate::logi!("get_current_texture: {status:?}; skipping this frame");
                return PaintOutcome::Skip;
            }
            wgpu::CurrentSurfaceTexture::Lost => {
                crate::loge!("get_current_texture: Lost; recreating the surface");
                gpu.surface = None;
                gpu.hybrid_renderer = None;
                return PaintOutcome::Recreate;
            }
            // The two recoverable-by-reconfiguring statuses. One retry, then give up on
            // *this frame only* — a loop that kept retrying would spin the render thread
            // on a genuinely broken surface.
            status @ (wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Validation) => {
                if attempt == 0 {
                    crate::loge!("get_current_texture: {status:?}; reconfiguring and retrying");
                    reconfigure(gpu, width, height, images.as_deref_mut());
                    continue;
                }
                crate::loge!("get_current_texture: {status:?} again after reconfigure; skipping this frame");
                return PaintOutcome::Skip;
            }
        }
    }
    let Some(frame) = frame else { return PaintOutcome::Skip };
    phases.acquire += acquire_started.elapsed();

    // Re-taken after the acquire, because `reconfigure` above may have replaced both.
    let Some(hybrid_renderer) = gpu.hybrid_renderer.as_mut() else {
        crate::logi!("paint_gpu: reconfigure left no renderer; skipping this frame");
        return PaintOutcome::Skip;
    };

    // Uploads before the scene is encoded, because the scene refers to bitmaps by
    // the `ImageId` this produces — see ../images.rs. Cheap and allocation-free on
    // every frame after the first for a given set of images.
    if let Some(store) = images.as_deref_mut() {
        crate::images::upload_pending(
            store,
            nodes,
            hybrid_renderer,
            &mut gpu.resources,
            &gpu.device,
            &gpu.queue,
        );
    }

    let scene_started = Instant::now();
    let mut scene = vello_hybrid::Scene::new(width as u16, height as u16);
    // The dp -> device-px conversion, applied once for the whole scene (../PLAN.md
    // §13). Everything below this line is in dp; the canvas is in device pixels.
    //
    // A scene transform rather than pre-multiplying each rect, for text: `glyph_run`
    // hands the current scene transform to `glifo`, whose `prepare_glyph_run`
    // *absorbs* a uniform scale into the font size (`AbsorbScaleHinted` ->
    // `vertical_font_size = font_size * scale`) and builds the hinting instance at
    // that size. So glyphs are outlined and hinted at real device resolution — which
    // pre-scaling the rects by hand would not have achieved, since the glyph runs
    // would still have carried dp font sizes.
    let base_transform = kurbo::Affine::scale(scale as f64);
    scene.set_transform(base_transform);
    paint_nodes_into_scene(&mut scene, &mut gpu.resources, nodes, base_transform, images.as_deref_mut());
    // Painted last (on top of everything else) and outside `paint_nodes_into_scene`
    // on purpose — it has no `PositionedNode` of its own (see `caret_rect`'s doc
    // comment on why this is an approximation, not a real per-node property).
    if let Some((x, y, w, h)) = caret {
        scene.set_paint(peniko::Color::from_rgba8(255, 255, 255, 255));
        fill_background(&mut scene, x, y, w, h, &zkite_renderer::Corners::default());
    }

    phases.scene += scene_started.elapsed();

    let present_started = Instant::now();
    let view = frame.texture.create_view(&wgpu::TextureViewDescriptor::default());
    let mut encoder =
        gpu.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("zkite-frame") });
    let render_size = vello_hybrid::RenderSize { width: width as u32, height: height as u32 };
    if let Err(e) = hybrid_renderer.render(
        &scene,
        &mut gpu.resources,
        &gpu.device,
        &gpu.queue,
        &mut encoder,
        &render_size,
        &view,
        &vello_hybrid::TextureBindings::new(),
    ) {
        crate::loge!("vello_hybrid render failed: {e:?}");
        return PaintOutcome::Skip;
    }
    // Three `Instant`s where there was one (§46). `present` is the largest phase in the
    // frame (7.43ms, 60% of total work at §44.3) and this document has asserted since
    // §39.5 that it is CPU-side strip generation — i.e. that it is all `render`, and
    // therefore that occlusion culling is the lever. Nothing measured that. The three
    // calls below do three unrelated things: ours, the driver's, and the swapchain's.
    let render_done = Instant::now();
    phases.present_render += render_done - present_started;
    let buffer = encoder.finish();
    let finish_done = Instant::now();
    phases.present_finish += finish_done - render_done;
    gpu.queue.submit([buffer]);
    let submit_done = Instant::now();
    phases.present_submit += submit_done - finish_done;
    frame.present();
    phases.present_swap += submit_done.elapsed();
    phases.present += present_started.elapsed();
    PaintOutcome::Painted
}

/// `currentColor` for a node: the same resolved text colour its own glyph runs are
/// painted in. Shared by both paint backends' icon passes so an `Icon` and a `Text` with
/// the same class string can never disagree about what `text-white` means.
fn current_color(n: &zkite_renderer::PositionedNode) -> Option<zkite_renderer::Color> {
    n.style.text.color
}

fn paint_nodes_into_scene(
    scene: &mut vello_hybrid::Scene,
    resources: &mut vello_hybrid::Resources,
    nodes: &[zkite_renderer::PositionedNode],
    base_transform: kurbo::Affine,
    mut images: Option<&mut zkite_imagestore::ImageStore>,
) {
    // **Everything a node draws, drawn together, in tree order** (../PLAN.md §31.1).
    //
    // This used to be three whole-tree passes — every node's box, then every node's
    // icon, then every node's glyphs — each with a comment claiming it reproduced "the
    // paint order the WebView backend gets from ordinary DOM stacking". It does so only
    // for a node against *its own ancestors*. Across sibling subtrees it inverts:
    // **every glyph in the tree painted above every background in the tree**, so an
    // opaque box could never cover an earlier sibling's text. `Stack`'s entire purpose
    // is to overlay children, and it was measurably broken by this (2.77% against real
    // Chromium on a two-child fixture); the error overlay could not hide the
    // "waiting for `zkite dev`…" screen behind it, which is how it was found (§15.3).
    //
    // Per node the sequence is unchanged — shadows (reversed), ring, background,
    // border, icon, glyphs — so §28.3's ordering contract and §18's icon-between-the-
    // two both still hold. What changes is only that the *next* node starts after this
    // one finishes, which is what DOM paint order actually is. One clip push/pop per
    // node now covers all of it rather than one per pass.
    // CSS `opacity` fades a node *together with its subtree*, so it is a layer over a
    // range of this flat list and not a per-node alpha. The arithmetic lives in
    // `zkite_renderer::OpacityGroups` because this loop exists three times in the repo
    // (here, `paint_cpu_fallback`, and `conformance`) against three scene types with
    // no trait between them — which is exactly how `paint.opacity` came to be parsed
    // by `twcore`, stored in every `ComputedStyle`, and read by none of the three for
    // the whole of phase 3 (../PLAN.md §45.1).
    let mut groups = zkite_renderer::OpacityGroups::new();
    for (i, n) in nodes.iter().enumerate() {
        // Before the `continue` below, not after: a layer an ancestor opened must
        // close at its own subtree's end whether or not the node sitting at that
        // index happens to paint anything.
        for _ in 0..groups.close_before(i) {
            scene.pop_layer();
        }
        if let Some(alpha) = groups.open_at(i, n) {
            scene.push_opacity_layer(alpha);
        }
        let (x, y, w, h) = n.rect;
        let deco = zkite_renderer::box_decoration(n.rect, &n.style, current_color(n));
        // A gradient counts as a box even with no `bg-*` colour beside it, which is the
        // overwhelmingly common case: `bg-linear-to-r from-cyan-500 to-blue-500` sets
        // `background-image` and leaves `background-color` transparent. Left out of this
        // condition, a gradient-only node is `continue`d past and paints nothing at all —
        // the exact silent-divergence shape ../PLAN.md §38 exists to remove.
        let gradient = zkite_renderer::background_gradient(n.rect, &n.style);
        let has_box = n.style.paint.background.is_some() || gradient.is_some() || !deco.is_empty();
        // A *focused, empty* `TextField` synthesizes a text child with content `""`
        // (see `zkite_pybridge::build_node`'s `is_the_focused_field` branch — it must
        // exist so `focused_text_node` resolves to a real node for caret placement,
        // even with nothing typed yet), and shaping empty content produces
        // `Some(vec![])` — present, zero runs, not absent. Treated as no text rather
        // than reaching `scene.set_paint` with nothing to actually draw afterward — a
        // reasonable simplification on its own, but *not* a fix for the real bug
        // that motivated looking at this loop in the first place: see ../PLAN.md §5's
        // known-gaps entry ("an unrelated text node goes blank once any TextField
        // can be focused") for the full account — every value read out of `nodes`
        // for the affected node (rect, color, clip, glyph count and position) was
        // confirmed byte-identical whether or not a field was focused, on *both*
        // paint backends, so this guard was ruled out as the cause before being kept
        // for its own, smaller merit.
        let runs = n.text.as_ref().filter(|r| !r.is_empty());
        if !has_box && n.icon.is_none() && n.image.is_none() && runs.is_none() {
            continue;
        }
        let clip = n.clip.map(clip_path);
        if let Some(path) = &clip {
            scene.push_clip_path(path);
        }

        if has_box {
            paint_shadows_and_ring_gpu(scene, &deco);
            if let Some(bg) = n.style.paint.background {
                scene.set_paint(peniko::Color::from_rgba8(bg.r, bg.g, bg.b, bg.a));
                fill_background(scene, x, y, w, h, &n.style.paint.radius);
            }
            // `background-image` over `background-color`: two CSS properties, painted in
            // that order over the same box, so a transparent stop lets the colour beneath
            // show through (../PLAN.md §38.1). The brush is built by `zkite_renderer` so
            // that both paint paths and both host consumers get the same interpolation
            // space — see that module's doc for why the space is not a detail.
            if let Some(g) = gradient {
                scene.set_paint(g);
                fill_background(scene, x, y, w, h, &n.style.paint.radius);
            }
            paint_border_gpu(scene, &deco);
        }

        // A bitmap sits where an icon does — above this node's own background and
        // border, below its own text — which is where CSS puts replaced content.
        //
        // `store.gpu_id` rather than the decoded pixmap: `vello_hybrid`'s wgpu backend
        // only draws `ImageSource::OpaqueId` (see ../images.rs), and `upload_pending`
        // has already run for this frame's nodes. A `src` with no id here is one whose
        // decode failed, so the node paints as the empty box `Image` always drew.
        if let (Some(src), Some(store)) = (&n.image, images.as_deref_mut()) {
            if let Some(id) = store.gpu_id(src) {
                if let Some(decoded) = store.get(src) {
                    if let Some(paint) = image_paint(n, &decoded) {
                        let rounded = !n.style.paint.radius.is_zero();
                        if rounded {
                            // Rounding an image is `rounded-full` on an avatar, which
                            // is common enough to be worth a clip layer. Only pushed
                            // when there is a radius, so the ordinary case pays
                            // nothing.
                            scene.push_clip_path(&rounded_box_path(n.rect, &n.style.paint.radius));
                        }
                        scene.set_paint(vello_common::paint::Image {
                            image: vello_common::paint::ImageSource::opaque_id_with_transparency_hint(
                                id,
                                decoded.pixmap.may_have_transparency(),
                            ),
                            sampler: peniko::ImageSampler::default(),
                        });
                        scene.set_paint_transform(paint.transform);
                        let (fx, fy, fw, fh) = paint.fill;
                        scene.fill_rect(&kurbo::Rect::new(
                            fx as f64,
                            fy as f64,
                            (fx + fw) as f64,
                            (fy + fh) as f64,
                        ));
                        // `set_paint_transform` is sticky like `set_transform` is —
                        // leaving it in place would offset every gradient or image
                        // painted after this node.
                        scene.reset_paint_transform();
                        if rounded {
                            scene.pop_clip_path();
                        }
                    }
                }
            }
        }

        // Icons sit between this node's background and this node's text: an icon must
        // cover its own node's background and must not cover its own node's glyphs.
        if let Some(icon) = &n.icon {
            // Composed onto the base (density) transform rather than replacing it, then
            // restored — `set_transform` is absolute, so leaving the icon's transform in
            // place would scale every node painted after it.
            scene.set_transform(base_transform * icon.transform(n.rect));
            for shape in &icon.shapes {
                if let Some(c) = shape.fill.resolve(current_color(n)) {
                    scene.set_paint(peniko::Color::from_rgba8(c.r, c.g, c.b, c.a));
                    scene.fill_path(&shape.path);
                }
                if let Some(stroke) = shape.stroke {
                    if let Some(c) = stroke.paint.resolve(current_color(n)) {
                        scene.set_paint(peniko::Color::from_rgba8(c.r, c.g, c.b, c.a));
                        scene.set_stroke(stroke.to_kurbo());
                        scene.stroke_path(&shape.path);
                    }
                }
            }
            scene.set_transform(base_transform);
        }

        if let Some(runs) = runs {
            let color = n
                .style
                .text
                .color
                .map(|c| peniko::Color::from_rgba8(c.r, c.g, c.b, c.a))
                .unwrap_or(peniko::color::palette::css::BLACK);
            scene.set_paint(color);
            for run in runs {
                let glyphs: Vec<glifo::Glyph> = run
                    .glyphs
                    .iter()
                    .map(|g: &PositionedGlyph| glifo::Glyph { id: g.id, x: g.x + x, y: g.y + y })
                    .collect();
                scene
                    .glyph_run(resources, &run.font)
                    .font_size(run.font_size)
                    .hint(true)
                    .fill_glyphs(glyphs.into_iter());
            }
        }

        if clip.is_some() {
            scene.pop_clip_path();
        }
    }
    for _ in 0..groups.close_all() {
        scene.pop_layer();
    }
}

/// Where an `Image` node's decoded bitmap is drawn, and how its pixels map onto the
/// canvas — the one piece of `Image` geometry both paint backends compute, so they
/// cannot disagree about it (../PLAN.md §35.2).
///
/// Shared for the same reason `zkite_renderer::box_decoration` and `icons.rs` are: the
/// two paint loops in this file are one paint order expressed against two scene APIs,
/// and a divergence between them shows up only on a device that fell back to CPU —
/// which, per ../PLAN.md §18.7, has never been run.
struct ImagePaint {
    /// The rect actually filled, absolute dp. Already the intersection of the
    /// `object-fit` rect with the node's own box, which is what makes `Cover` crop
    /// and `Contain` letterbox without either backend special-casing a mode.
    fill: (f32, f32, f32, f32),
    /// Image pixel space -> absolute dp. Handed to `set_paint_transform`, which vello
    /// applies *after* the scene's density transform, so this is in dp and not in
    /// device pixels.
    transform: kurbo::Affine,
}

fn image_paint(
    n: &zkite_renderer::PositionedNode,
    decoded: &zkite_imagestore::Decoded,
) -> Option<ImagePaint> {
    let (bx, by, bw, bh) = n.rect;
    let (iw, ih) = (decoded.width as f32, decoded.height as f32);
    let (dx, dy, dw, dh) =
        zkite_renderer::object_fit_rect(n.style.paint.object_fit, (bw, bh), (iw, ih));
    if dw <= 0.0 || dh <= 0.0 {
        return None;
    }
    // The destination in absolute coordinates, then clipped to the node's own box.
    // `Cover` and `None` overflow it by construction (see `object_fit_rect`), and CSS
    // clips a replaced element's content to its content box.
    let (ax, ay) = (bx + dx, by + dy);
    let (fx0, fy0) = (ax.max(bx), ay.max(by));
    let (fx1, fy1) = ((ax + dw).min(bx + bw), (ay + dh).min(by + bh));
    if fx1 <= fx0 || fy1 <= fy0 {
        return None;
    }
    let fill = (fx0, fy0, fx1 - fx0, fy1 - fy0);
    // Maps (0,0)..(iw,ih) onto the *unclipped* destination — deliberately not onto
    // `fill`. The clip changes which part of the image is visible; it must not change
    // the scale, or a `Cover` image would be squashed back into the box it was
    // supposed to overflow.
    let transform = kurbo::Affine::translate((ax as f64, ay as f64))
        * kurbo::Affine::scale_non_uniform((dw / iw) as f64, (dh / ih) as f64);
    Some(ImagePaint { fill, transform })
}

/// The `BezPath` for a `PositionedNode.clip` rect — a plain rectangle, no radius
/// (clip regions come from a scroll *container's* box, which a `ScrollView` in
/// practice rarely rounds; if it does, the clip is a very slightly looser rectangle
/// around the true rounded shape, a minor cosmetic gap, not a functional one — see
/// ../PLAN.md §5). Shared by both `push_clip_path` call sites (background and text
/// passes) since a node's clip rect never differs between the two.
fn clip_path(rect: (f32, f32, f32, f32)) -> kurbo::BezPath {
    use kurbo::{Rect, Shape};
    Rect::new(rect.0 as f64, rect.1 as f64, (rect.0 + rect.2) as f64, (rect.1 + rect.3) as f64).to_path(0.1)
}

// ---------------------------------------------------------------- §28 decoration
//
// The shadow/ring and border halves of a node's box, one pair of functions per paint
// backend, deliberately kept adjacent in this file.
//
// `zkite_renderer::box_decoration` owns every decision — the corner maths, the
// blur-to-std_dev conversion, the uniform-vs-per-side split — so these four functions
// are pure call sequences. They still exist rather than being inlined at the two call
// sites because the sequence carries a contract that is easy to break silently: **the
// shadow list is painted in reverse**, since `box-shadow`'s first layer is the topmost
// one. Getting that backwards is invisible on a single-layer shadow and wrong on every
// multi-layer one, which is all of Tailwind v4's sm/md/lg/xl.

fn paint_shadows_and_ring_gpu(scene: &mut vello_hybrid::Scene, deco: &zkite_renderer::BoxDecoration) {
    for s in deco.shadows.iter().rev() {
        scene.set_paint(peniko::Color::from_rgba8(s.color.r, s.color.g, s.color.b, s.color.a));
        scene.fill_blurred_rounded_rect(&s.rect, s.radius as f32, s.std_dev as f32, false);
    }
    if let Some(ring) = &deco.ring {
        scene.set_paint(peniko::Color::from_rgba8(ring.color.r, ring.color.g, ring.color.b, ring.color.a));
        scene.set_stroke(kurbo::Stroke::new(ring.width));
        scene.stroke_path(&ring.path);
    }
}

fn paint_border_gpu(scene: &mut vello_hybrid::Scene, deco: &zkite_renderer::BoxDecoration) {
    match &deco.border {
        Some(zkite_renderer::Border::Uniform(o)) => {
            scene.set_paint(peniko::Color::from_rgba8(o.color.r, o.color.g, o.color.b, o.color.a));
            scene.set_stroke(kurbo::Stroke::new(o.width));
            scene.stroke_path(&o.path);
        }
        Some(zkite_renderer::Border::PerSide(sides)) => {
            for side in sides {
                scene.push_clip_path(&side.wedge);
                scene.set_paint(peniko::Color::from_rgba8(side.color.r, side.color.g, side.color.b, side.color.a));
                scene.fill_path(&side.band);
                scene.pop_clip_path();
            }
        }
        None => {}
    }
}

fn paint_shadows_and_ring_cpu(ctx: &mut vello_cpu::RenderContext, deco: &zkite_renderer::BoxDecoration) {
    use vello_cpu::color::AlphaColor;
    for s in deco.shadows.iter().rev() {
        ctx.set_paint(AlphaColor::<vello_cpu::color::Srgb>::from_rgba8(s.color.r, s.color.g, s.color.b, s.color.a));
        ctx.fill_blurred_rounded_rect(&s.rect, s.radius as f32, s.std_dev as f32, false);
    }
    if let Some(ring) = &deco.ring {
        ctx.set_paint(AlphaColor::<vello_cpu::color::Srgb>::from_rgba8(
            ring.color.r,
            ring.color.g,
            ring.color.b,
            ring.color.a,
        ));
        ctx.set_stroke(vello_cpu::kurbo::Stroke::new(ring.width));
        ctx.stroke_path(&ring.path);
    }
}

fn paint_border_cpu(ctx: &mut vello_cpu::RenderContext, deco: &zkite_renderer::BoxDecoration) {
    use vello_cpu::color::AlphaColor;
    match &deco.border {
        Some(zkite_renderer::Border::Uniform(o)) => {
            ctx.set_paint(AlphaColor::<vello_cpu::color::Srgb>::from_rgba8(o.color.r, o.color.g, o.color.b, o.color.a));
            ctx.set_stroke(vello_cpu::kurbo::Stroke::new(o.width));
            ctx.stroke_path(&o.path);
        }
        Some(zkite_renderer::Border::PerSide(sides)) => {
            for side in sides {
                ctx.push_clip_path(&side.wedge);
                ctx.set_paint(AlphaColor::<vello_cpu::color::Srgb>::from_rgba8(
                    side.color.r,
                    side.color.g,
                    side.color.b,
                    side.color.a,
                ));
                ctx.fill_path(&side.band);
                ctx.pop_clip_path();
            }
        }
        None => {}
    }
}

fn fill_background(scene: &mut vello_hybrid::Scene, x: f32, y: f32, w: f32, h: f32, radius: &zkite_renderer::Corners) {
    if radius.is_zero() {
        scene.fill_rect(&kurbo::Rect::new(x as f64, y as f64, (x + w) as f64, (y + h) as f64));
    } else {
        scene.fill_path(&rounded_box_path((x, y, w, h), radius));
    }
}

/// A node's own box as a path, honouring its corner radii — the shape a background
/// fill and an `Image`'s clip both need.
///
/// Callers check `Corners::is_zero()` first when they have a cheaper rectangle path
/// available; this always builds the rounded form, so it is correct (if not minimal)
/// for a zero radius too.
fn rounded_box_path(rect: (f32, f32, f32, f32), radius: &zkite_renderer::Corners) -> kurbo::BezPath {
    use kurbo::{RoundedRect, Shape};
    let (x, y, w, h) = rect;
    RoundedRect::new(
        x as f64,
        y as f64,
        (x + w) as f64,
        (y + h) as f64,
        (radius.top_left as f64, radius.top_right as f64, radius.bottom_right as f64, radius.bottom_left as f64),
    )
    .to_path(0.1)
}

// ---------------------------------------------------------------------- CPU fallback

/// `clip_path`'s counterpart for the CPU path — same shape, `vello_cpu`'s own
/// re-exported `kurbo` (matching this function's existing imports) rather than the
/// top-level crate; both resolve to the same type (same `kurbo` version), so this
/// exists only to keep each paint backend's `use`s self-contained, not because the
/// types actually differ.
fn cpu_clip_path(rect: (f32, f32, f32, f32)) -> vello_cpu::kurbo::BezPath {
    use vello_cpu::kurbo::{Rect, Shape};
    Rect::new(rect.0 as f64, rect.1 as f64, (rect.0 + rect.2) as f64, (rect.1 + rect.3) as f64).to_path(0.1)
}

/// `rounded_box_path`'s counterpart for the CPU path, for the same
/// keep-each-backend's-`use`s-self-contained reason `cpu_clip_path` exists.
fn cpu_rounded_box_path(rect: (f32, f32, f32, f32), radius: &zkite_renderer::Corners) -> vello_cpu::kurbo::BezPath {
    use vello_cpu::kurbo::{RoundedRect, Shape};
    let (x, y, w, h) = rect;
    RoundedRect::new(
        x as f64,
        y as f64,
        (x + w) as f64,
        (y + h) as f64,
        (radius.top_left as f64, radius.top_right as f64, radius.bottom_right as f64, radius.bottom_left as f64),
    )
    .to_path(0.1)
}

fn paint_cpu_fallback(
    window: &NativeWindow,
    width: i32,
    height: i32,
    scale: f32,
    nodes: &[zkite_renderer::PositionedNode],
    caret: Option<(f32, f32, f32, f32)>,
    images: &mut Option<Box<zkite_imagestore::ImageStore>>,
) -> bool {
    use vello_cpu::color::AlphaColor;
    use vello_cpu::kurbo::{Affine, Rect};
    use vello_cpu::{Pixmap, RenderContext, Resources};

    let mut ctx = RenderContext::new(width as u16, height as u16);
    let mut resources = Resources::new();
    // Same dp -> device-px scene transform as the GPU path, for the same reasons —
    // see `paint_gpu`'s own comment on why this is a transform rather than
    // pre-multiplied rects. `vello_cpu::RenderContext::glyph_run` feeds
    // `scene_transform()` to the identical `glifo` code path, so the two backends
    // stay pixel-comparable under density exactly as ../PLAN.md §9 requires them to
    // be without it.
    let base_transform = Affine::scale(scale as f64);
    ctx.set_transform(base_transform);

    // Everything a node draws, drawn together, in tree order — same structure and same
    // reasoning as the GPU path above (../PLAN.md §31.1). These two loops must stay in
    // lockstep: they are the same paint order expressed against two scene APIs, and a
    // divergence between them would be visible only on a device that fell back to CPU.
    // Same opacity grouping as the GPU path, sharing the same bookkeeping so the two
    // cannot drift — see that loop's comment and `zkite_renderer::OpacityGroups`.
    let mut groups = zkite_renderer::OpacityGroups::new();
    for (i, n) in nodes.iter().enumerate() {
        for _ in 0..groups.close_before(i) {
            ctx.pop_layer();
        }
        if let Some(alpha) = groups.open_at(i, n) {
            ctx.push_opacity_layer(alpha);
        }
        let (x, y, w, h) = n.rect;
        let deco = zkite_renderer::box_decoration(n.rect, &n.style, current_color(n));
        // See the GPU path's note: a gradient with no `bg-*` colour is still a box.
        let gradient = zkite_renderer::background_gradient(n.rect, &n.style);
        let has_box = n.style.paint.background.is_some() || gradient.is_some() || !deco.is_empty();
        // Same empty-runs guard as `paint_nodes_into_scene`'s GPU path, kept for the
        // same reason (see that function's comment: a real simplification, ruled
        // out as the cause of ../PLAN.md §5's known rendering gap, which reproduces
        // identically on *this* backend too — not GPU-specific).
        let runs = n.text.as_ref().filter(|r| !r.is_empty());
        if !has_box && n.icon.is_none() && n.image.is_none() && runs.is_none() {
            continue;
        }
        let clip = n.clip.map(cpu_clip_path);
        if let Some(path) = &clip {
            ctx.push_clip_path(path);
        }

        if has_box {
            paint_shadows_and_ring_cpu(&mut ctx, &deco);
            let fill_box = |ctx: &mut vello_cpu::RenderContext| {
                let rect = Rect::new(x as f64, y as f64, (x + w) as f64, (y + h) as f64);
                let radius = &n.style.paint.radius;
                if radius.is_zero() {
                    ctx.fill_rect(&rect);
                } else {
                    ctx.fill_path(&cpu_rounded_box_path(n.rect, radius));
                }
            };
            if let Some(bg) = n.style.paint.background {
                ctx.set_paint(AlphaColor::<vello_cpu::color::Srgb>::from_rgba8(bg.r, bg.g, bg.b, bg.a));
                fill_box(&mut ctx);
            }
            // The CPU counterpart of the GPU path's gradient fill, same position in the
            // sequence and the same brush — these two loops must stay in lockstep.
            if let Some(g) = gradient {
                ctx.set_paint(g);
                fill_box(&mut ctx);
            }
            paint_border_cpu(&mut ctx, &deco);
        }

        // The CPU counterpart of the GPU image pass — same position in the sequence,
        // same geometry (`image_paint` is shared), and the *simpler* of the two
        // because `vello_cpu` samples a `Pixmap` directly and needs no atlas upload.
        if let (Some(src), Some(store)) = (&n.image, images.as_deref_mut()) {
            if let Some(decoded) = store.get(src) {
                if let Some(paint) = image_paint(n, &decoded) {
                    let rounded = !n.style.paint.radius.is_zero();
                    if rounded {
                        ctx.push_clip_path(&cpu_rounded_box_path(n.rect, &n.style.paint.radius));
                    }
                    ctx.set_paint(vello_cpu::Image {
                        image: vello_cpu::ImageSource::Pixmap(decoded.pixmap.clone()),
                        sampler: vello_cpu::peniko::ImageSampler::default(),
                    });
                    ctx.set_paint_transform(paint.transform);
                    let (fx, fy, fw, fh) = paint.fill;
                    ctx.fill_rect(&Rect::new(fx as f64, fy as f64, (fx + fw) as f64, (fy + fh) as f64));
                    ctx.reset_paint_transform();
                    if rounded {
                        ctx.pop_clip_path();
                    }
                }
            }
        }

        if let Some(icon) = &n.icon {
            ctx.set_transform(base_transform * icon.transform(n.rect));
            for shape in &icon.shapes {
                if let Some(c) = shape.fill.resolve(current_color(n)) {
                    ctx.set_paint(AlphaColor::<vello_cpu::color::Srgb>::from_rgba8(c.r, c.g, c.b, c.a));
                    ctx.fill_path(&shape.path);
                }
                if let Some(stroke) = shape.stroke {
                    if let Some(c) = stroke.paint.resolve(current_color(n)) {
                        ctx.set_paint(AlphaColor::<vello_cpu::color::Srgb>::from_rgba8(c.r, c.g, c.b, c.a));
                        ctx.set_stroke(stroke.to_kurbo());
                        ctx.stroke_path(&shape.path);
                    }
                }
            }
            ctx.set_transform(base_transform);
        }

        if let Some(runs) = runs {
            let color = n.style.text.color.map(|c| vello_cpu::peniko::Color::from_rgba8(c.r, c.g, c.b, c.a)).unwrap_or(vello_cpu::color::palette::css::BLACK);
            ctx.set_paint(AlphaColor::<vello_cpu::color::Srgb>::from(color));
            for run in runs {
                let glyphs: Vec<vello_cpu::Glyph> =
                    run.glyphs.iter().map(|g: &PositionedGlyph| vello_cpu::Glyph { id: g.id, x: g.x + x, y: g.y + y }).collect();
                ctx.glyph_run(&mut resources, &run.font).font_size(run.font_size).hint(true).fill_glyphs(glyphs.into_iter());
            }
        }

        if clip.is_some() {
            ctx.pop_clip_path();
        }
    }
    // Closed before the caret, which is deliberately *not* inside anyone's opacity
    // group: it is painted from `caret`, has no `PositionedNode`, and a caret that
    // faded with its field would be a caret nobody could see in a half-faded form.
    for _ in 0..groups.close_all() {
        ctx.pop_layer();
    }
    // Same "painted last, no PositionedNode of its own" reasoning as `paint_gpu`'s
    // caret handling — see `caret_rect`'s doc comment.
    if let Some((x, y, w, h)) = caret {
        ctx.set_paint(AlphaColor::<vello_cpu::color::Srgb>::from_rgba8(255, 255, 255, 255));
        ctx.fill_rect(&Rect::new(x as f64, y as f64, (x + w) as f64, (y + h) as f64));
    }
    ctx.flush();
    let mut target = Pixmap::new(width as u16, height as u16);
    ctx.render(&mut target, &mut resources);
    // `take_unpremultiplied()` returns typed `color::Rgba8` pixels (a `#[repr(C)]`
    // 4x-u8 struct), not raw bytes — flatten explicitly rather than reach for
    // `bytemuck` for one call site.
    let rgba: Vec<u8> = target
        .take_unpremultiplied()
        .into_iter()
        .flat_map(|p| [p.r, p.g, p.b, p.a])
        .collect();

    if let Err(e) = window.set_buffers_geometry(width, height, Some(HardwareBufferFormat::R8G8B8A8_UNORM)) {
        crate::loge!("set_buffers_geometry failed: {e}");
        return false;
    }
    let mut guard = match window.lock(None) {
        Ok(g) => g,
        Err(e) => {
            crate::loge!("ANativeWindow_lock failed: {e}");
            return false;
        }
    };
    let stride_px = guard.stride();
    let buf_height = guard.height();
    // If the locked buffer's real geometry disagrees with what was just requested,
    // every row this function copies lands at the wrong offset and the frame is
    // silently garbled/skewed rather than merely wrong-coloured. Cheap to check,
    // impossible to diagnose from a screenshot alone, and previously unchecked
    // (`guard.width()` was never even read) — see ../PLAN.md §7.14 for the
    // neighbouring geometry bug that made this worth asserting.
    if guard.width() != width as usize || buf_height != height as usize {
        crate::loge!(
            "ANativeWindow buffer geometry mismatch — requested {width}x{height}, locked buffer is {}x{} (stride {stride_px}); this frame will be skewed",
            guard.width(),
            buf_height
        );
    }
    let Some(dst) = guard.bytes() else {
        crate::loge!("ANativeWindow buffer has no CPU-accessible bytes");
        return false;
    };
    let row_bytes = width as usize * 4;
    let stride_bytes = stride_px * 4;
    for row in 0..(height as usize).min(buf_height) {
        let src_off = row * row_bytes;
        let dst_off = row * stride_bytes;
        if src_off + row_bytes > rgba.len() || dst_off + row_bytes > dst.len() {
            break;
        }
        for (d, s) in dst[dst_off..dst_off + row_bytes].iter_mut().zip(&rgba[src_off..src_off + row_bytes]) {
            d.write(*s);
        }
    }
    true
}
