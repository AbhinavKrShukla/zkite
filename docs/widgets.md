# ZKite widgets — what works right now

A write-code-against-this reference for the **18 widgets that exist today**, with the
per-backend caveats stated inline rather than in a footnote, because **native is the
default backend** and several things that work on WebView silently do nothing on it.

Accurate as of 2026-08-09 (`phase_3/PLAN.md` §33). Every "does not work" below is
a real, documented gap with a section reference, not a guess.

> **Read the caveat column before styling anything.** The dangerous class of gap here
> is not "errors" — it is "renders, nothing errors, the effect is just absent."

---

## The two rules

**1. The class string is always the first positional argument. Always, on every
widget, no exceptions.**

```python
Text("text-7xl font-bold text-white", "42")        # class first, content second
Button("px-6 py-3 bg-indigo-500", "Tap me", on_tap=...)
Column("flex-1 gap-4", child, child)
```

plan1.md §8's one code example writes it the other way round; that example is wrong and
`phase_1/PLAN.md` §3 explains why this order was chosen instead.

**2. A `view()` function is the entry point, and stores are injected by type
annotation.** No `App()`, no `run()`, no `if __name__ == "__main__"`.

```python
from zkite import Column, Text, Button, store

@store
class Counter:
    n: int = 0

def view(c: Counter):
    return Column("flex-1 items-center justify-center gap-6 bg-slate-950",
        Text("text-7xl font-bold text-white tabular-nums", f"{c.n}"),
        Button("px-6 py-3 rounded-2xl bg-indigo-500 text-white",
               "Increment", on_tap=lambda: c.update(n=c.n + 1)),
    )
```

---

## Containers

| Widget | Implied classes | Notes |
|---|---|---|
| `Column(cls, *children)` | `flex flex-col` | |
| `Row(cls, *children)` | `flex flex-row` | |
| `Grid(cls, *children)` | `grid` | use `grid-cols-*`, `gap-*` |
| `Stack(cls, *children)` | `grid` | children overlaid in one cell; later children paint **over** earlier ones |
| `ScrollView(cls, *children, horizontal=False)` | `flex flex-col overflow-auto min-h-0` | `horizontal=True` → `flex flex-row overflow-x-auto min-w-0` |
| `List(cls, count=, item_height=, builder=)` | `flex flex-col overflow-auto min-h-0` | virtualised |

```python
Column("flex-1 gap-4 p-6 bg-slate-950",
    Row("items-center justify-between",
        Text("text-lg text-white", "Title"),
        Button("px-3 py-1 rounded bg-slate-700 text-white", "Edit", on_tap=edit),
    ),
    Grid("grid-cols-3 gap-2", *[Text("text-white", str(i)) for i in range(9)]),
)
```

**`Stack` — two bugs fixed in §31, worth knowing if you saw them.** Until §31, on the
native backend a later child's opaque background did **not** cover an earlier child's
text (every glyph in the tree painted above every background in it), and a `Stack` child
with no explicit `w-` was sized to its content instead of filling the stack. Both worked
on the WebView backend the whole time, so this was a silent divergence. Both are fixed
and pinned by conformance fixtures. If you wrote `w-full` or an explicit width on a
`Stack` child to work around the second one, you no longer need to.

**`ScrollView`/`List` gotcha that will bite you — read this one.** A `flex-1` scroll
region defaults to `min-height: auto`, which grows it to fit **all** its content — so
there is nothing to clip and nothing scrolls. `min-h-0` is baked into `ScrollView`,
`List` and `Nav`, but **every intermediate `flex-1` ancestor between the scroll region
and the nearest non-growing container needs `min-h-0` too — including the `Column` your
screen returns.** This is real CSS behaviour, not a ZKite quirk, and it is identical on
both backends.

The failure is silent and does not look like a layout bug. A real app hit it this way:

```python
def screen(nav: Navigator):
    return Column("flex-1 bg-zinc-950",      # <- needs min-h-0 and does not have it
        header,
        List("flex-1", count=600, item_height=76, builder=row),
        tab_bar,                              # <- silently pushed off the display
    )
```

600 rows at 76px laid that screen out **30614dp tall against a 960dp viewport**. No
error, no warning; the list looked perfect and the tab bar was simply never on screen.
The fix is `Column("flex-1 min-h-0 bg-zinc-950", ...)`.

**Rule of thumb: if a screen has a scroll region *and* anything below it, put `min-h-0`
on every `flex-1` container from the screen root down to the scroll region.**

### Horizontal scrolling — `ScrollView(..., horizontal=True)`

```python
ScrollView("gap-2 p-2",
    *[Card(i) for i in range(20)],
    horizontal=True,
)
```

**Works on both backends as of §27.** It used to clip its content and never move on
native; that was §15.1's divergence 1.

- **Use `horizontal=True`, not hand-written classes.** `ScrollView("flex-row
  overflow-x-auto")` would work on native and *not* on WebView: `twcore` resolves a
  class string last-wins, but real Tailwind decides `flex-col` vs `flex-row` by
  stylesheet order, not attribute order. The flag swaps the whole implied set so
  neither backend has a conflict to resolve.
- **Put `shrink-0` on the children.** This is the x-axis twin of the `min-h-0` trap
  and it fails just as silently: a flex child with an explicit width still shrinks to
  fit unless you stop it, so the strip ends up exactly as wide as its viewport and
  there is nothing to scroll. `min-w-0` on the strip itself is already baked in.
- **A drag picks one axis and keeps it.** The axis is chosen from the movement that
  crosses the touch slop and does not change for the rest of that drag — so a strip
  inside a vertical list gets horizontal drags, the list gets vertical ones, and a
  sloppy diagonal does one thing rather than both. A 45° tie goes to vertical.
- **A strip with nothing to scroll passes the drag through** to whatever is behind it,
  rather than swallowing it.
- **Momentum works on both axes**, and clamps at the bounds. There is still no
  overscroll/rubber-band.
- **Fling distance is superlinear in flick speed** (native backend, §41): it follows
  `v^1.74`, the same exponent Android's `OverScroller` uses, so a gentle flick settles
  within a row or two while a hard one crosses many screens. Before §41 it was exactly
  linear, which read as every flick having the same gearing.

**Still missing: `Carousel`/`PageView`.** Horizontal scrolling unblocks them; it does
not provide them. A carousel needs snap-to-page, which is a different animation from
fling decay and does not exist yet.

**No horizontal virtualisation.** `List` is vertical-only, and horizontal scroll
position is never reported to Python — a horizontal strip builds all of its children
every frame. Fine for tens of cards, not for thousands.

### `List` — the virtualised one

```python
List("flex-1", count=10_000, item_height=56,
     builder=lambda i: Row("items-center px-4", Text("text-white", f"Item {i}")))
```

- **`item_height` is a contract, not a hint.** Rows are wrapped in a fixed-height
  `overflow-hidden` box, so a taller row is clipped rather than desynchronising the
  scroll maths. Variable heights are not supported.
- **`gap-*` raises `ValueError` at construction** — a windowed list has a different
  number of gaps than a full one, which would break the scroll extent. Put the spacing
  *inside* the row and include it in `item_height`.
- **`overscan` (default 4) is how many rows of slack are built beyond the visible
  ones, and it is what decides how *often* scrolling re-runs your `view()`.** The
  window is kept until the viewport reaches its edge, so a rebuild happens roughly
  once per `overscan + 1` rows scrolled rather than once per row. Raising it makes
  rebuilds rarer but each one bigger; lowering it to `0` re-runs `view()` on every
  row boundary. Leave it alone unless you have measured a `sync` spike (§39.5, §40).
- Measured: 10,000 rows at ~42 fps, versus ~0.26 fps un-virtualised (§11.5, §11.9).

---

## Leaves

| Widget | Signature | Events |
|---|---|---|
| `Text` | `Text(cls, content)` | — |
| `Button` | `Button(cls, label, on_tap=, disabled=)` | `tap` |
| `TextField` | `TextField(cls, value=, placeholder=, type=, secure=, on_change=)` | `change` |
| `Switch` | `Switch(cls, [label], checked=, on_change=, indicator=)` | `change` |
| `Checkbox` | `Checkbox(cls, [label], checked=, on_change=, indicator=)` | `change` |
| `Radio` | `Radio(cls, [label], checked=, on_change=, indicator=)` | `change` |
| `Slider` | `Slider(cls, min=, max=, step=, value=, on_change=)` | `change` |
| `Icon` | `Icon(cls, name)` | — |
| `Image` | `Image(cls, src=, alt=)` | — |

Any `on_*` callable prop registers an event named by its suffix, so `on_tap` → `"tap"`.
`tap`, `change` and `input` are dispatched by both backends (`on_input` was native-only
missing until §34.1).

> **Caveat — `on_change` on a `TextField` fires at different times per backend.**
> Measured in real Chromium: typing three characters emits `input` three times and
> **no** `change`; `change` arrives once, on blur. **Native dispatches `change` on every
> keystroke.** So `on_change` means "live typing" natively and "the user finished" on
> the WebView, from identical code. **If you want per-keystroke behaviour on both, use
> `on_input`** — that is what it is for, and it now behaves identically on both. The
> contract for `change` itself is an open owner decision (§34.9).

### `TextField`

```python
TextField("px-4 py-2 rounded-lg bg-slate-800 text-white",
          value=s.name, placeholder="Your name", on_change=lambda v: s.update(name=v))

TextField("...", secure=True, placeholder="Password", on_change=...)
TextField("...", type="email", on_change=...)   # text | email | url | tel | number
```

Fully working on native: real keyboard, live typing, tap-to-position-cursor,
double-tap/long-press word selection, a native copy/paste toolbar, password masking,
per-field keyboard types, and the field pans above the keyboard when focused (§7).
**Only tested against Gboard** (§7.13).

### `Switch` / `Slider`

```python
Switch(checked=s.on, on_change=lambda v: s.update(on=v))          # v is a bool
Slider("", min=0, max=100, step=1, value=s.vol,
       on_change=lambda v: s.update(vol=v))                        # v is a float
```

Both are drawn by ZKite (a track + thumb built from Tailwind tokens), not by the OS, so
they look identical on both backends. `on_change` receives the new value —
`bool` for `Switch`, `float` for `Slider`.

**Both work on native as of §26, and that sentence about types is only true as of §26
too.** Before it, `Switch` and `Slider` rendered and did nothing at all on the native
backend, and on *both* backends `on_change` received a **string** — so
`on_change=lambda v: s.update(on=v)` stored `"false"`, which is truthy, and a switch
could be turned on and never off. If you are reading old code that works around either,
the workaround is no longer needed.

- **Use `on_change`, not `on_tap`.** A `Switch` with `on_tap` is not the documented API
  and never was; it happened to work on native while `on_change` did not.
- **They are controlled.** What you see is whatever the last render said, so a control
  with no `on_change` will visibly not move, and one whose handler declines to update
  the store snaps back — the same as a React-controlled `<input>`.
- **A tap on a `Switch` also fires an ancestor's `on_tap`,** on both backends. That is
  ordinary DOM bubbling, not a bug — if you have a tappable row containing a switch,
  expect both handlers.
- **…but the two fire in opposite orders on the two backends** (measured in real
  Chromium, §27.6): WebView runs the **row's `on_tap` first**, native runs the
  **switch's `on_change` first**. Both handlers always run; only the order differs. It
  only matters if both handlers write the same store field — if they do, the last
  writer wins and that is a different one per backend. Don't rely on the order.
- **A drag that starts on a `Slider` belongs to the slider**, in any direction, and
  will not scroll a `ScrollView` around it. Dragging vertically on a slider moves the
  slider.
- `float` means `float`: a store field annotated `volume: int` will quietly hold
  `72.0`. Dataclasses do not enforce annotations.

### `Checkbox` / `Radio` — §30

```python
Checkbox(checked=s.wifi, on_change=lambda v: s.update(wifi=v))          # v is a bool

# A radio group is your store field. There is no `name`, and no grouping widget:
# exclusivity is `checked=(...)` on each one, and the re-render deselects the others.
Radio(checked=s.choice == "low",  on_change=lambda v: s.update(choice="low"))
Radio(checked=s.choice == "high", on_change=lambda v: s.update(choice="high"))
```

Both work on **both backends** and are drawn by ZKite, like `Switch` and `Slider`.

- **`Radio`'s payload is always `True`.** Tapping a radio — even one that is already
  selected — sends `True`, because a real radio button cannot be unchecked by clicking
  it. So the *handler* is what names the choice (`on_change=lambda v: s.update(...)`),
  not the value. A `Checkbox` toggles normally and sends `False` when you clear it.
- **Nothing enforces exclusivity for you.** Two radios both `checked=True` will both
  render filled. That is the same controlled contract every other ZKite widget has.
- **They are 20x20dp.** That is smaller than the ~48dp touch target Android's own
  guidance asks for, so put them in a padded `Row` if the tap needs to be forgiving.
- **A tap also fires an ancestor's `on_tap`**, exactly as `Switch` does — see the
  ordering caveat above, which applies identically here.
- No indeterminate/`mixed` state, and no `disabled`.

### Labels — just pass one as content (§32)

```python
Checkbox("text-sm text-white", "Wi-Fi", checked=s.wifi,
         on_change=lambda v: s.update(wifi=v))
```

**Tapping the text works, on both backends.** The whole control — box and label — is
one tap target. Before §32 the live area was the 20x20dp box and tapping the label did
nothing; the old workaround (a tappable `Row` wrapping a handler-less control) is no
longer needed, and if you are reading code that does it, it still works.

- **The label is ordinary content**, so `Checkbox("", Text("font-bold", "High"), ...)`
  works and is styled like any other `Text`.
- **The class string goes on the whole control**, not on the box — `Checkbox("text-sm
  text-white h-[44px]", "Wi-Fi", ...)` sizes and colours the *row*. The box keeps its
  own fixed 20dp size so it always lines up with its label.
- **The gap between box and label is 8dp**, applied as a margin on the box. Your own
  `gap-*` adds to it rather than replacing it — deliberately, because an implied `gap`
  would resolve differently on the two backends (the same Tailwind class-order footgun
  `ScrollView(horizontal=True)` exists to avoid).
- **A screen reader announces it properly.** Verified on a Moto G54: Android reports
  `android.widget.CheckBox` whose *name is the label* ("Wi-Fi: OFF"), so TalkBack says
  the label, the role and the state.
- **Hit target:** a labelled control is as wide as its content — measured at 400x36dp
  in the device fixture, against the bare control's 20x20dp. Add `h-[48px]` (or
  `py-*`) if you want the full ~48dp Android asks for; nothing does that for you.
- **A labelled control absorbs the tap.** If you put one inside a `Row(on_tap=...)`,
  only the control's handler fires — both backends stop at the nearest tappable
  ancestor. That is why the ordering trap below cannot arise here.

### Your own styling instead of the box — `indicator=False` (§32.5)

For a multiple-choice question, a segmented control or a colour swatch, where the
option itself shows the selection and a 20dp circle beside it would be wrong:

```python
def option(key, text):
    picked = s.answer == key
    return Radio(
        "px-4 py-3 rounded-xl border-2 text-sm " + (
            "border-indigo-500 bg-indigo-950 text-white" if picked
            else "border-slate-700 text-slate-300"),
        text, checked=picked, indicator=False,
        on_change=lambda v: s.update(answer=key))
```

`indicator=False` drops the built-in box/circle/track and nothing else. **Use this
rather than a tappable `Row`** — a `Row` is announced to a screen reader as generic
clickable text, while this stays a real `android.widget.RadioButton` carrying the
option text and its selected state (verified on a device). Content is optional, so a
bare swatch works too:

```python
Radio("w-12 h-12 rounded-full bg-red-500 ring-2 ring-white",
      checked=s.colour == "red", indicator=False, on_change=lambda v: s.update(colour="red"))
```

Exclusivity is still yours: `checked=` on each option, driven by one store field.

> **The one trap, and it only applies to the *bare* form.** If you put a control with
> `on_change` inside a `Row(on_tap=...)` and both write the same field, both handlers
> fire on a tap on the box. As of §32 they fire in the same order on both backends —
> the row's `on_tap` first, then the control's `on_change` — so the result is at least
> consistent, but two handlers writing one field is still a bug waiting to happen.
> Give the control a label instead and delete the row's handler.

### `Icon` — works on both backends

```python
Icon("text-white", "star")
```

Thirteen bundled sprites: `check`, `chevron-down`, `chevron-left`, `chevron-right`,
`close`, `heart`, `home`, `minus`, `plus`, `search`, `settings`, `star`, `trash`. A name
that isn't one of those draws nothing (it does not raise).

- **Sized `1em` by default** — an icon is one line tall and scales with the text around
  it, so `Icon` inside `text-[32px]` is 32px. Give it `w-8 h-8` (or any explicit size) to
  override.
- **Coloured by the surrounding text.** The sprite is painted in the node's own resolved
  text colour, so `text-white` on the icon *or on any ancestor* colours it. There is no
  separate fill prop.
- Both backends draw the same geometry by construction: `phase_3/renderer/scripts/gen_icons.py`
  transcribes the native sprite table from the very `<symbol>` sheet the WebView renders
  (§18).
- **No accessible-name prop yet** — a screen reader reads the sprite name (`"star"`),
  which is fine for `close`/`search` and poor for a decorative icon (§8.6).

### `Image` — a bitmap, on both backends (§35)

```python
Image("w-32 h-32 rounded-xl", src="logo.png", alt="Logo")
```

Put `logo.png` next to `main.py`. `zkite build` stages `.png`, `.jpg` and `.jpeg` from
your project directory alongside your Python. **Top level only** — a file in a
subdirectory is not staged yet.

`src` is either that relative path or a `data:` URI (handy when you already have the
bytes, e.g. from a request). A `src` that tries to escape the app directory is refused
on both backends.

**Size it yourself.** Unlike a browser's `<img>`, an `Image` never takes its size from
the bitmap — give it `w-`/`h-` classes or let a flex parent size it, exactly as you
would any other box. This means a slow or missing image can never reflow your layout.

**`object-fit`.** Every `Image` is `object-cover` unless you say otherwise;
`object-contain`, `object-fill`, `object-none` and `object-scale-down` all work, and
writing one *replaces* the default rather than competing with it. `rounded-*` clips the
bitmap, so `rounded-full` gives you an avatar.

```python
Image("w-14 h-14 rounded-full", src="avatar.png", alt="Ana")   # cover, circular
Image("w-full h-40 object-contain", src="chart.png", alt="Q3 revenue")  # letterboxed
```

**`alt` is the accessible name**, and `alt=""` deliberately leaves the image unnamed —
that is how you mark a purely decorative image so a screen reader skips it.

**A missing or unreadable `src` paints nothing** and leaves the box as it was; it does
not raise and does not stop the app. ZKite tries once and remembers, so a typo costs one
filesystem hit rather than one per frame.

**HTTPS URLs:** native PNG/JPEG images download asynchronously with verified TLS,
a 2 MiB download limit per image, four workers and a bounded cache. Declare
`android.permissions: ["INTERNET"]` in `app.json` and give the image dimensions.
HTTP cleartext URLs are not loaded. WebView uses its browser loader.

**Not yet:** WebP/GIF/AVIF, `object-position`, and any bound on how much
memory decoded images occupy — an app cycling through many distinct `src` values will
grow (§35.6).

### `Select` — a picker with a self-drawn popup (§33)

```python
Select("px-4 py-3 rounded-xl bg-slate-800 text-white",
       options=["Brazil", "India", "Kenya"],       # or [(value, label), ...]
       value=s.country,
       on_change=lambda v: s.update(country=v))
```

Works on both backends. Options are bare values (used as their own label) or
`(value, label)` pairs; `value=` says which one is selected, the same controlled
contract `Checkbox`/`Switch`/`Slider` use.

- **You do not have to hold the open state.** The widget keeps it, and one `@store`
  field for the *value* is the whole app-side cost. Pass `open=` to take it over, and
  then `on_open`/`on_dismiss` are yours.
- **The popup dismisses on a tap outside it, and on Back** — Back closes it instead of
  popping a screen or exiting.
- **Nothing about it is hardcoded.** Each of these is a class string with a default:

  | prop | default | what it is |
  |---|---|---|
  | `placement` | `items-center justify-center p-6` | where the panel sits. `items-end` is a bottom sheet, `items-start` a top menu |
  | `scrim` | `bg-black/50` | the dimming. `scrim=""` keeps outside-tap dismissal with no dim |
  | `panel` | a rounded slate card | the popup panel itself |
  | `option_class` | a padded row | one option |
  | `option_selected_class` | `bg-slate-700` | added to the current option |

- **`option=` draws a row yourself and keeps the semantics.** `option(value, label,
  selected)` returns the row's contents; the row keeps `role="option"`, its selected
  state, its accessible name and its tap. Pair it with `option_class=""` for full
  control. This matters for the same reason `indicator=False` does — a tappable `Row`
  is announced to a screen reader as generic clickable text.
- **`header=` puts a widget above the list** — a search field, typically. It is an
  ordinary widget, so a `TextField` there is an ordinary text field, keyboard and all.
- **Passing children replaces the trigger's content** (the selected label and a
  chevron), the same way a label replaces a `Checkbox`'s.
- Android reports the trigger as a real `android.widget.Spinner` named by the current
  option, and the list as an `android.widget.ListView`.
- **Not supported:** anchoring the panel under the trigger (nothing tells Python where a
  node landed on screen), multi-select, a `disabled` option, type-ahead, and any
  animation — it appears in one frame. The option list is not virtualised.

### `Overlay` — the layer under `Select`, and it is yours too (§33)

```python
Overlay("rounded-2xl bg-slate-800 p-4",             # the panel: your classes
        Text("text-white", "Anything at all"),
        placement="items-end",                      # a bottom sheet
        scrim="bg-black/60",
        on_dismiss=lambda: s.update(open=False))
```

Renders **above the whole app**, wherever you write it in the tree — it occupies no
space where it sits. A tap outside the panel calls `on_dismiss`; a tap on the panel does
nothing; Back calls `on_dismiss` before anything outer sees it. Build it conditionally
(`s.open and Overlay(...)`) the way you would any other widget.

This is the substrate a `Dialog`, a menu, a tooltip or your own picker would be built
on, and it is public so you do not have to reimplement "paint above everything and
dismiss on an outside tap". `Overlay` itself does not animate — it appears in one
frame — but its children take `anim=` like any other widget, so a fade or a slide is
yours to add (see Animation, below).

---

## Animation — `anim=` and `Anim` (§47)

**Native backend only.** The WebView backend has no animation driver yet: a declaration
renders there in its final state and `on_animend` never fires. Your app will print one
line saying so when it runs on that backend — it is not silent.

Animations are **declared on a widget**, not started by calling something. The same
declaration arrives on every frame (the tree is rebuilt ~60 times a second while you
scroll), and the driver starts it once.

```python
from zkite import Anim, Column, Text

Column("flex-1 items-center justify-center",
    Text("text-2xl text-white", "Hello"),
    anim=Anim.enter(opacity=(0, 1), translate_y=(16, 0), ms=220, ease="out"),
)
```

**Three channels: `opacity`, `translate_x`, `translate_y`.** Give a channel a target
(`opacity=1`) or a `(from, to)` pair (`opacity=(0, 1)`). An unstated `from` means
"wherever this node's classes already put it", which is usually what you want.

`rotate` and `scale` are **rejected with an error**, on purpose. They work on the WebView
backend and the native renderer does not paint them, so animating one would run, finish,
call your handler, and move nothing.

**Three triggers.**

| | when it runs |
|---|---|
| `Anim.enter(...)` | once, when the node first appears with this declaration |
| `Anim.change(...)` | whenever the target value changes — follows a moving number |
| `Anim.exit_(...)` | once, and tells you when it is done so *you* can remove the node |

**Timing: exactly one of `ms=` or `spring=`.**

```python
Anim.enter(opacity=(0, 1), ms=220, ease="out")        # ease: linear|ease|in|out|in_out
Anim.enter(opacity=(0, 1), ms=220, ease=[.2,0,.8,1])  # or a raw cubic-bezier
Anim.change(translate_x=x, spring=(180, 26))          # stiffness, damping
```

Also available: `delay=` (ms), `repeat=` (a count or `"forever"`), `dir=`
(`normal`/`reverse`/`alternate`/`alternate_reverse`). `repeat`/`dir` are for eased
tracks; a spring's duration comes from its own settling.

**Use a spring for anything the user can interrupt.** Interrupting a spring carries its
current position *and velocity* across, so it never jumps; an eased track carries
position only.

### Exiting: your `view()` holds the node until the animation finishes

The renderer cannot animate a node out of a tree it is no longer in. So the contract is
the other way round — you keep rendering it, and drop it when `on_animend` fires:

```python
@store
class S:
    leaving: bool = False
    gone: bool = False

def view(s: S):
    rows = [...]
    if not s.gone:
        rows.append(Row("bg-rose-500 h-[56px] w-[200px]",
            Text("text-white", "bye"),
            anim=Anim.exit_(opacity=(1, 0), translate_x=(0, -160), ms=200) if s.leaving else None,
            on_animend=lambda e: s.update(gone=True),
            on_tap=lambda: s.update(leaving=True),
        ))
    return Column("flex-1 items-center justify-center gap-3", *rows)
```

`on_animend` receives an `AnimEnd` with `.channel` and `.trigger`, so one handler can
tell an exit finishing from an entrance finishing.

**One performance rule, measured.** `opacity` composites the node's whole subtree into
an offscreen buffer; on the test phone one spanning the **full screen** costs ~1.9ms of a
~12ms frame. That is fine for a 200–300ms transition and not fine as a permanent
fixture. `translate_*` costs nothing extra — prefer it for anything long-lived, and keep
opacity groups small.

---

## Navigation (`Nav` + `Navigator`) — §16

```python
from zkite import Button, Column, Nav, Navigator, Text

def home(nav: Navigator):
    return Column("flex-1 items-center justify-center gap-6 bg-slate-950",
        Text("text-3xl font-bold text-white", "Home"),
        Button("px-6 py-3 rounded-2xl bg-indigo-500 text-white", "Settings",
               on_tap=lambda: nav.push(settings)),
    )

def settings(nav: Navigator):
    return Column("flex-1 items-center justify-center gap-6 bg-emerald-950",
        Text("text-3xl font-bold text-white", "Settings"),
        Button("px-6 py-3 rounded-2xl bg-slate-700 text-white", "Back", on_tap=nav.pop),
    )

def detail(nav: Navigator, item_id: int):     # a pushed param
    return Column("flex-1 items-center justify-center bg-rose-950",
        Text("text-5xl text-white", str(item_id)))

def view():
    return Nav("flex-1", home, settings, detail)   # first screen listed is the root
```

- **A screen is an ordinary view function.** Stores come in by annotation exactly as
  they do for `view()`; anything else is matched against what it was pushed with.
- `nav.push(settings)` or `nav.push("settings")` — both work. Push params:
  `nav.push(detail, item_id=42)`.
- API: `push` · `pop` · `replace` · `pop_to_root` · `reset` · `depth` · `can_pop` ·
  `current`.
- **Back is wired on both backends.** The system button and the back gesture pop the
  stack; on the root screen they exit the app, exactly as a one-screen app always did.
- Nav state survives hot reload, and a pushed screen picks up your edits (§16.3).
- **No screen transition animations.** The driver now exists (§47, and see Animation
  above); `Nav` does not use it yet, so a screen still replaces another in one frame.

---

## Styling — what resolves and what silently doesn't

Tailwind classes compile to native structs (`twcore`); the palette is generated from
the same Tailwind v4 bundle the WebView backend uses, so **colours agree exactly across
both backends** (§9.6).

**Works on both:** flex/grid layout, `gap`, padding/margin, width/height (incl.
arbitrary `w-[137px]`), colours (`bg-*`, `text-*`), `rounded-*`, `opacity-*`,
`translate-x-*`/`translate-y-*`,
`font-sans`/`serif`/`mono`, `font-bold`, `text-{size}`, `leading-*`, `tracking-*`,
`tabular-nums`, `text-align` (see caveat), `overflow`, `border-*`, `shadow-*`, `ring-*`,
linear gradients (`bg-linear-*` + `from-*`/`via-*`/`to-*`).

> **Correction (§45).** This line claimed `opacity` worked on both backends and it did
> not — `opacity-*` and `translate-*` compiled to a `ComputedStyle` field that no native
> paint code read, while the WebView backend painted both correctly. Measured against
> real Chromium: 42% of pixels wrong on an opacity fixture, 15% on a translate one. Both
> now paint on both backends and have conformance fixtures. **`rotate-*` and `scale-*`
> are still parsed and silently dropped on native** — see below.

- **`opacity-*` fades a node *and everything inside it*, as one group** — the same as
  CSS. An `opacity-50` box containing an opaque panel shows that panel at 50% over the
  background, not the panel blended with whatever it covers.
- **It costs real time on native, and the cost scales with the group's *area*, not with
  what is inside it.** A group is an offscreen render target. Measured on a Moto G54
  (§46.6): one `opacity-*` spanning the **full screen** costs **~1.9ms per frame** — 16%
  of the frame budget — for as long as it is on screen. A small group is proportionally
  cheaper, and nesting two means two targets. So: use it freely for a badge or a
  disabled control; think twice before wrapping a whole screen in one and leaving it
  there. `translate-x-*`/`translate-y-*` open no layer and are effectively free.
- **`translate-x-*`/`translate-y-*` move a node and its children without moving anything
  else.** Siblings stay where they were, so a translated node overlaps rather than
  displaces — and a tap lands where the node is *drawn*, not where it was laid out.
- **`rotate-*` and `scale-*` do nothing on native** (they work on WebView). They parse,
  so no unknown-class warning tells you. Avoid them if you need both backends.

### Borders, shadows and rings — §28

```python
Column("border-2 border-slate-700 rounded-xl shadow-md p-4", ...)
Column("border-t-4 border-b-2 border-red-500", ...)   # per-side widths work too
Column("ring-2 ring-indigo-500 rounded-lg", ...)
```

**All three paint on both backends as of §28.** Per-side widths and colours
(`border-t-4 border-l-2`, `border-t-amber-400`) work, not just uniform ones.

- **A border with no colour class takes the surrounding text colour.** That is
  Tailwind v4's `currentColor` default, and it is what real Chromium does — so
  `Column("border-2", ...)` inside `text-emerald-400` draws an emerald border, not a
  grey one. The same is true of `ring`. If you are reading older ZKite code that assumes
  a grey default, that assumption was v3's and was wrong on the WebView backend too.
- **A bare `ring` is 1px, not 3px** — also a v4 change that ZKite was getting wrong.
- **The shadow scale is v4's, which is not v3's.** `shadow-xs` is what v3 called
  `shadow-sm`, and `shadow-sm` is what v3 called plain `shadow`. If a shadow looks a
  step lighter than you expected, that is why, and both backends agree.
- **A ring is drawn *outside* the box** and nothing reserves room for it, so it overlaps
  whatever sits next to it. Add spacing yourself. A border is drawn *inside* the box and
  the layout already accounts for it.
- **Shadows are subtle on dark backgrounds** — Tailwind's whole scale is low-alpha
  black, so on `bg-slate-950` a `shadow-md` is close to invisible. That is real Tailwind
  behaviour, not a ZKite gap.
- **`shadow-2xl` is noticeably darker on native than on WebView** — measured on a real
  device against real Chromium: about 1.75x the darkness at the box edge, with the right
  extent but too much of it concentrated in the middle (§29.8). It is the only size that
  diverges; `xs` through `xl` agree closely. If you need the two backends to match
  exactly, prefer `shadow-xl`.

**Not supported:** `shadow-inner` (inset), coloured shadows (`shadow-red-500`),
arbitrary shadows (`shadow-[...]`), and `ring-offset-*`. All four are unparsed rather
than mis-painted.

### Gradients — §38

```python
Column("bg-linear-to-r from-cyan-500 to-blue-500", ...)
Column("bg-linear-to-br from-fuchsia-600 via-violet-500 to-cyan-400", ...)
Column("bg-linear-45 from-yellow-400 to-blue-600", ...)      # a bare angle
Column("bg-linear-[30deg] from-yellow-400 to-blue-600", ...)  # an arbitrary one
```

**Linear gradients paint on both backends**, in all eight keyword directions (`to-t`,
`to-r`, `to-b`, `to-l`, `to-tr`, `to-tl`, `to-br`, `to-bl`), at any angle, with two or
three stops. v3's `bg-gradient-to-r` spelling still works — it is still live in Tailwind
v4 itself, so ZKite accepts it too.

The rules, all of which are what real Tailwind does rather than ZKite's own inventions:

- **You need a direction *and* at least one colour stop.** `from-cyan-500 to-blue-500`
  with no `bg-linear-*` paints nothing, and so does `bg-linear-to-r` on its own. This is
  not a ZKite limitation: it is exactly what the same classes do in a browser.
- **A missing stop is `transparent`, not absent.** `bg-linear-to-r from-cyan-500` fades
  from cyan to transparent — which is genuinely useful over a background colour.
- **A gradient and `bg-<colour>` coexist.** `bg-red-500 bg-linear-to-r from-yellow-400`
  paints yellow fading to red, because the gradient is `background-image` and the colour
  is `background-color` underneath it.
- **Stop positions:** `from-25%`, `via-30%`, `to-75%`. They default to 0% / 50% / 100%,
  and setting one does not move the others.
- **Opacity modifiers and arbitrary colours work on stops:** `from-cyan-500/50`,
  `to-[#0000ff]`.
- **A gradient is clipped by the node's own `rounded-*`**, so a `rounded-full` pill with
  a gradient works as you would expect.

Two caveats worth knowing:

- **`bg-radial-*` and `bg-conic-*` do not exist yet** on either backend in ZKite. They are
  reported as unknown classes rather than silently ignored, so `zkite doctor` will tell
  you.
- **The `/srgb` interpolation modifier diverges from the WebView backend** by up to
  54/255 on a saturated pair (§38.5 explains why — it is a limitation in the GPU
  library's colour ramp, not in the parser). The default (no modifier) matches Chromium
  to within a couple of units; prefer it.

**Silently does nothing on native — works on WebView:**

| Class | Status |
|---|---|
| ~~`bg-gradient-*`, `from-*`, `via-*`, `to-*`~~ | **Now painted on both backends** (§38). See the gradient section below. |
| `bg-radial-*`, `bg-conic-*` | **Not parsed.** Reported as an unknown class rather than mis-painted. §38.5 |
| `dark:` | Parses to a state bit **nothing ever sets** — never activates. §15.1 item 2 |
| Emoji in any text | No emoji face is bundled *and* there is no colour-glyph support. §15.1 item 4 |
| ~~`overflow-x-*`~~ | **Now scrolls on both backends** (§27). Prefer `ScrollView(horizontal=True)` |
| ~~`border-*`, `shadow-*`, `ring-*`~~ | **Now painted on both backends** (§28). See the note below. |
| `-translate-x-1/2` and other percent-of-own-size translates | Not parsed — `twcore` does pixel offsets only |
| `italic` | Pushed to the shaper, but no italic face is registered — renders upright |

**Missing on both backends:**

- `underline` / `line-through` — no paint support in either (§9.7)
- `transition-*` / `animate-*` — **no animation support anywhere at all**
- The system font-size accessibility setting is ignored by both backends (§15.1 item 5)
- `text-align` only applies when the text box's width is externally fixed; content-sized
  wrapped text is uncovered (§9.7)
- `font-['Inter']` and other arbitrary families — `twcore` has three family slots only

---

## What does not exist yet

From plan1.md §8's own v1 list: **`Sheet`** and **`Native(...)`**.

Beyond that list, a production app will want these and none of them exist:
`DatePicker`/`TimePicker`, `Dialog`,
`Toast`/snackbar, `ProgressIndicator`, `Carousel`/`PageView` (no longer *blocked* —
horizontal scrolling exists as of §27 — but it needs snap-to-page, which does not),
pull-to-refresh, sticky headers. `Dialog`, `Sheet` and `Toast` are each now one
composition over `Overlay` (§33), which does exist.

**Animation exists on the native backend only** — see `Anim` above. `transition-*` and
`animate-*` still parse to nothing on purpose; the API is explicit channels and
triggers, not Tailwind classes. A popup still appears in one frame and a screen still
replaces another in one frame, because `Sheet`/`Dialog`/`Toast` and `Nav` transitions
are compositions nobody has written yet — the driver they were blocked on is what
§47 built.

**Nothing can repaint on its own.** Every frame is drawn in response to a touch, a
scroll or a hot reload: a background thread that finishes work and calls
`store.update()` refreshes the tree and nothing paints it until the next interaction
(§33.8). So an app that fetches data in the background has no way to show the result
yet.

Also absent: Autofill, deep links, permissions, RTL/i18n, and state restoration after
Android kills a backgrounded app.

---

## Running your code

**Hot reload (fastest loop, no APK build):**

```bash
zkite new myapp && cd myapp
zkite dev                       # native backend by default
zkite dev --backend webview     # the other one
```

Edit `main.py` and save — the change appears in well under a second with `@store` state
preserved. `zkite dev` installs and launches the Dev Shell for you if this checkout has a
prebuilt one, and tells you exactly what to run if it doesn't (§24). Nothing *downloads*
one yet, so from a fresh checkout you may still need
`ZKITE_DEVSHELL=1 ABI=arm64-v8a phase_3/gpu_harness/scripts/build_apk.sh` once.

**Your app's name and icon do not appear during `zkite dev`.** The Dev Shell is generic —
one prebuilt APK shared by every project — so the phone shows ZKite's name and icon while
`zkite dev` is running. `app.json` is read at build/preview preparation time; restart `zkite dev` after changing it.

**A real signed APK:**

```bash
zkite build apk                     # native
zkite build apk --backend webview
```

Your app's identity comes from `app.json`, read by build/run and preview preparation:

```json
{
  "name": "myapp",
  "android": {
    "package": "com.yourcompany.myapp",
    "label": "My App",
    "version_code": 1,
    "icon": "assets/icon.png",
    "permissions": ["INTERNET"]
  }
}
```

**`permissions` is what a networked app needs** (§34.4). A generated manifest requests
**nothing** by default, so without this an app that calls `urllib`/`socket` fails on the
device with a permission error and nothing naming the manifest as the cause. Bare names
are qualified for you (`INTERNET` → `android.permission.INTERNET`); anything already
containing a `.` is passed through unchanged, so vendor and custom permissions work.

`icon` is validated at build time: a non-PNG (a renamed `.jpg`, say) is rejected with the
path, because nothing downstream transcodes and it would otherwise fail on the device
(§22). Custom icons are a single legacy `drawable` — launchers mat them onto a white
plate rather than filling the icon mask, and adaptive icons are not supported yet (§22.2).

**If you hit something that renders wrong**, check this file's caveat tables first —
most "it didn't work" cases here are one of the documented silent divergences, and the
same code on `--backend webview` is a fast way to tell a ZKite bug from a native-renderer
gap.


## Background work and network requests

ZKite renders whenever a `@store` changes, **including from a thread you started
yourself** (§34). Nothing repaints on its own without that — before it existed, a
background update refreshed the tree and the screen only caught up on the next touch.

```python
import threading, urllib.request
from zkite import Button, Column, Text, store

@store
class Data:
    status: str = "idle"
    body: str = ""

def _fetch(d: Data):
    with urllib.request.urlopen("https://example.com", timeout=15) as r:
        code, text = r.status, r.read(200).decode("utf-8", "replace")
    d.update(status="ok %d" % code, body=text)       # this repaints, with no touch

def view(d: Data):
    return Column("flex-1 p-6 gap-4",
        Button("px-6 py-3 rounded-2xl bg-indigo-500 text-white", "Fetch",
               on_tap=lambda: _start(d)),
        Text("text-white", d.status),
    )

def _start(d: Data):
    d.update(status="fetching…")
    threading.Thread(target=_fetch, args=(d,), daemon=True).start()
```

Rules worth knowing:

- **Do the slow thing on a thread, not in `view()`.** `view()` runs on every state change
  and ZKite holds a lock across it, so blocking there stalls other frames.
- **Don't call `update()` from inside `view()`.** It is a no-op by design (the render
  already in progress is the one that will contain your change), but arming a timer or
  kicking off a first fetch belongs in a `threading.Timer`, not inline.
- **Add `"permissions": ["INTERNET"]`** to `android` in `app.json`, or the request fails on
  the device with nothing naming the manifest.
- **HTTPS works and validates against the device's own CA store.** You do not need
  `certifi` and you should not disable verification.
- A burst of updates is coalesced to the newest frame, so a chatty worker costs one
  repaint per burst rather than one per update.
