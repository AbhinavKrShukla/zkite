//! Variants as a bitset, per plan1.md §4: "a small bitset of active states per node, so
//! variant resolution is a mask-and-pick, not a re-parse."
//!
//! A class string is parsed once into `Vec<(VariantSet, Decl)>`. Resolving for a given
//! `NodeState` is then a bitmask test per declaration — no string touched.

use crate::theme::Theme;

/// One bit per boolean-ish node condition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct StateFlags(pub u16);

impl StateFlags {
    pub const HOVER: StateFlags = StateFlags(1 << 0);
    pub const ACTIVE: StateFlags = StateFlags(1 << 1);
    pub const FOCUS: StateFlags = StateFlags(1 << 2);
    pub const DISABLED: StateFlags = StateFlags(1 << 3);
    pub const DARK: StateFlags = StateFlags(1 << 4);
    pub const RTL: StateFlags = StateFlags(1 << 5);
    pub const LANDSCAPE: StateFlags = StateFlags(1 << 6);

    pub const fn empty() -> StateFlags {
        StateFlags(0)
    }
    pub const fn union(self, other: StateFlags) -> StateFlags {
        StateFlags(self.0 | other.0)
    }
    pub const fn contains(self, other: StateFlags) -> bool {
        self.0 & other.0 == other.0
    }
}

impl core::ops::BitOr for StateFlags {
    type Output = StateFlags;
    fn bitor(self, rhs: StateFlags) -> StateFlags {
        self.union(rhs)
    }
}

/// The live per-node conditions the resolver is asked about.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NodeState {
    pub flags: StateFlags,
    /// Viewport width in dp, for `sm:` / `md:` / `lg:` / `xl:` / `2xl:`.
    pub viewport_width: f32,
}

impl Default for NodeState {
    fn default() -> Self {
        // A mid-size phone in portrait: no breakpoint above `sm` active.
        NodeState { flags: StateFlags::empty(), viewport_width: 400.0 }
    }
}

impl NodeState {
    pub fn with(mut self, f: StateFlags) -> Self {
        self.flags = self.flags | f;
        self
    }

    /// Index of the largest active breakpoint (0 = none, 1 = sm .. 5 = 2xl).
    /// Also the cache-key discriminator: two viewport widths in the same bucket cannot
    /// produce different styles, so they must share a cache entry.
    pub fn breakpoint_bucket(&self, theme: &Theme) -> u8 {
        let s = &theme.screens;
        let w = self.viewport_width;
        if w >= s.xxl {
            5
        } else if w >= s.xl {
            4
        } else if w >= s.lg {
            3
        } else if w >= s.md {
            2
        } else if w >= s.sm {
            1
        } else {
            0
        }
    }

    pub fn cache_key(&self, theme: &Theme) -> u32 {
        (self.flags.0 as u32) | ((self.breakpoint_bucket(theme) as u32) << 16)
    }
}

/// The compiled precondition of one declaration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct VariantSet {
    /// All of these flags must be set on the node.
    pub required: StateFlags,
    /// 0 = unconditional, 1 = sm .. 5 = 2xl.
    pub breakpoint: u8,
}

impl VariantSet {
    pub fn matches(&self, state: &NodeState, theme: &Theme) -> bool {
        state.flags.contains(self.required) && state.breakpoint_bucket(theme) >= self.breakpoint
    }

    /// Emission order, standing in for CSS source order. Larger wins.
    ///
    /// Media-query blocks are emitted after the base layer, so `md:x` beats `hover:x`;
    /// within a block, Tailwind's default variant order applies.
    pub fn rank(&self) -> u16 {
        let f = self.required;
        let state_rank = if f.contains(StateFlags::DISABLED) {
            6
        } else if f.contains(StateFlags::ACTIVE) {
            5
        } else if f.contains(StateFlags::HOVER) {
            4
        } else if f.contains(StateFlags::FOCUS) {
            3
        } else if f.contains(StateFlags::RTL) || f.contains(StateFlags::LANDSCAPE) {
            2
        } else if f.contains(StateFlags::DARK) {
            1
        } else {
            0
        };
        (self.breakpoint as u16) * 16 + state_rank
    }
}

/// Parse one `foo:` prefix. Returns None for an unknown variant, which the caller
/// treats as "drop this whole class" rather than "ignore the prefix" — silently
/// applying `hovr:hidden` unconditionally would be much worse than dropping it.
pub fn parse_variant(name: &str, theme: &Theme) -> Option<VariantSet> {
    let flag = match name {
        "hover" => StateFlags::HOVER,
        "active" => StateFlags::ACTIVE,
        "focus" => StateFlags::FOCUS,
        "disabled" => StateFlags::DISABLED,
        "dark" => StateFlags::DARK,
        "rtl" => StateFlags::RTL,
        "landscape" => StateFlags::LANDSCAPE,
        _ => {
            let width = theme.breakpoint(name)?;
            let s = &theme.screens;
            let idx = if width == s.sm {
                1
            } else if width == s.md {
                2
            } else if width == s.lg {
                3
            } else if width == s.xl {
                4
            } else {
                5
            };
            return Some(VariantSet { required: StateFlags::empty(), breakpoint: idx });
        }
    };
    Some(VariantSet { required: flag, breakpoint: 0 })
}
