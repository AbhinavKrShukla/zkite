//! Phase 3 spike 1 — see ../PLAN.md §3.
//!
//! Renders the plan1.md §8 counter app end to end with no CSS/DOM/Blink anywhere,
//! using the shared `zkite_renderer` crate for layout + real text measurement, and
//! `vello_cpu` — plan1.md §5's own documented runtime fallback path — for paint:
//!
//!   class string --twcore--> ComputedStyle --taffy(+parley measure)--> positioned nodes
//!                                                                    `--vello_cpu--> pixels
//!
//! Host-only: no Android, no GPU.

use zkite_renderer::{PositionedGlyph, Renderer};
use vello_cpu::color::AlphaColor;
use vello_cpu::kurbo::{Rect, RoundedRect, Shape};
use vello_cpu::peniko::Color as PenikoColor;
use vello_cpu::{Glyph as VelloGlyph, Pixmap, RenderContext, Resources};

fn twcore_color_to_peniko(c: zkite_renderer::Color) -> PenikoColor {
    PenikoColor::from_rgba8(c.r, c.g, c.b, c.a)
}

fn main() {
    // --- 1. fonts (no system font discovery — see zkite_renderer::Renderer::new's
    //        doc comment for why. Loaded straight off this dev box's filesystem as a
    //        stand-in for what a real app would bundle as an asset. Bold is a second,
    //        separate face registered under the same family — see Renderer::new's
    //        doc comment and ../PLAN.md §9.2 item 1 for why a single face silently
    //        made every weight render identically.) ------------------
    let font_path = std::env::var("ZKITE_SPIKE_FONT")
        .unwrap_or_else(|_| "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf".to_string());
    let font_bytes = std::fs::read(&font_path)
        .unwrap_or_else(|e| panic!("couldn't read font at {font_path}: {e}"));
    let bold_font_path = std::env::var("ZKITE_SPIKE_FONT_BOLD")
        .unwrap_or_else(|_| "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf".to_string());
    let bold_font_bytes = std::fs::read(&bold_font_path)
        .unwrap_or_else(|e| panic!("couldn't read bold font at {bold_font_path}: {e}"));
    let mut r = Renderer::new(zkite_renderer::FontSet::sans_only(font_bytes, bold_font_bytes))
        .expect("fonts register cleanly");

    // --- 2. widget tree -> twcore -> taffy layout, with real text measurement ------
    // Note this is now the plan1.md §8 example verbatim, including the class
    // *placement* — `text-white font-medium` sits on the Button, exactly like
    // `phase_1/zkite_py/tests/test_app_counter.py`'s reference app, and "Increment" is
    // a bare string child with no classes of its own. That only renders correctly
    // because `Renderer::container` now inherits typography to its children
    // (`../src/lib.rs`'s `resolve_inherited`) the same way a browser inherits `color`/
    // `font-weight` from an ancestor with no rule of the child's own — previously this
    // spike sidestepped the gap by hardcoding `text-white` directly on the label's own
    // `.text()` call, which happened to look right without actually exercising
    // inheritance. Button also carries the phase-3-discovered
    // `flex items-center justify-center` fix (see ../../phase_1/PLAN.md's
    // "Correction" note and zkite_py's widgets.py) instead of hand-centering.
    let column = r.container(
        "flex flex-col flex-1 items-center justify-center gap-6 bg-slate-950",
        |r| {
            let counter_text = r.text("text-7xl font-bold text-white tabular-nums", "0");
            let button = r.container(
                "appearance-none flex items-center justify-center px-6 py-3 rounded-2xl bg-indigo-500 text-white font-medium",
                |r| vec![r.text("", "Increment")],
            );
            vec![counter_text, button]
        },
    );
    let screen = r.container("flex flex-col w-full h-full", |_| vec![column]);

    const W: f32 = 400.0;
    const H: f32 = 800.0;
    r.layout(screen, W, H);
    let nodes = r.positioned(screen, &Default::default()); // no scrolling in this spike

    for n in &nodes {
        let (x, y, w, h) = n.rect;
        println!(
            "rect ({x:>6.1}, {y:>6.1}) {w:>5.1}x{h:>5.1}  bg={:?}  text={:?}",
            n.style.paint.background,
            n.text.as_ref().map(|runs| runs.len())
        );
    }

    // --- 3. rasterize: backgrounds (with radius) first, then glyphs on top --------
    let mut ctx = RenderContext::new(W as u16, H as u16);
    let mut resources = Resources::new();

    for n in &nodes {
        let (x, y, w, h) = n.rect;
        // Shadows, ring, background, border — the same per-node sequence, and the same
        // shadows-in-reverse ordering contract, as `gpu_harness`'s two paint paths and
        // `conformance` (../PLAN.md §28.3). This spike is the fourth consumer of
        // `zkite_renderer::box_decoration`.
        let deco = zkite_renderer::box_decoration(n.rect, &n.style, n.style.text.color);
        for s in deco.shadows.iter().rev() {
            ctx.set_paint(AlphaColor::<vello_cpu::color::Srgb>::from_rgba8(
                s.color.r, s.color.g, s.color.b, s.color.a,
            ));
            ctx.fill_blurred_rounded_rect(&s.rect, s.radius as f32, s.std_dev as f32, false);
        }
        if let Some(ring) = &deco.ring {
            let c = ring.color;
            ctx.set_paint(AlphaColor::<vello_cpu::color::Srgb>::from_rgba8(c.r, c.g, c.b, c.a));
            ctx.set_stroke(vello_cpu::kurbo::Stroke::new(ring.width));
            ctx.stroke_path(&ring.path);
        }
        // `background-color` then `background-image` (a gradient), over the same box —
        // ../PLAN.md §38.1. The fourth paint consumer, kept in step with the other three
        // for the reason `box_decoration`'s doc gives.
        let fill_box = |ctx: &mut RenderContext| {
            let rect = Rect::new(x as f64, y as f64, (x + w) as f64, (y + h) as f64);
            let radius = &n.style.paint.radius;
            if radius.is_zero() {
                ctx.fill_rect(&rect);
            } else {
                let rr = RoundedRect::new(
                    rect.x0,
                    rect.y0,
                    rect.x1,
                    rect.y1,
                    (
                        radius.top_left as f64,
                        radius.top_right as f64,
                        radius.bottom_right as f64,
                        radius.bottom_left as f64,
                    ),
                );
                ctx.fill_path(&rr.to_path(0.1));
            }
        };
        if let Some(bg) = n.style.paint.background {
            ctx.set_paint(AlphaColor::<vello_cpu::color::Srgb>::from_rgba8(bg.r, bg.g, bg.b, bg.a));
            fill_box(&mut ctx);
        }
        if let Some(g) = zkite_renderer::background_gradient(n.rect, &n.style) {
            ctx.set_paint(g);
            fill_box(&mut ctx);
        }
        match &deco.border {
            Some(zkite_renderer::Border::Uniform(o)) => {
                let c = o.color;
                ctx.set_paint(AlphaColor::<vello_cpu::color::Srgb>::from_rgba8(c.r, c.g, c.b, c.a));
                ctx.set_stroke(vello_cpu::kurbo::Stroke::new(o.width));
                ctx.stroke_path(&o.path);
            }
            Some(zkite_renderer::Border::PerSide(sides)) => {
                for side in sides {
                    ctx.push_clip_path(&side.wedge);
                    let c = side.color;
                    ctx.set_paint(AlphaColor::<vello_cpu::color::Srgb>::from_rgba8(c.r, c.g, c.b, c.a));
                    ctx.fill_path(&side.band);
                    ctx.pop_clip_path();
                }
            }
            None => {}
        }
    }

    for n in &nodes {
        let (x, y, _w, _h) = n.rect;
        let Some(runs) = &n.text else { continue };
        let color =
            n.style.text.color.map(twcore_color_to_peniko).unwrap_or(vello_cpu::color::palette::css::BLACK);
        ctx.set_paint(AlphaColor::<vello_cpu::color::Srgb>::from(color));
        for run in runs {
            let glyphs: Vec<VelloGlyph> = run
                .glyphs
                .iter()
                .map(|g: &PositionedGlyph| VelloGlyph { id: g.id, x: g.x + x, y: g.y + y })
                .collect();
            ctx.glyph_run(&mut resources, &run.font)
                .font_size(run.font_size)
                .hint(true)
                .fill_glyphs(glyphs.into_iter());
        }
    }

    ctx.flush();
    let mut target = Pixmap::new(W as u16, H as u16);
    ctx.render(&mut target, &mut resources);

    let out_path = "counter_app.png";
    let file = std::fs::File::create(out_path).unwrap();
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), W as u32, H as u32);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().unwrap();
    let mut raw = Vec::with_capacity(target.data().len() * 4);
    for px in target.data() {
        raw.extend_from_slice(&[px.r, px.g, px.b, px.a]);
    }
    writer.write_image_data(&raw).unwrap();

    println!("wrote {out_path} ({W}x{H})");
}
