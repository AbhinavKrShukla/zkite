//! # twcore — Phase 0 spike 3
//!
//! Resolves a Tailwind class string directly to a `ComputedStyle` (a real
//! `taffy::Style` plus paint / text / transform). No CSS, no cascade, no selector
//! matching — see plan1.md §4.
//!
//! ```
//! use twcore::{Resolver, NodeState};
//! let mut r = Resolver::new();
//! let s = r.resolve("flex-1 px-4 py-2 rounded-xl bg-indigo-500", NodeState::default());
//! assert_eq!(s.paint.background.unwrap().to_rgba_u32(), 0x615fffff); // Tailwind v4 indigo-500
//! ```
//!
//! Two caches sit in front of the parser:
//!
//! 1. **compile cache** — class string -> `Vec<(VariantSet, Decl)>`, sorted by emission
//!    rank. Paid once per distinct class string in the whole app.
//! 2. **style cache** — (class string, node state) -> `ComputedStyle`. Paid once per
//!    distinct *rendered* combination.
//!
//! A resolve that hits either cache does no string work at all.

pub mod decl;
pub mod palette;
pub mod parse;
pub mod style;
pub mod theme;
pub mod variant;

use std::hash::{Hash, Hasher};
use std::sync::Arc;

use rustc_hash::{FxHashMap, FxHasher};

pub use decl::Decl;
pub use style::{
    BorderColors, BoxShadow, Color, ComputedStyle, Corners, Decoration, ExtColor, FontFamily,
    Gradient, GradientColorSpace, GradientHue, GradientLine, GradientSpec, GradientStop,
    LineHeight, ObjectFit, Paint, PaintColor, ShadowLayer, StopSlot, TextAlignment,
    TextStyle, Transform,
};
pub use theme::{Screens, Theme};
pub use variant::{NodeState, StateFlags, VariantSet};

/// A class string parsed into declarations, ordered so that a plain left-to-right
/// application produces Tailwind's precedence.
#[derive(Debug, Clone, Default)]
pub struct Compiled {
    pub decls: Vec<(VariantSet, Decl)>,
    /// Classes we did not recognise. Surfaced rather than swallowed.
    pub unknown: Vec<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Stats {
    pub compiles: u64,
    pub compile_cache_hits: u64,
    pub style_builds: u64,
    pub style_cache_hits: u64,
}

/// The `StyleEngine` protocol from plan1.md §4, in Rust form. `tokens` and `inline`
/// engines would implement the same trait.
pub trait StyleEngine {
    fn resolve(&mut self, spec: &str, state: NodeState) -> Arc<ComputedStyle>;
}

pub struct Resolver {
    theme: Theme,
    // Keyed by hash, but the source string is stored alongside and checked on every hit —
    // a bare u64 key with no verification would silently return the wrong style on the
    // (astronomically unlikely but not impossible) hash collision. Cheap to check, and a
    // wrong-render-with-no-error is exactly the failure mode not worth risking here.
    compile_cache: FxHashMap<u64, (Box<str>, Arc<Compiled>)>,
    style_cache: FxHashMap<(u64, u32), (Box<str>, Arc<ComputedStyle>)>,
    stats: Stats,
}

impl Default for Resolver {
    fn default() -> Self {
        Resolver::new()
    }
}

fn hash_str(s: &str) -> u64 {
    let mut h = FxHasher::default();
    s.hash(&mut h);
    h.finish()
}

impl Resolver {
    pub fn new() -> Resolver {
        Resolver::with_theme(Theme::default())
    }

    pub fn with_theme(theme: Theme) -> Resolver {
        Resolver {
            theme,
            compile_cache: FxHashMap::default(),
            style_cache: FxHashMap::default(),
            stats: Stats::default(),
        }
    }

    pub fn theme(&self) -> &Theme {
        &self.theme
    }

    pub fn stats(&self) -> Stats {
        self.stats
    }

    /// Parse (or fetch from cache) the declarations for a class string.
    pub fn compile(&mut self, classes: &str) -> Arc<Compiled> {
        let key = hash_str(classes);
        if let Some((s, c)) = self.compile_cache.get(&key) {
            if s.as_ref() == classes {
                self.stats.compile_cache_hits += 1;
                return c.clone();
            }
        }
        self.stats.compiles += 1;
        let compiled = Arc::new(compile_uncached(classes, &self.theme));
        self.compile_cache.insert(key, (classes.into(), compiled.clone()));
        compiled
    }

    /// Full resolve: class string + node state -> computed style.
    pub fn resolve(&mut self, classes: &str, state: NodeState) -> Arc<ComputedStyle> {
        let key = (hash_str(classes), state.cache_key(&self.theme));
        if let Some((s, out)) = self.style_cache.get(&key) {
            if s.as_ref() == classes {
                self.stats.style_cache_hits += 1;
                return out.clone();
            }
        }
        let compiled = self.compile(classes);
        self.stats.style_builds += 1;
        let mut cs = ComputedStyle::default();
        for (variants, decl) in &compiled.decls {
            if variants.matches(&state, &self.theme) {
                decl.apply(&mut cs);
            }
        }
        let out = Arc::new(cs);
        self.style_cache.insert(key, (classes.into(), out.clone()));
        out
    }

    /// Classes in `classes` that no utility matched.
    pub fn unknown(&mut self, classes: &str) -> Vec<String> {
        self.compile(classes).unknown.clone()
    }

    pub fn clear_caches(&mut self) {
        self.compile_cache.clear();
        self.style_cache.clear();
    }
}

impl StyleEngine for Resolver {
    fn resolve(&mut self, spec: &str, state: NodeState) -> Arc<ComputedStyle> {
        Resolver::resolve(self, spec, state)
    }
}

fn compile_uncached(classes: &str, theme: &Theme) -> Compiled {
    let mut out = Compiled::default();
    for class in classes.split_whitespace() {
        // Split `md:hover:bg-red-500` into variants + utility. Bracketed arbitrary
        // values can contain `:`, so only split on colons outside brackets.
        let mut variants = VariantSet::default();
        let mut rest = class;
        let mut bad_variant = false;
        while let Some(idx) = split_point(rest) {
            let (name, tail) = rest.split_at(idx);
            match variant::parse_variant(name, theme) {
                Some(v) => {
                    variants.required = variants.required | v.required;
                    variants.breakpoint = variants.breakpoint.max(v.breakpoint);
                }
                None => {
                    bad_variant = true;
                    break;
                }
            }
            rest = &tail[1..];
        }
        if bad_variant {
            out.unknown.push(class.to_string());
            continue;
        }
        match parse::parse_utility(rest, theme) {
            Some(decls) => {
                for d in decls {
                    out.decls.push((variants, d));
                }
            }
            None => out.unknown.push(class.to_string()),
        }
    }
    // Stable sort: equal-rank declarations keep author order, higher ranks win.
    out.decls.sort_by_key(|(v, _)| v.rank());
    out
}

/// Index of the next `:` that is not inside `[...]`, or None.
fn split_point(s: &str) -> Option<usize> {
    let mut depth = 0i32;
    for (i, b) in s.bytes().enumerate() {
        match b {
            b'[' => depth += 1,
            b']' => depth -= 1,
            b':' if depth == 0 => return Some(i),
            _ => {}
        }
    }
    None
}
