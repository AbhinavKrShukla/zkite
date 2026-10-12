# Spike 3 — `twcore`: Tailwind class strings → `taffy::Style` + paint

**Criterion (plan1.md §10):** *`twcore` prototype: parse 40 common utilities into a
`taffy::Style` + paint struct.*

**Verdict: PASS — 60 utility families, 52 tests, 19 ns per cached resolve.**

```rust
use twcore::{Resolver, NodeState};

let mut r = Resolver::new();
let s = r.resolve("flex-1 px-4 py-2 rounded-xl bg-indigo-500", NodeState::default());

s.layout        // a real taffy::Style — flex_grow 1.0, padding 16/8, …
s.paint         // background indigo-500, radius 12dp
s.text          // size, weight, leading, tracking, family
s.transform     // rotate / scale / translate
```

```bash
cargo test              # 52 tests
cargo test --release -- --nocapture resolving_is_fast   # the timing number
```

No CSS is produced at any point. No cascade, no selector matching, no TS runtime — §4's
premise, made concrete.

## What is covered

60 families, listed exhaustively in `tests/utilities.rs::FAMILIES` (the test asserts the
list is ≥ 40 and that every entry actually resolves):

- **Layout** — `flex` `grid` `block` `hidden`, `flex-row/col(-reverse)`, `flex-wrap`,
  `flex-1/auto/initial/none`, `grow` `shrink`, `basis-*`, `items-*`, `justify-*`,
  `self-*`, `content-*`, `gap[-x|-y]-*`
- **Box model** — `p/px/py/pt/pr/pb/pl-*`, `m*-*` including `auto` and negatives,
  `w/h/size-*`, `min-w/min-h/max-w/max-h-*`, fractions (`w-1/2`), `box-border/content`,
  `aspect-square/video`
- **Position** — `relative` `absolute` `fixed` `sticky`, `inset[-x|-y]-*`,
  `top/right/bottom/left-*`, negatives
- **Overflow** — `overflow[-x|-y]-{auto,hidden,scroll,clip,visible}`
- **Grid** — `grid-cols-N`, `grid-rows-N`, `col-span-N`, `row-span-N`
- **Paint** — `bg-*`, `border[-x|-y|-t|-r|-b|-l][-width|-colour]`,
  `rounded[-t|-b|-l|-r|-tl|…]-*`, `opacity-*`, `shadow-*`, `ring-*`, alpha modifiers
  (`bg-black/50`)
- **Text** — `text-{xs..9xl}`, `text-{colour}`, `font-{thin..black}`,
  `font-{sans,serif,mono}`, `leading-*`, `tracking-*`, `text-{left,center,right,justify}`,
  `italic`, `underline`, `line-through`, `tabular-nums`
- **Transform** — `rotate-*`, `scale[-x|-y]-*`, `translate-x/y-*`, negatives
- **Arbitrary values** — `w-[137px]`, `bg-[#0f172a]`, `text-[13px]`, `p-[2rem]`,
  `h-[50%]`, `-mt-[10px]`
- **Variants** — `hover:` `active:` `focus:` `disabled:` `dark:` `rtl:` `landscape:`
  `sm:` `md:` `lg:` `xl:` `2xl:`, and combinations (`dark:hover:bg-slate-800`)

Deliberately out of scope, per §4: arbitrary selectors, the cascade, `@apply` against real
stylesheets, container queries, `:has()`. *Tailwind-compatible, not CSS-compatible.*

## Three things the tests prove beyond the count

### 1. The output really is a `taffy::Style`

`tests/layout.rs` builds trees out of class strings, runs taffy, and asserts geometry.
The counter app from plan1.md §8, on a 400×800 screen:

```rust
let column = app.node("flex flex-col flex-1 items-center justify-center gap-6 bg-slate-950",
                      &[text, button]);
```

```
column                 (0, 0, 400, 800)     flex-1 fills the screen
text at y = 324        (800 − (80 + 24 + 48)) / 2 — justify-center
button at y = 428      324 + 80 + 24 — gap-6 really is 24dp
both centred in x                          items-center
```

Also asserted: padding shrinks the content box, `border-2` participates in layout,
`justify-between` pushes a row apart, `absolute inset-0` fills its parent,
`grid-cols-3 gap-4` splits 332dp into 100/100/100, `w-1/2` resolves against the parent,
and `md:flex-row` changes the *computed positions* at 900dp but not at 400dp.

There is no conversion step between `twcore` and taffy. That is the point.

### 2. Variants are a bitmask, exactly as §4 specifies

> Implemented as a small bitset of active states per node, so variant resolution is a
> mask-and-pick, not a re-parse.

A class string compiles **once** into `Vec<(VariantSet, Decl)>` sorted by emission rank.
Resolving for a `NodeState` tests bits and applies matching declarations; no string is
touched. Two caches sit in front:

| Cache | Key | Paid |
|---|---|---|
| compile | class string | once per distinct class string in the app |
| style | (class string, node state) | once per distinct *rendered* combination |

Asserted:

- `class_string_is_parsed_exactly_once` — 10,000 resolves, `stats.compiles == 1`
- `each_distinct_state_builds_at_most_one_style` — 4,000 resolves across 4 states, 4 builds
- `viewport_widths_in_the_same_bucket_share_a_cache_entry` — 340 widths below `sm`, 1 build
- `resolving_is_fast_enough_to_do_per_frame` — **19 ns** per cached resolve (release)

**Emission order was the subtle part.** Tailwind's precedence comes from CSS source order,
not from the order of classes in the attribute, so these must be identical when pressed:

```
"bg-indigo-500 active:bg-indigo-600"
"active:bg-indigo-600 bg-indigo-500"
```

And because media blocks are emitted *after* the base layer, `md:` beats `hover:`:

```
"bg-red-500 hover:bg-green-500 md:bg-blue-500"   at 900dp + hovered  ->  blue
```

`VariantSet::rank()` encodes this as `breakpoint * 16 + state_rank`, and a stable sort
preserves author order within a rank. Both cases are tested.

### 3. The palette is generated, not typed

`src/palette.rs` is generated from `tailwindcss@3.4.17`'s own `src/public/colors.js` — 22
hues × 11 shades = **242 colours**, with `palette.json` kept beside it as the provenance
record. `tests/palette.rs` asserts the table is sorted for binary search, spot-checks a
dozen values against the published palette, and confirms **every one of the 242 entries is
reachable through a `bg-<hue>-<shade>` class string**.

Getting `indigo-500` wrong by one hex digit is the kind of thing nobody notices for six
months.

## Theming

`Theme` is the `@theme`-equivalent from §4 — the thing a `theme.py` or
`[tool.zkite.theme]` block would drive. Overriding is a data change, not a code change:

```rust
let mut theme = Theme::default();
theme.spacing_unit = 8.0;                                   // p-4 becomes 32dp
theme.custom_colors.push(("brand".into(), Color::rgb(0xff5500)));
theme.screens.md = 500.0;
let mut r = Resolver::with_theme(theme);
r.resolve("p-4 bg-brand", NodeState::default());            // both resolve
```

Tested in `tests/variants_and_cache.rs`.

## Unknown classes are reported, not swallowed

```rust
r.unknown("flex bg-indigo-500 not-a-real-class hovr:hidden bg-notacolor-500")
// ["not-a-real-class", "hovr:hidden", "bg-notacolor-500"]
```

A misspelled variant drops the whole class rather than applying the utility
unconditionally — silently making `hovr:hidden` mean `hidden` would be much worse than
dropping it. This is the hook `zkite doctor` needs to tell a user their class was a typo
rather than rendering nothing.

## Structure

```
src/
├── lib.rs       Resolver, the two caches, StyleEngine trait, class-string splitting
├── parse.rs     the only file that does string work: utility -> Vec<Decl>
├── decl.rs      Decl: the normalised form. Applying one is a match + a field assignment.
├── style.rs     ComputedStyle { layout: taffy::Style, paint, text, transform }
├── theme.rs     token tables — spacing, colours, sizes, radii, shadows, breakpoints
├── variant.rs   StateFlags bitset, NodeState, VariantSet, emission rank
└── palette.rs   @generated from tailwindcss@3.4.17
```

`StyleEngine` is §4's pluggable-engine protocol in Rust form; the `tokens` and `inline`
engines would implement the same trait.

## Known simplifications

Phase-0 scope, all deliberate and all documented in the code:

- ~~**Shadows keep one layer.**~~ **Fixed in phase_3/PLAN.md §28.1.** This said the
  single layer was "enough to lay out and paint, not enough for pixel-exact parity",
  which stopped being true the moment anything painted it: every one of Tailwind v4's
  `shadow-sm`/`md`/`lg`/`xl` is *two* layers with different offsets and spreads, so one
  layer is a visibly different shadow rather than a slightly cheaper one. `BoxShadow`
  now holds a fixed-capacity layer list and **refuses** an over-capacity one instead of
  truncating. The same pass found that this table's *values* were still Tailwind v3's
  — v4 renamed the whole scale — see §28.1.
- **No viewport units.** `w-screen` maps to `100%` because the root node *is* the screen,
  so it is exact in practice but wrong in principle if that stops being true.
- **`sticky` maps to `absolute`** — taffy has no sticky positioning. `static` maps to
  `relative`, which is taffy's default and equivalent for our purposes.
- **`translate-x-1/2` is unsupported.** Transforms carry absolute dp, so percentage
  translation would need the node's resolved size, which is a layout-time value.
- **`ring` is a width + colour**, not the full inset/offset model. `ring-offset-*` and
  inset rings are still unparsed as of phase_3/PLAN.md §28; the width and colour
  *defaults* were corrected to v4's there (1px and `currentColor`, from 3px and
  blue-500).
- **Only `taffy`.** Parley (text shaping) and Vello (rasterisation) are phase 3;
  `TextStyle` is populated but nothing consumes it yet.
