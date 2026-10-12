//! The animation wire format — ../PLAN.md §47, Queue F layer 1.
//!
//! One `anim` attribute on a node, carrying a list of per-channel declarations. This
//! module turns that JSON into `zkite_anim::Track`s and nothing else: it never samples
//! a clock, never holds state between frames, and never decides *when* an animation
//! runs. Those are the driver's (`../gpu_harness/src/render_thread.rs`) and they need
//! a device; this needs only a string, which is why it lives here.
//!
//! # Declared, not commanded
//!
//! §45.3 principle 3: animations are *declared in the tree*. The tree is the only
//! channel both backends already share and it is already the frame protocol, so an
//! animation is a property of a node the same way a class string is — not a message
//! sent once that the renderer must then remember was sent.
//!
//! The consequence that shapes everything here: **the same declaration arrives on
//! every frame.** A drag rebuilds the tree ~60 times a second, so a 300ms entrance
//! spec is seen ~18 times before it finishes. That is what [`Trigger`] is for, and it
//! is why the driver's default policy is `Interrupt::Ignore` rather than `Restart` —
//! see [`Trigger::interrupt`].
//!
//! # Why not `transition-*` classes
//!
//! The owner's decision (§45.3 principle 2, §45.7): explicit channels and triggers,
//! not Tailwind's `transition-*`. Borrowing CSS's spelling would buy CSS's semantics
//! — its interruption behaviour, its property-change triggers and its `transitionend`
//! timing — which is a new silent divergence bought for a feature meant to remove
//! one. §32.1 rejected HTML's `<label>` on exactly this argument, with a measurement.
//!
//! # What is deliberately strict
//!
//! Unknown keys, unknown channels and non-finite numbers are **errors**, not silent
//! drops. An unknown Tailwind class is dropped because Tailwind has thousands and a
//! typo is cheap; an author who writes `rotate=...` has asked for one specific thing,
//! and §15.1's entire "silent divergence" category is what happens when the answer to
//! that is silence. Errors surface through the same unknown-class report path a
//! misspelled utility already uses.

use std::time::Duration;

use zkite_anim::{Direction, Easing, Interrupt, Repeat, Spring, Track};
use zkite_renderer::Channel;
use serde::Deserialize;

/// What causes a declaration to actually start.
///
/// All three are *declared* states, not events the renderer detects. That is forced
/// by the architecture rather than chosen: the tree is authoritative, so a node that
/// has left it is simply gone and there is nothing left to animate out. §45.2's
/// answer, priced there: Python keeps the departing node in the tree with an `exit`
/// spec on it, the driver runs the animation and dispatches **one** completion, and
/// Python drops the node then. One Python call per animation, not per frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Trigger {
    /// Run once, the first time this declaration is seen for this node and channel.
    /// The default, and the one an entrance uses.
    Enter,
    /// Run whenever `to` differs from the target this key was last started with —
    /// "follow this value". The driver keeps the last target per key; nothing else
    /// here does.
    Change,
    /// Run once, like [`Enter`](Trigger::Enter), but the completion means *the node
    /// may now be removed*. Mechanically identical to `Enter` inside the driver; the
    /// difference is entirely in what `zkite_py` does with the completion, which is
    /// why the dispatched payload names the trigger.
    Exit,
}

impl Trigger {
    /// The interruption policy this trigger starts its track with.
    ///
    /// `Ignore` for `Enter`/`Exit` is what makes a declared animation **idempotent**,
    /// which matters more here than in most frameworks: without it the same entrance
    /// spec restarts on every rebuilt frame and the animation never finishes.
    /// `zkite_anim`'s own `ignore_leaves_a_running_animation_alone_but_not_a_finished_
    /// one` is the test that pins it, with `Restart` as its negative control.
    ///
    /// `Retarget` for `Change` carries position *and* velocity across for a spring
    /// and position only for an eased track — so a value being followed never jumps,
    /// and a sheet the user can grab mid-flight should be a spring.
    pub fn interrupt(self) -> Interrupt {
        match self {
            Trigger::Enter | Trigger::Exit => Interrupt::Ignore,
            Trigger::Change => Interrupt::Retarget,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Trigger::Enter => "enter",
            Trigger::Change => "change",
            Trigger::Exit => "exit",
        }
    }

    fn parse(s: &str) -> Option<Trigger> {
        match s {
            "enter" => Some(Trigger::Enter),
            "change" => Some(Trigger::Change),
            "exit" => Some(Trigger::Exit),
            _ => None,
        }
    }
}

/// One channel of one node, declared. The validated form of a wire entry.
#[derive(Debug, Clone, PartialEq)]
pub struct AnimSpec {
    pub channel: Channel,
    /// `None` means "start from whatever this node's class string resolves to",
    /// resolved against the live renderer by [`track`](AnimSpec::track). Keeping it
    /// optional rather than making `zkite_py` compute it is deliberate: only this side
    /// knows the resolved value, since it depends on `twcore`, on inheritance, and —
    /// for `1em`-relative things — on ancestry that Python cannot see (§18.4).
    pub from: Option<f32>,
    pub to: f32,
    pub curve: SpecCurve,
    pub delay: Duration,
    pub repeat: Repeat,
    pub direction: Direction,
    pub trigger: Trigger,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SpecCurve {
    Eased { duration: Duration, easing: Easing },
    Spring(Spring),
}

impl AnimSpec {
    /// Build the runnable track, filling in `from` from the node's resolved style
    /// when the author did not state one.
    pub fn track(&self, class_value: f32) -> Track {
        let from = self.from.unwrap_or(class_value);
        match self.curve {
            SpecCurve::Eased { duration, easing } => Track::eased(from, self.to, duration, easing)
                .with_repeat(self.repeat)
                .with_direction(self.direction)
                .with_delay(self.delay),
            // `Repeat`/`Direction` are eased-only in `zkite_anim` (a spring's duration
            // comes from its own settling), and `Track::sample` ignores them for a
            // spring rather than pretending. Not passed here, so the two agree.
            SpecCurve::Spring(spring) => Track::spring(from, self.to, spring).with_delay(self.delay),
        }
    }
}

// ----------------------------------------------------------------- the wire shape

/// Keys are short because this crosses the bridge inside every rebuilt tree, and §43
/// measured what tree JSON costs: a rebuild's `json.dumps` was 44% of a rebuild frame
/// once its C accelerator was actually reachable. `deny_unknown_fields` turns a typo
/// into a message naming the key instead of an animation that silently does nothing.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawSpec {
    ch: String,
    to: f32,
    #[serde(default)]
    from: Option<f32>,
    #[serde(default)]
    ms: Option<f32>,
    #[serde(default)]
    ease: Option<RawEase>,
    #[serde(default)]
    spring: Option<[f32; 2]>,
    #[serde(default)]
    delay: Option<f32>,
    #[serde(default)]
    repeat: Option<RawRepeat>,
    #[serde(default)]
    dir: Option<String>,
    #[serde(default)]
    on: Option<String>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum RawEase {
    Named(String),
    /// `cubic-bezier(x1, y1, x2, y2)`, so an author is never stuck with four presets.
    Bezier([f32; 4]),
}

#[derive(Deserialize)]
#[serde(untagged)]
enum RawRepeat {
    Times(u32),
    Named(String),
}

/// Whether the driver should start `spec` now, given the target it last started for
/// this same node and channel — ../PLAN.md §47.
///
/// # Why the gate is on the declaration and not on `Interrupt`
///
/// `zkite_anim::Interrupt::Ignore` declines to restart an animation that is **still
/// running**, and documents that a *finished* entry does not block a new run — "a
/// second open of the same sheet animates again". That is right for a spec that is
/// sent, withdrawn, and sent again.
///
/// It is wrong for a *declaration*, which is what §45.3 principle 3 makes these. The
/// entrance spec is still on the node the frame after it completes, so `Ignore` alone
/// would start it over, and over, forever. **This is the check that stops an entrance
/// animation looping**, and `Ignore` stays underneath it as a second line of defence
/// rather than as the mechanism.
///
/// `previous` is `None` when nothing has been started for this key — either it is new,
/// or its declaration was withdrawn and the driver evicted it, which is what makes a
/// reopened sheet animate again.
///
/// Bit equality rather than `==` on the target: a target that genuinely has not moved
/// must compare equal to itself even when it is a value like `0.1` that no arithmetic
/// reproduces exactly, and this runs on every frame. (`-0.0` and `0.0` differ in bits
/// and are the same position, so that pair retargets a spring to where it already is —
/// harmless, and the alternative is a NaN-tolerant comparison nobody can read.)
pub fn should_start(previous: Option<f32>, spec: &AnimSpec) -> bool {
    match previous {
        None => true,
        Some(last_to) => spec.trigger == Trigger::Change && last_to.to_bits() != spec.to.to_bits(),
    }
}

/// Parse a node's `anim` attribute.
///
/// `None` (the attribute is absent) and an empty list both mean "nothing declared",
/// which is every node in every app that does not animate — so the hot path is one
/// `Option` check.
pub fn parse(value: Option<&serde_json::Value>) -> Result<Vec<AnimSpec>, String> {
    let Some(value) = value else { return Ok(Vec::new()) };
    let raw: Vec<RawSpec> =
        serde_json::from_value(value.clone()).map_err(|e| format!("bad `anim` attribute: {e}"))?;
    raw.into_iter().map(validate).collect()
}

fn validate(r: RawSpec) -> Result<AnimSpec, String> {
    let channel = Channel::parse(&r.ch).ok_or_else(|| {
        let known: Vec<&str> = Channel::all().iter().map(|c| c.name()).collect();
        // `rotate`/`scale` get their own sentence because they are the ones an author
        // will actually reach for: they parse in `twcore`, they work on the WebView
        // backend, and they are painted by nothing on native (§45.7). Silence here
        // would be the §15.1 divergence reproduced inside the fix for it.
        let hint = if matches!(r.ch.as_str(), "rotate" | "scale" | "scale_x" | "scale_y") {
            format!(" — `{}` is not painted by the native renderer yet (PLAN.md §45.7)", r.ch)
        } else {
            String::new()
        };
        format!("unknown animation channel `{}`; known: {}{hint}", r.ch, known.join(", "))
    })?;

    // Every number reaching a `Track` is checked here rather than trusted. A NaN
    // `translate_y` propagates straight into `positioned()`'s arithmetic and from
    // there into `hit_test`, producing a node that is painted nowhere and tappable
    // nowhere, with no error anywhere — and it is reachable from ordinary Python
    // (`float("nan")`, or a `0/0` that numpy hands back without raising).
    let finite = |v: f32, what: &str| -> Result<f32, String> {
        if v.is_finite() {
            Ok(v)
        } else {
            Err(format!("`{what}` on channel `{}` is {v}, which is not a finite number", channel.name()))
        }
    };

    let to = finite(r.to, "to")?;
    let from = r.from.map(|v| finite(v, "from")).transpose()?;
    let delay_ms = finite(r.delay.unwrap_or(0.0), "delay")?;
    if delay_ms < 0.0 {
        return Err(format!("`delay` on channel `{}` is negative ({delay_ms}ms)", channel.name()));
    }

    let curve = match (r.ms, r.spring) {
        (Some(ms), None) => {
            let ms = finite(ms, "ms")?;
            if ms < 0.0 {
                return Err(format!("`ms` on channel `{}` is negative ({ms})", channel.name()));
            }
            SpecCurve::Eased { duration: millis(ms), easing: parse_ease(r.ease)? }
        }
        (None, Some([stiffness, damping])) => {
            let stiffness = finite(stiffness, "spring stiffness")?;
            let damping = finite(damping, "spring damping")?;
            if stiffness <= 0.0 {
                return Err(format!("spring stiffness must be > 0, got {stiffness}"));
            }
            if damping < 0.0 {
                return Err(format!("spring damping must be >= 0, got {damping}"));
            }
            if r.ease.is_some() {
                return Err("a spring has no easing — `ease` and `spring` are exclusive".into());
            }
            SpecCurve::Spring(Spring { stiffness, damping, mass: 1.0 })
        }
        // Both or neither. Stated as an error rather than defaulted because the two
        // are genuinely different animations and guessing which one the author meant
        // is how a framework acquires a surprising default nobody can find later.
        (Some(_), Some(_)) => {
            return Err(format!("channel `{}` declares both `ms` and `spring`; pick one", channel.name()))
        }
        (None, None) => {
            return Err(format!("channel `{}` declares neither `ms` nor `spring`", channel.name()))
        }
    };

    let repeat = match r.repeat {
        None => Repeat::Once,
        Some(RawRepeat::Times(n)) => Repeat::Times(n),
        Some(RawRepeat::Named(s)) if s == "forever" => Repeat::Forever,
        Some(RawRepeat::Named(s)) => return Err(format!("unknown `repeat` value `{s}`; use a count or \"forever\"")),
    };

    let direction = match r.dir.as_deref() {
        None | Some("normal") => Direction::Normal,
        Some("reverse") => Direction::Reverse,
        Some("alternate") => Direction::Alternate,
        Some("alternate_reverse") => Direction::AlternateReverse,
        Some(other) => {
            return Err(format!(
                "unknown `dir` value `{other}`; use normal, reverse, alternate or alternate_reverse"
            ))
        }
    };

    let trigger = match r.on.as_deref() {
        None => Trigger::Enter,
        Some(s) => Trigger::parse(s)
            .ok_or_else(|| format!("unknown animation trigger `{s}`; use enter, change or exit"))?,
    };

    // A `Forever` track never finishes, so it never reports a completion and never
    // lets the render thread go idle (`zkite_anim`'s `a_forever_track_never_finishes_
    // and_never_lets_the_loop_idle`). That is correct for a spinner and catastrophic
    // for an exit, which would keep a departing node alive and repainting for the
    // life of the process while Python waited for a callback that cannot arrive.
    if matches!(repeat, Repeat::Forever) && trigger == Trigger::Exit {
        return Err("an `exit` animation cannot repeat forever — its completion is what removes the node".into());
    }

    Ok(AnimSpec { channel, from, to, curve, delay: millis(delay_ms), repeat, direction, trigger })
}

/// Milliseconds to a `Duration`, rounded to the nearest nanosecond.
///
/// **Not `Duration::from_secs_f32(ms / 1000.0)`**, which is what this was first
/// written as: dividing by 1000 in `f32` and multiplying back up by 1e9 makes a plain
/// `200` come out as **200.000003ms**. Harmless on screen and not harmless in a test,
/// which then has to assert approximate equality on a number the author typed exactly
/// — and it gets worse, not better, for the long delays a staggered list uses.
fn millis(ms: f32) -> Duration {
    Duration::from_nanos((ms as f64 * 1_000_000.0).round() as u64)
}

fn parse_ease(raw: Option<RawEase>) -> Result<Easing, String> {
    Ok(match raw {
        // CSS's own default for a bare `transition`, and `zkite_anim` carries its
        // control points to the digit so the future JS port has one published answer
        // to check against rather than a second description to diverge from.
        None => Easing::EASE,
        Some(RawEase::Named(s)) => match s.as_str() {
            "linear" => Easing::Linear,
            "ease" => Easing::EASE,
            "in" => Easing::EASE_IN,
            "out" => Easing::EASE_OUT,
            "in_out" => Easing::EASE_IN_OUT,
            other => {
                return Err(format!(
                    "unknown easing `{other}`; use linear, ease, in, out, in_out, or [x1,y1,x2,y2]"
                ))
            }
        },
        Some(RawEase::Bezier(p)) => {
            for (i, v) in p.iter().enumerate() {
                if !v.is_finite() {
                    return Err(format!("easing control point {i} is {v}, which is not a finite number"));
                }
            }
            // Only `x` is constrained. CSS requires the x coordinates in [0,1] for the
            // curve to be a function of time at all, and deliberately does *not*
            // constrain y — a control point outside [0,1] on y is how "back out" and
            // "anticipate" overshoot, which `Easing`'s own doc comment says it will
            // not flatten.
            if !(0.0..=1.0).contains(&p[0]) || !(0.0..=1.0).contains(&p[2]) {
                return Err(format!(
                    "easing x control points must be in [0,1] (got {} and {}); y may overshoot",
                    p[0], p[2]
                ));
            }
            Easing::CubicBezier { x1: p[0], y1: p[1], x2: p[2], y2: p[3] }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(json: &str) -> Result<Vec<AnimSpec>, String> {
        let v: serde_json::Value = serde_json::from_str(json).unwrap();
        parse(Some(&v))
    }

    fn one(json: &str) -> AnimSpec {
        spec(json).unwrap().pop().unwrap()
    }

    #[test]
    fn an_absent_or_empty_attribute_is_not_an_error() {
        assert_eq!(parse(None).unwrap(), Vec::new());
        assert_eq!(spec("[]").unwrap(), Vec::new());
    }

    #[test]
    fn a_minimal_eased_declaration_parses_with_css_defaults() {
        let s = one(r#"[{"ch":"opacity","to":1,"from":0,"ms":200}]"#);
        assert_eq!(s.channel, Channel::Opacity);
        assert_eq!((s.from, s.to), (Some(0.0), 1.0));
        assert_eq!(s.trigger, Trigger::Enter, "enter is the default trigger");
        assert_eq!(s.repeat, Repeat::Once);
        assert_eq!(s.direction, Direction::Normal);
        assert_eq!(s.delay, Duration::ZERO);
        // The unstated easing is CSS's own `ease`, not `Linear` — `Easing::default()`
        // is `Linear`, so a `#[serde(default)]` on the field would have silently
        // produced the wrong curve for every animation in the framework.
        assert_eq!(
            s.curve,
            SpecCurve::Eased { duration: Duration::from_millis(200), easing: Easing::EASE }
        );
    }

    #[test]
    fn a_spring_declaration_parses_and_carries_unit_mass() {
        let s = one(r#"[{"ch":"translate_y","to":0,"from":320,"spring":[180,26],"on":"change"}]"#);
        assert_eq!(s.channel, Channel::TranslateY);
        assert_eq!(s.trigger, Trigger::Change);
        assert_eq!(s.curve, SpecCurve::Spring(Spring { stiffness: 180.0, damping: 26.0, mass: 1.0 }));
    }

    #[test]
    fn every_named_easing_and_a_raw_bezier_are_reachable() {
        let named = [
            ("linear", Easing::Linear),
            ("ease", Easing::EASE),
            ("in", Easing::EASE_IN),
            ("out", Easing::EASE_OUT),
            ("in_out", Easing::EASE_IN_OUT),
        ];
        for (name, expected) in named {
            let s = one(&format!(r#"[{{"ch":"opacity","to":1,"ms":100,"ease":"{name}"}}]"#));
            assert_eq!(s.curve, SpecCurve::Eased { duration: Duration::from_millis(100), easing: expected });
        }
        let s = one(r#"[{"ch":"opacity","to":1,"ms":100,"ease":[0.2,-0.5,0.8,1.5]}]"#);
        // y outside [0,1] is *allowed* — that is how an overshoot curve is written,
        // and `Easing` documents that it will not flatten one.
        assert_eq!(
            s.curve,
            SpecCurve::Eased {
                duration: Duration::from_millis(100),
                easing: Easing::CubicBezier { x1: 0.2, y1: -0.5, x2: 0.8, y2: 1.5 }
            }
        );
    }

    #[test]
    fn several_channels_on_one_node_parse_independently() {
        let specs = spec(
            r#"[{"ch":"opacity","to":1,"from":0,"ms":200},
                {"ch":"translate_y","to":0,"from":16,"spring":[180,26]}]"#,
        )
        .unwrap();
        assert_eq!(specs.len(), 2);
        assert_eq!(specs[0].channel, Channel::Opacity);
        assert_eq!(specs[1].channel, Channel::TranslateY);
    }

    /// `from` is optional and its absence is not zero — it means "wherever the class
    /// string puts this node". Resolving it to 0.0 here would make every unstated
    /// fade start from fully transparent and every unstated slide start at the origin.
    #[test]
    fn an_unstated_from_defers_to_the_nodes_resolved_class_value() {
        let s = one(r#"[{"ch":"opacity","to":0,"ms":200}]"#);
        assert_eq!(s.from, None);
        assert_eq!(s.track(0.75).from, 0.75, "the class value must be used");
        assert_eq!(s.track(0.75).to, 0.0);
        // …and a stated `from` wins over it.
        let stated = one(r#"[{"ch":"opacity","to":0,"from":0.25,"ms":200}]"#);
        assert_eq!(stated.track(0.75).from, 0.25);
    }

    #[test]
    fn a_track_carries_delay_repeat_and_direction_through() {
        let s = one(
            r#"[{"ch":"opacity","to":1,"from":0,"ms":100,"delay":300,"repeat":3,"dir":"alternate"}]"#,
        );
        let t = s.track(0.0);
        assert_eq!(t.delay, Duration::from_millis(300));
        assert_eq!(t.repeat, Repeat::Times(3));
        assert_eq!(t.direction, Direction::Alternate);
        let forever = one(r#"[{"ch":"opacity","to":1,"from":0,"ms":100,"repeat":"forever"}]"#);
        assert_eq!(forever.track(0.0).repeat, Repeat::Forever);
    }

    /// A spring's `Repeat`/`Direction` are ignored by `Track::sample`, so this side
    /// must not pretend to set them — otherwise a `repeat` on a spring reads back as
    /// honoured and does nothing.
    #[test]
    fn a_spring_track_does_not_claim_a_repeat_it_cannot_honour() {
        let s = one(r#"[{"ch":"translate_y","to":0,"from":10,"spring":[180,26],"repeat":5}]"#);
        assert_eq!(s.repeat, Repeat::Times(5), "the declaration is preserved as written");
        assert_eq!(s.track(0.0).repeat, Repeat::Once, "…and not handed to a track that ignores it");
    }

    #[test]
    fn each_trigger_maps_to_the_interruption_policy_that_makes_it_idempotent() {
        // The whole reason `Ignore` is the default: the tree is rebuilt on every
        // frame of a scroll, so a 300ms entrance spec arrives ~18 times before it
        // finishes, and `Restart` would mean it never does.
        assert_eq!(Trigger::Enter.interrupt(), Interrupt::Ignore);
        assert_eq!(Trigger::Exit.interrupt(), Interrupt::Ignore);
        // `Change` follows a moving target, so it must not jump.
        assert_eq!(Trigger::Change.interrupt(), Interrupt::Retarget);
        for t in [Trigger::Enter, Trigger::Change, Trigger::Exit] {
            assert_eq!(Trigger::parse(t.name()), Some(t));
        }
    }

    // ---- the start gate ---------------------------------------------------------

    /// **The test that stops an entrance animation looping forever.**
    ///
    /// The declaration is still on the node after it completes — that is what
    /// "declared, not commanded" means — so a gate that asked `zkite_anim` whether
    /// anything is running would restart it on the very next frame. This is also the
    /// single most likely way for this driver to be wrong in a way that looks like a
    /// rendering bug rather than a logic one.
    #[test]
    fn a_finished_entrance_is_not_restarted_by_its_own_declaration_arriving_again() {
        let s = one(r#"[{"ch":"opacity","to":1,"from":0,"ms":200}]"#);
        // Frame 1: nothing started yet.
        assert!(should_start(None, &s));
        // Every frame after, for the life of the node — including long after the
        // animation has finished and the entry is holding its final value.
        for _ in 0..120 {
            assert!(!should_start(Some(1.0), &s), "an enter restarted itself");
        }
        // Same for an exit, which would otherwise never let the node be removed.
        let e = one(r#"[{"ch":"opacity","to":0,"ms":200,"on":"exit"}]"#);
        assert!(should_start(None, &e));
        assert!(!should_start(Some(0.0), &e));
    }

    /// `change` is the one trigger that re-runs, and only when the target actually
    /// moved. Restarting it on every frame would make a followed value never arrive;
    /// never restarting it would make `change` identical to `enter`.
    #[test]
    fn change_restarts_only_when_its_target_moves() {
        let to_zero = one(r#"[{"ch":"translate_x","to":0,"spring":[180,26],"on":"change"}]"#);
        let to_hundred = one(r#"[{"ch":"translate_x","to":100,"spring":[180,26],"on":"change"}]"#);

        assert!(should_start(None, &to_zero), "the first arrival always starts");
        // The same declaration, sixty times a second, changes nothing.
        assert!(!should_start(Some(0.0), &to_zero));
        // The target moves: retarget.
        assert!(should_start(Some(0.0), &to_hundred));
        // …and having moved, it settles again.
        assert!(!should_start(Some(100.0), &to_hundred));
        // A value no arithmetic reproduces exactly must still compare equal to
        // itself, or a `change` bound to a computed float retargets every frame and
        // never converges.
        let tenth = one(r#"[{"ch":"opacity","to":0.1,"ms":100,"on":"change"}]"#);
        assert!(!should_start(Some(0.1_f32), &tenth));
    }

    // ---- what must be rejected, and why each one is here ------------------------

    fn err(json: &str) -> String {
        spec(json).expect_err(&format!("accepted {json}"))
    }

    /// A channel that parses in `twcore`, works on the WebView backend, and is painted
    /// by nothing on native (§45.7). Accepting it would produce an animation that
    /// runs, completes and fires its callback without moving a pixel — §15.1's silent
    /// divergence, reproduced inside the feature built to remove one.
    #[test]
    fn rotate_and_scale_are_rejected_by_name_with_a_reason() {
        for ch in ["rotate", "scale", "scale_x", "scale_y"] {
            let e = err(&format!(r#"[{{"ch":"{ch}","to":1,"ms":100}}]"#));
            assert!(e.contains("not painted by the native renderer"), "{ch}: {e}");
            assert!(e.contains("§45.7"), "{ch}: {e}");
        }
        // An outright typo gets the list of what is valid rather than the hint.
        let e = err(r#"[{"ch":"opactiy","to":1,"ms":100}]"#);
        assert!(e.contains("opacity, translate_x, translate_y"), "{e}");
    }

    /// The one that would be genuinely dangerous. A NaN reaches `positioned()`'s
    /// arithmetic and from there `hit_test`, giving a node painted nowhere and
    /// tappable nowhere with no error anywhere — and `float("nan")` is ordinary
    /// Python.
    #[test]
    fn non_finite_numbers_are_rejected_on_every_numeric_field() {
        for bad in ["1e39", "-1e39"] {
            for field in ["to", "from", "delay", "ms"] {
                let json = format!(r#"[{{"ch":"opacity","to":1,"ms":100,"{field}":{bad}}}]"#);
                let e = err(&json);
                assert!(e.contains("not a finite number"), "{field}={bad}: {e}");
            }
        }
        assert!(err(r#"[{"ch":"opacity","to":1,"spring":[1e39,26]}]"#).contains("not a finite"));
        assert!(err(r#"[{"ch":"opacity","to":1,"ms":100,"ease":[0.2,1e39,0.8,1]}]"#).contains("not a finite"));
    }

    #[test]
    fn a_declaration_must_pick_exactly_one_curve() {
        assert!(err(r#"[{"ch":"opacity","to":1}]"#).contains("neither `ms` nor `spring`"));
        assert!(err(r#"[{"ch":"opacity","to":1,"ms":100,"spring":[180,26]}]"#).contains("pick one"));
        assert!(err(r#"[{"ch":"opacity","to":1,"spring":[180,26],"ease":"out"}]"#).contains("exclusive"));
    }

    #[test]
    fn nonsense_spring_constants_and_negative_times_are_rejected() {
        assert!(err(r#"[{"ch":"opacity","to":1,"spring":[0,26]}]"#).contains("stiffness must be > 0"));
        assert!(err(r#"[{"ch":"opacity","to":1,"spring":[-5,26]}]"#).contains("stiffness must be > 0"));
        assert!(err(r#"[{"ch":"opacity","to":1,"spring":[180,-1]}]"#).contains("damping must be >= 0"));
        assert!(err(r#"[{"ch":"opacity","to":1,"ms":-5}]"#).contains("negative"));
        assert!(err(r#"[{"ch":"opacity","to":1,"ms":100,"delay":-5}]"#).contains("negative"));
    }

    /// An x control point outside [0,1] makes the curve not a function of time, so
    /// CSS forbids it. y outside [0,1] is legal and is how an overshoot is written —
    /// rejecting both would quietly remove every "back out" curve.
    #[test]
    fn a_bezier_x_outside_the_unit_interval_is_rejected_and_y_is_not() {
        assert!(err(r#"[{"ch":"opacity","to":1,"ms":100,"ease":[1.2,0,0.8,1]}]"#).contains("must be in [0,1]"));
        assert!(err(r#"[{"ch":"opacity","to":1,"ms":100,"ease":[0.2,0,-0.1,1]}]"#).contains("must be in [0,1]"));
        assert!(spec(r#"[{"ch":"opacity","to":1,"ms":100,"ease":[0,-2,1,3]}]"#).is_ok(), "y must be free");
    }

    /// A forever-repeating exit keeps a departing node alive and repainting for the
    /// life of the process, while Python waits for a completion that cannot arrive.
    /// Cheap to write by accident and impossible to diagnose from the symptom.
    #[test]
    fn an_exit_that_never_finishes_is_rejected() {
        let e = err(r#"[{"ch":"opacity","to":0,"ms":200,"repeat":"forever","on":"exit"}]"#);
        assert!(e.contains("cannot repeat forever"), "{e}");
        // The control: the identical spec on `enter` is fine — a spinner is exactly
        // that, and the restriction is about the trigger, not about `forever`.
        assert!(spec(r#"[{"ch":"opacity","to":0,"ms":200,"repeat":"forever","on":"enter"}]"#).is_ok());
    }

    #[test]
    fn unknown_keys_triggers_easings_repeats_and_directions_all_name_themselves() {
        assert!(err(r#"[{"ch":"opacity","to":1,"ms":100,"durationn":5}]"#).contains("durationn"));
        assert!(err(r#"[{"ch":"opacity","to":1,"ms":100,"on":"hover"}]"#).contains("hover"));
        assert!(err(r#"[{"ch":"opacity","to":1,"ms":100,"ease":"bouncy"}]"#).contains("bouncy"));
        assert!(err(r#"[{"ch":"opacity","to":1,"ms":100,"repeat":"twice"}]"#).contains("twice"));
        assert!(err(r#"[{"ch":"opacity","to":1,"ms":100,"dir":"backwards"}]"#).contains("backwards"));
        // A missing required key names the key too, rather than defaulting to 0.
        assert!(err(r#"[{"ch":"opacity","ms":100}]"#).contains("to"));
    }

    /// The attribute is a *list*. A bare object is the mistake an author writing one
    /// channel will make, and it has to say so rather than being silently ignored.
    #[test]
    fn a_bare_object_instead_of_a_list_is_an_error() {
        let e = err(r#"{"ch":"opacity","to":1,"ms":100}"#);
        assert!(e.contains("bad `anim` attribute"), "{e}");
    }
}
