//! Host-only reproduction attempt for ../PLAN.md §7.6/§7.9: an unrelated text node
//! (the demo app's counter digit) permanently stops rendering once any `TextField`
//! anywhere in the tree becomes focused, even with nothing typed. §7.9 narrowed the
//! trigger down to a minimal 3-node tree (`Text` + `TextField` + one sibling `Text`)
//! via `ZKITE_APP_MAIN` bisection on a real device, but never got further because
//! that bisection needed device installs for every step. This crate asks a cheaper
//! question first: does the exact same 3-node tree, pushed through the *real*
//! `vello_cpu` paint pipeline (`zkite_pybridge::build` -> `zkite_renderer::Renderer`'s
//! `layout`/`positioned` -> the same `glyph_run`/`fill_rect`/clip-push-pop sequence
//! `gpu_harness::render_thread::paint_cpu_fallback` uses), reproduce the bug purely
//! on the host — no Android, no GPU, no device?
//!
//! **Result: no.** Run with default settings (33 total redraws — 25 unfocused, 5
//! focused, 3 blurred-again, deliberately far more than the "appeared on the third
//! redraw" the original device run needed), the "0" node's own rect never once came
//! back without its glyph's pixels actually painted. This is a real result, not a
//! shrug: the previous host-only investigation (`zkite_renderer`'s own
//! `empty_text_leaf_does_not_corrupt_a_sibling_after_repeated_layouts` test, ../
//! renderer/src/lib.rs) only ever checked *layout metadata* (glyph count from
//! `positioned()`) across repeated `clear()`+build+`layout()` cycles — it never
//! called any paint backend at all, since `zkite_renderer` deliberately has no
//! dependency on either vello flavor (see that crate's own module doc). This is the
//! first host-only attempt that actually rasterizes, using the identical call
//! sequence the real device's own CPU-fallback path uses, and it still comes back
//! clean.
//!
//! **What this rules out, with real evidence, not a guess:** `zkite_renderer`'s
//! layout/shaping, `zkite_pybridge`'s tree-building/focus-state synthesis, and
//! `vello_cpu`'s own rasterization — including any warm-state/glyph-cache
//! accumulation across many repeated redraws, and any per-frame data corruption from
//! the specific unfocused->unfocused->focused->focused...->blurred sequence a real
//! session goes through — are all now excluded as the root cause, at least for a
//! short-lived host process. (§7.6 had already ruled out GPU-vs-CPU-specificity and
//! the caret paint by other means; this rules out the rest of the *shared* pipeline
//! both backends paint through.)
//!
//! **What's left, and genuinely can't be tested from here:** the `ANativeWindow`
//! buffer-geometry/lock/blit tail of `paint_cpu_fallback` (deliberately *not*
//! replicated by this crate's own `paint()` below — there is no `ANativeWindow` on a
//! Linux host), the `vello_hybrid`/`wgpu` GPU path specifically (no GPU context set
//! up here either), and anything tied to a *real* surface resize
//! (`onSurfaceChanged`/`Resize`/`reconfigure` in `render_thread.rs`) triggered by the
//! keyboard actually showing/hiding on a real device — this crate never varies its
//! fixed `W`x`H` or calls anything resize-shaped, unlike a real focus transition.
//! §7.6's own "best working hypothesis" (something in the paint step's interaction
//! with the surface/window state during or after an IME-triggered resize) is,
//! if anything, *strengthened* by this crate's negative result rather than
//! contradicted by it: the more of the shared, portable pipeline gets cleared, the
//! more the remaining suspects concentrate on the Android-only tail this crate
//! structurally cannot reach. A future session with device access should reach for
//! real GPU-frame-capture tooling on the *surface*/*resize* path specifically,
//! per §7.6's own recommendation — not more tree bisection, which this crate's
//! result suggests has already been pushed past the point of paying off on its own.
//!
//! Run with `cargo run` (needs `ZKITE_SPIKE_FONT`/`ZKITE_SPIKE_FONT_BOLD` env vars if
//! DejaVu Sans isn't at the default path `zkite_renderer::Renderer::new` documents).

use zkite_pybridge::{build, FocusedField, RawNode};
use zkite_renderer::{PositionedGlyph, PositionedNode, Renderer};
use vello_cpu::color::AlphaColor;
use vello_cpu::kurbo::{Rect, RoundedRect, Shape};
use vello_cpu::peniko::Color as PenikoColor;
use vello_cpu::{Glyph as VelloGlyph, Pixmap, RenderContext, Resources};

const W: i32 = 400;
const H: i32 = 800;
/// Slate-950 (`bg-slate-950`, the demo app's own root background) — used to tell
/// "background" from "something got painted" in `region_has_nonbg_pixels`.
const BG: (u8, u8, u8) = (2, 6, 23);

/// The §7.9 minimal repro, verbatim: `Text("0")` + an empty `TextField` (`id="r.1"`,
/// matched by `FocusedField::zkite_id` below) + `Row(Text("Item 0"))`, as siblings —
/// the exact shape that section's on-device `ZKITE_APP_MAIN` bisection found was the
/// smallest tree that still reproduced the bug.
fn tree_json() -> String {
    serde_json::json!({
        "tag": "div", "id": "r", "classes": "flex flex-col bg-slate-950", "events": [], "attrs": {},
        "children": [
            {
                "tag": "span", "id": "r.0", "classes": "text-7xl font-bold text-white tabular-nums",
                "events": [], "attrs": {}, "children": [{"tag": "#text", "text": "0"}]
            },
            {
                "tag": "input", "id": "r.1", "classes": "appearance-none outline-none bg-slate-800 text-white",
                "events": [], "attrs": {"input_type": "text", "value": "", "placeholder": ""}, "children": []
            },
            {
                "tag": "div", "id": "r.2", "classes": "flex flex-row", "events": [], "attrs": {},
                "children": [{
                    "tag": "span", "id": "r.2.0", "classes": "text-white",
                    "events": [], "attrs": {}, "children": [{"tag": "#text", "text": "Item 0"}]
                }]
            }
        ]
    })
    .to_string()
}

/// Mirrors `gpu_harness::render_thread::paint_cpu_fallback` exactly — same two
/// passes (backgrounds, then text), same per-node clip push/pop, same empty-runs
/// guard — up to (not including) its `ANativeWindow` blit tail, which has no host
/// equivalent. Returns raw RGBA8 bytes instead of writing to a window buffer.
fn paint(nodes: &[PositionedNode]) -> Vec<u8> {
    let mut ctx = RenderContext::new(W as u16, H as u16);
    let mut resources = Resources::new();

    for n in nodes {
        let (x, y, w, h) = n.rect;
        if let Some(bg) = n.style.paint.background {
            let clip = n.clip.map(clip_path);
            if let Some(path) = &clip {
                ctx.push_clip_path(path);
            }
            ctx.set_paint(AlphaColor::<vello_cpu::color::Srgb>::from_rgba8(bg.r, bg.g, bg.b, bg.a));
            let rect = Rect::new(x as f64, y as f64, (x + w) as f64, (y + h) as f64);
            let radius = &n.style.paint.radius;
            if radius.top_left == 0.0 && radius.top_right == 0.0 && radius.bottom_left == 0.0 && radius.bottom_right == 0.0 {
                ctx.fill_rect(&rect);
            } else {
                let rr = RoundedRect::new(
                    rect.x0,
                    rect.y0,
                    rect.x1,
                    rect.y1,
                    (radius.top_left as f64, radius.top_right as f64, radius.bottom_right as f64, radius.bottom_left as f64),
                );
                ctx.fill_path(&rr.to_path(0.1));
            }
            if clip.is_some() {
                ctx.pop_clip_path();
            }
        }
    }
    for n in nodes {
        let (x, y, _w, _h) = n.rect;
        let Some(runs) = &n.text else { continue };
        if runs.is_empty() {
            continue;
        }
        let clip = n.clip.map(clip_path);
        if let Some(path) = &clip {
            ctx.push_clip_path(path);
        }
        let color =
            n.style.text.color.map(|c| PenikoColor::from_rgba8(c.r, c.g, c.b, c.a)).unwrap_or(vello_cpu::color::palette::css::BLACK);
        ctx.set_paint(AlphaColor::<vello_cpu::color::Srgb>::from(color));
        for run in runs {
            let glyphs: Vec<VelloGlyph> =
                run.glyphs.iter().map(|g: &PositionedGlyph| VelloGlyph { id: g.id, x: g.x + x, y: g.y + y }).collect();
            ctx.glyph_run(&mut resources, &run.font).font_size(run.font_size).hint(true).fill_glyphs(glyphs.into_iter());
        }
        if clip.is_some() {
            ctx.pop_clip_path();
        }
    }
    ctx.flush();
    let mut target = Pixmap::new(W as u16, H as u16);
    ctx.render(&mut target, &mut resources);
    target.take_unpremultiplied().into_iter().flat_map(|p| [p.r, p.g, p.b, p.a]).collect()
}

fn clip_path(rect: (f32, f32, f32, f32)) -> vello_cpu::kurbo::BezPath {
    use vello_cpu::kurbo::{Rect, Shape};
    Rect::new(rect.0 as f64, rect.1 as f64, (rect.0 + rect.2) as f64, (rect.1 + rect.3) as f64).to_path(0.1)
}

fn write_png(path: &str, rgba: &[u8]) {
    let file = std::fs::File::create(path).unwrap();
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), W as u32, H as u32);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().unwrap();
    writer.write_image_data(rgba).unwrap();
}

/// Does the "0" node's own rect contain any non-background pixel in the rasterized
/// `rgba` buffer? A direct pixel check, independent of glyph-count metadata (which
/// §7.6 already confirmed stays correct even when the real bug fires — the point of
/// this crate is to check what actually got *painted*, not what `positioned()` says
/// should have been).
fn region_has_nonbg_pixels(rgba: &[u8], rect: (f32, f32, f32, f32)) -> (bool, usize) {
    let (x, y, w, h) = rect;
    let mut count = 0usize;
    let x0 = x.max(0.0) as i32;
    let y0 = y.max(0.0) as i32;
    let x1 = (x + w).min(W as f32) as i32;
    let y1 = (y + h).min(H as f32) as i32;
    for py in y0..y1 {
        for px in x0..x1 {
            let idx = ((py * W + px) * 4) as usize;
            if idx + 3 >= rgba.len() {
                continue;
            }
            let (r, g, b) = (rgba[idx], rgba[idx + 1], rgba[idx + 2]);
            if (r, g, b) != BG {
                count += 1;
            }
        }
    }
    (count > 0, count)
}

/// One redraw: build (with or without `focused`), layout, position, paint, and check
/// whether "0" — always the first node with non-empty text, near the top of the
/// screen — still has real pixels. Returns `false` if it vanished.
fn redraw_and_check(renderer: &mut Renderer, root: &RawNode, focused: Option<&FocusedField>) -> bool {
    let built = build(renderer, root, focused);
    renderer.layout(built.root, W as f32, H as f32);
    let nodes = renderer.positioned(built.root, &Default::default());
    let rgba = paint(&nodes);
    let zero_node = nodes.iter().find(|n| n.text.as_ref().is_some_and(|t| !t.is_empty()) && n.rect.1 < 200.0);
    match zero_node {
        Some(zn) => region_has_nonbg_pixels(&rgba, zn.rect).0,
        None => false,
    }
}

fn main() {
    let font_path = std::env::var("ZKITE_SPIKE_FONT").unwrap_or_else(|_| "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf".to_string());
    let bold_font_path =
        std::env::var("ZKITE_SPIKE_FONT_BOLD").unwrap_or_else(|_| "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf".to_string());
    let font_bytes = std::fs::read(&font_path).unwrap_or_else(|e| panic!("couldn't read font at {font_path}: {e}"));
    let bold_font_bytes =
        std::fs::read(&bold_font_path).unwrap_or_else(|e| panic!("couldn't read bold font at {bold_font_path}: {e}"));
    let mut renderer = Renderer::new(zkite_renderer::FontSet::sans_only(font_bytes, bold_font_bytes))
        .expect("fonts register cleanly");

    let root = RawNode::from_json(&tree_json()).expect("tree JSON matches RawNode's shape");
    let focused = FocusedField { zkite_id: "r.1", text: "" };

    let mut all_ok = true;
    let mut check = |label: &str, ok: bool| {
        println!("{label}: '0' rendered = {ok}");
        all_ok &= ok;
    };

    for pass in 1..=25 {
        check(&format!("pass {pass:2} (unfocused)"), redraw_and_check(&mut renderer, &root, None));
    }
    for pass in 26..=30 {
        check(&format!("pass {pass:2} (FOCUSED)  "), redraw_and_check(&mut renderer, &root, Some(&focused)));
    }
    for pass in 31..=33 {
        check(&format!("pass {pass:2} (blurred)  "), redraw_and_check(&mut renderer, &root, None));
    }

    // One more, saved as a PNG for a human to look at directly.
    let built = build(&mut renderer, &root, Some(&focused));
    renderer.layout(built.root, W as f32, H as f32);
    let nodes = renderer.positioned(built.root, &Default::default());
    let rgba = paint(&nodes);
    write_png("/tmp/repro_7_6_final_focused.png", &rgba);

    println!();
    if all_ok {
        println!("RESULT: did not reproduce — '0' rendered correctly on all 33 passes. See this file's own module doc.");
    } else {
        println!("RESULT: REPRODUCED — see the pass(es) marked false above. This is new: update ../PLAN.md §7.6 with a real repro.");
        std::process::exit(1);
    }
}
