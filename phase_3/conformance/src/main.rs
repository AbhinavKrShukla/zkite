//! Phase 3 §5 item 5 — per-widget conformance testing against the WebView backend.
//! See ../PLAN.md §5 and ../fixtures.py's own module doc for the full pipeline:
//!
//!   fixtures.py --(html, tree)--> manifest.json
//!     tree --(this binary, `render`)-------------> <name>_native.png
//!     html --(screenshot_web.mjs, real Chromium)--> <name>_web.png
//!   <name>_native.png, <name>_web.png --(this binary, `diff`)--> <name>_diff.png + stats
//!
//! `render` reuses exactly `spike1_render_pipeline`'s own layout->paint pipeline
//! (`zkite_renderer::Renderer` + `vello_cpu`), just fed a fixture's `to_tree()` JSON via
//! `zkite_pybridge::{build, RawNode}` instead of a hand-built tree — the same
//! `RawNode`/`build` path `gpu_harness`'s render thread uses for the real app, so this
//! binary exercises the actual native-tree-to-pixels path, not a simplified stand-in.

use std::path::PathBuf;

use zkite_pybridge::RawNode;
use zkite_renderer::{PositionedGlyph, Renderer};
use serde::{Deserialize, Serialize};
use vello_cpu::color::AlphaColor;
use vello_cpu::kurbo::Rect;
use vello_cpu::peniko::Color as PenikoColor;
use vello_cpu::{Glyph as VelloGlyph, Pixmap, RenderContext, Resources};

#[derive(Deserialize)]
struct FixtureEntry {
    width: u32,
    height: u32,
    tree: serde_json::Value,
}

fn twcore_color_to_peniko(c: zkite_renderer::Color) -> PenikoColor {
    PenikoColor::from_rgba8(c.r, c.g, c.b, c.a)
}

/// Rasterizes `root`'s already-`layout()`-ed tree to an RGBA8 buffer of exactly
/// `w`x`h` pixels — the same background-then-glyphs paint loop
/// `spike1_render_pipeline/src/main.rs` uses, factored out here so `render_one` stays
/// readable. No scrolling (`&Default::default()`), same as that spike: every fixture
/// here is a single static frame, not an interactive scroll.
/// `w`/`h` are the canvas size in **device pixels**; `dpr` is the device pixel ratio
/// the tree was laid out against, so the dp-space geometry in `nodes` is scaled by it
/// exactly the way `gpu_harness::render_thread`'s own paint does on a real phone (see
/// ../PLAN.md §13). At `dpr == 1` this is the identity and every pre-existing fixture
/// number is unchanged.
/// A node's own box as a path, honouring its corner radii — the shape a background
/// fill and an `Image`'s clip both need. The third copy of this (one per paint
/// consumer); `gpu_harness` holds the other two, and all three are exercised by the
/// same fixtures.
fn rounded_box_path(rect: (f32, f32, f32, f32), radius: &zkite_renderer::Corners) -> vello_cpu::kurbo::BezPath {
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

fn paint(r: &Renderer, root: zkite_renderer::NodeId, w: u32, h: u32, dpr: f32) -> Vec<u8> {
    let nodes = r.positioned(root, &Default::default());

    // The *same* decoder the device runs (../PLAN.md §35.4). Sharing `zkite_imagestore`
    // rather than decoding here is what makes an image fixture's diff a measurement of
    // ZKite against Chromium, instead of a measurement of one decoder against another.
    //
    // Rooted at this directory: an image fixture uses a `data:` URI, which needs no
    // root at all, so nothing here actually reads a file — the root exists because
    // `ImageStore` requires one, not because a fixture uses it.
    let mut images = zkite_imagestore::ImageStore::new(env!("CARGO_MANIFEST_DIR"));

    let mut ctx = RenderContext::new(w as u16, h as u16);
    let mut resources = Resources::new();
    // The one line that makes this harness able to see a density bug at all. It is
    // deliberately the *same mechanism* the device uses — a scene transform, which
    // `glifo` absorbs into the font size so glyphs are outlined and hinted at device
    // resolution — rather than a re-layout at a larger size, which would test
    // something the real backend never does.
    let base_transform = vello_cpu::kurbo::Affine::scale(dpr as f64);
    ctx.set_transform(base_transform);

    // Everything a node draws, drawn together, in tree order — the same sequence and
    // the same ordering contract as `gpu_harness`'s two paint paths (../PLAN.md §28.3,
    // §31.1). Note the `.rev()` on shadows: `box-shadow`'s first layer paints on top.
    //
    // This was three whole-tree passes until §31.1, which is what made this harness
    // structurally unable to see the bug it exists to catch: with all glyphs painted
    // after all backgrounds, `stack`'s fixture agreed with Chromium only because both
    // its children are opaque rectangles with no text in them.
    // CSS `opacity` composites a node *and its subtree* and fades the result, so it
    // is a layer spanning a range of this flat list rather than a per-node alpha.
    // `OpacityGroups` owns the "which layers are open" arithmetic for all three of
    // this repo's paint loops (../PLAN.md §45.3); this one contributes the two
    // `vello_cpu` calls.
    let mut groups = zkite_renderer::OpacityGroups::new();
    for (i, n) in nodes.iter().enumerate() {
        for _ in 0..groups.close_before(i) {
            ctx.pop_layer();
        }
        if let Some(alpha) = groups.open_at(i, n) {
            ctx.push_opacity_layer(alpha);
        }
        let (x, y, bw, bh) = n.rect;
        let deco = zkite_renderer::box_decoration(n.rect, &n.style, n.style.text.color);

        for s in deco.shadows.iter().rev() {
            ctx.set_paint(AlphaColor::<vello_cpu::color::Srgb>::from_rgba8(s.color.r, s.color.g, s.color.b, s.color.a));
            ctx.fill_blurred_rounded_rect(&s.rect, s.radius as f32, s.std_dev as f32, false);
        }
        if let Some(ring) = &deco.ring {
            let c = ring.color;
            ctx.set_paint(AlphaColor::<vello_cpu::color::Srgb>::from_rgba8(c.r, c.g, c.b, c.a));
            ctx.set_stroke(vello_cpu::kurbo::Stroke::new(ring.width));
            ctx.stroke_path(&ring.path);
        }
        // `background-color` and then `background-image` — two CSS properties over the
        // same box, in that order, so a gradient with a transparent stop lets the colour
        // beneath it show through (../PLAN.md §38.1 measured exactly that in Chromium).
        let fill_box = |ctx: &mut RenderContext| {
            let rect = Rect::new(x as f64, y as f64, (x + bw) as f64, (y + bh) as f64);
            let radius = &n.style.paint.radius;
            if radius.is_zero() {
                ctx.fill_rect(&rect);
            } else {
                ctx.fill_path(&rounded_box_path(n.rect, radius));
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

        // A bitmap sits where an icon does — above this node's own background and
        // border, below its own text. Geometry from `zkite_renderer::object_fit_rect`,
        // the same function both device paint paths call, so this fixture measures
        // that geometry against Chromium rather than a restatement of it.
        if let Some(src) = &n.image {
            if let Some(decoded) = images.get(src) {
                let (dx, dy, dw, dh) = zkite_renderer::object_fit_rect(
                    n.style.paint.object_fit,
                    (bw, bh),
                    (decoded.width as f32, decoded.height as f32),
                );
                if dw > 0.0 && dh > 0.0 {
                    let (ax, ay) = (x + dx, y + dy);
                    let (fx0, fy0) = (ax.max(x), ay.max(y));
                    let (fx1, fy1) = ((ax + dw).min(x + bw), (ay + dh).min(y + bh));
                    if fx1 > fx0 && fy1 > fy0 {
                        let rounded = !n.style.paint.radius.is_zero();
                        if rounded {
                            ctx.push_clip_path(&rounded_box_path(n.rect, &n.style.paint.radius));
                        }
                        ctx.set_paint(vello_cpu::Image {
                            image: vello_cpu::ImageSource::Pixmap(decoded.pixmap.clone()),
                            sampler: vello_cpu::peniko::ImageSampler::default(),
                        });
                        ctx.set_paint_transform(
                            vello_cpu::kurbo::Affine::translate((ax as f64, ay as f64))
                                * vello_cpu::kurbo::Affine::scale_non_uniform(
                                    (dw / decoded.width as f32) as f64,
                                    (dh / decoded.height as f32) as f64,
                                ),
                        );
                        ctx.fill_rect(&vello_cpu::kurbo::Rect::new(
                            fx0 as f64,
                            fy0 as f64,
                            fx1 as f64,
                            fy1 as f64,
                        ));
                        ctx.reset_paint_transform();
                        if rounded {
                            ctx.pop_clip_path();
                        }
                    }
                }
            }
        }

        // Icons sit between this node's background and this node's text: an icon must
        // cover its own node's background and must not cover its own node's glyphs.
        if let Some(icon) = &n.icon {
            ctx.set_transform(base_transform * icon.transform(n.rect));
            for shape in &icon.shapes {
                if let Some(c) = shape.fill.resolve(n.style.text.color) {
                    ctx.set_paint(AlphaColor::<vello_cpu::color::Srgb>::from_rgba8(c.r, c.g, c.b, c.a));
                    ctx.fill_path(&shape.path);
                }
                if let Some(stroke) = shape.stroke {
                    if let Some(c) = stroke.paint.resolve(n.style.text.color) {
                        ctx.set_paint(AlphaColor::<vello_cpu::color::Srgb>::from_rgba8(c.r, c.g, c.b, c.a));
                        ctx.set_stroke(stroke.to_kurbo());
                        ctx.stroke_path(&shape.path);
                    }
                }
            }
            ctx.set_transform(base_transform);
        }

        if let Some(runs) = &n.text {
            let color = n.style.text.color.map(twcore_color_to_peniko).unwrap_or(vello_cpu::color::palette::css::BLACK);
            ctx.set_paint(AlphaColor::<vello_cpu::color::Srgb>::from(color));
            for run in runs {
                let glyphs: Vec<VelloGlyph> =
                    run.glyphs.iter().map(|g: &PositionedGlyph| VelloGlyph { id: g.id, x: g.x + x, y: g.y + y }).collect();
                ctx.glyph_run(&mut resources, &run.font).font_size(run.font_size).hint(true).fill_glyphs(glyphs.into_iter());
            }
        }
    }
    for _ in 0..groups.close_all() {
        ctx.pop_layer();
    }

    ctx.flush();
    let mut target = Pixmap::new(w as u16, h as u16);
    ctx.render(&mut target, &mut resources);

    let mut raw = Vec::with_capacity(target.data().len() * 4);
    for px in target.data() {
        raw.extend_from_slice(&[px.r, px.g, px.b, px.a]);
    }
    raw
}

fn write_png(path: &PathBuf, w: u32, h: u32, rgba: &[u8]) {
    let file = std::fs::File::create(path).unwrap_or_else(|e| panic!("create {path:?}: {e}"));
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), w, h);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().unwrap();
    writer.write_image_data(rgba).unwrap();
}

fn read_png(path: &PathBuf) -> (u32, u32, Vec<u8>) {
    let file = std::fs::File::open(path).unwrap_or_else(|e| panic!("open {path:?}: {e}"));
    let decoder = png::Decoder::new(std::io::BufReader::new(file));
    let mut reader = decoder.read_info().unwrap_or_else(|e| panic!("read_info {path:?}: {e}"));
    let mut buf = vec![0u8; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf).unwrap_or_else(|e| panic!("next_frame {path:?}: {e}"));
    let bytes = &buf[..info.buffer_size()];
    // Normalize to RGBA8 — Playwright's element screenshots and this binary's own
    // `write_png` both already emit RGBA8, but a decoder-level normalization here
    // means `diff` doesn't silently misread a differently-encoded PNG (e.g. RGB,
    // no alpha) as garbage.
    let rgba = match info.color_type {
        png::ColorType::Rgba => bytes.to_vec(),
        png::ColorType::Rgb => {
            let mut out = Vec::with_capacity(bytes.len() / 3 * 4);
            for px in bytes.chunks_exact(3) {
                out.extend_from_slice(&[px[0], px[1], px[2], 255]);
            }
            out
        }
        other => panic!("{path:?}: unsupported PNG color type {other:?}"),
    };
    (info.width, info.height, rgba)
}

/// Renders one fixture natively at `dpr`.
///
/// **Layout is always at the fixture's declared size, in dp** — the fixture's
/// `w-[Npx] h-[Npx]` is a CSS-pixel size, which is what a dp is, and *not* scaling it
/// is the whole point: this mirrors exactly what the real backend does (lay out in dp,
/// scale once at paint, ../PLAN.md §13). Only the canvas grows. The WebView side gets
/// the same treatment for free from Playwright's `deviceScaleFactor`, which likewise
/// keeps CSS pixels fixed and multiplies the backing store.
///
/// So a fixture that renders identically at dpr 1 and 2 is evidence that both backends
/// agree about density, and a fixture that passes at 1 and fails at 2 is precisely the
/// bug §12.5.2 found on a device and §9.3 recorded this harness as structurally unable
/// to see.
fn render_one(entry: &FixtureEntry, out: &PathBuf, dpr: f32) {
    let tree_json = serde_json::to_string(&entry.tree).expect("fixture tree re-serializes");
    let root_node = RawNode::from_json(&tree_json).expect("fixture tree matches RawNode's shape");

    let mut r = Renderer::new(font_set()).expect("fonts register cleanly");
    let built = zkite_pybridge::build(&mut r, &root_node, None);
    r.layout(built.root, entry.width as f32, entry.height as f32);

    let (cw, ch) = scaled_size(entry.width, entry.height, dpr);
    let rgba = paint(&r, built.root, cw, ch, dpr);
    write_png(out, cw, ch, &rgba);
}

/// A fixture's canvas size in device pixels at `dpr`. Rounding must match what
/// Chromium does for `deviceScaleFactor`, or the two PNGs differ in size and
/// `diff_one` refuses to compare them — Blink rounds the backing store, and every
/// fixture size here times a whole-number dpr is exact anyway.
fn scaled_size(w: u32, h: u32, dpr: f32) -> (u32, u32) {
    ((w as f32 * dpr).round() as u32, (h as f32 * dpr).round() as u32)
}

#[derive(Serialize)]
struct DiffReport {
    name: String,
    width: u32,
    height: u32,
    total_pixels: u64,
    mismatched_pixels: u64,
    mismatched_pct: f64,
    /// Same comparison at `STRICT_CHANNEL_TOLERANCE` — see that constant for why one
    /// number cannot answer both "is this the same picture" and "is anything missing".
    strict_mismatched_pixels: u64,
    strict_mismatched_pct: f64,
    max_channel_delta: u8,
}

/// Per-channel tolerance below which two pixels count as "the same" — deliberately
/// generous, not 0. `render_thread`/every other phase-3 session already found real,
/// distinct rendering engines (Chromium's text rasterizer/AA vs. `parley`+`vello_cpu`)
/// never produce byte-identical glyph edges even from the same font file at the same
/// size — a tolerance of 0 would fail every fixture with any text in it on font
/// antialiasing alone, which is noise, not the signal this tool exists to catch (a
/// missing background, a wrong color, a misplaced box). 40 was chosen empirically
/// against this session's own fixtures (see PLAN.md §5's writeup) — loose enough that
/// `button`/`text`/`column` (identical layout logic, same font, differing only in AA)
/// pass cleanly, tight enough that a genuinely wrong flat color (e.g. `bg-indigo-500`
/// vs `bg-indigo-600`, a real 40+ per-channel difference) still fails.
///
/// **That last claim turned out to be too optimistic, and the counter-example is worth
/// keeping here rather than only in PLAN.md** (§11): dark-theme surface colors are
/// closer together than 40. `bg-slate-800` (30,41,59) against `bg-slate-950` (2,6,23)
/// is a max per-channel delta of **36** — under this tolerance. So a `List` row whose
/// background rendered at a third of its correct width, a blatant structural
/// divergence, scored *identically* to a correct render (3.59%, to the pixel) because
/// only the row's white text ever exceeded 40 and that text was in the same place
/// either way. The harness built to catch exactly this class of bug did not catch it;
/// a `readpng` one-liner measuring where the row background actually ended did. Hence
/// `STRICT_CHANNEL_TOLERANCE` below.
const CHANNEL_TOLERANCE: i32 = 40;

/// A second, tighter tolerance reported alongside `CHANNEL_TOLERANCE`'s number.
///
/// The loose tolerance answers "is this the same picture, ignoring how two different
/// rasterizers draw the same glyph edge." It cannot also answer "is anything
/// structurally missing," because the thing that makes it immune to AA noise is
/// exactly what makes it blind to a low-contrast element (see above). Rather than
/// trade one blindness for the other by picking a single number, both are reported:
/// a fixture that is clean at 40 but large at 12 has something real going on that AA
/// alone doesn't explain, and is worth looking at the diff image for.
///
/// 12 is comfortably above 8-bit AA jitter between two rasterizers and comfortably
/// below any two adjacent Tailwind shades of the same hue (`slate-800` vs `slate-900`
/// is 15). It is a *reporting* threshold, not a gate — there is still no pass/fail
/// gate here (PLAN.md §9.3), and picking per-fixture thresholds is that work, not this.
const STRICT_CHANNEL_TOLERANCE: i32 = 12;

fn diff_one(name: &str, web: &PathBuf, native: &PathBuf, out_diff: &PathBuf) -> DiffReport {
    let (ww, wh, web_rgba) = read_png(web);
    let (nw, nh, native_rgba) = read_png(native);
    assert_eq!((ww, wh), (nw, nh), "{name}: web/native screenshots have different dimensions: {ww}x{wh} vs {nw}x{nh}");

    let mut diff_rgba = vec![0u8; web_rgba.len()];
    let mut mismatched: u64 = 0;
    let mut strict_mismatched: u64 = 0;
    let mut max_delta: i32 = 0;
    let total = (ww as u64) * (wh as u64);

    for i in 0..(total as usize) {
        let o = i * 4;
        let (wr, wg, wb) = (web_rgba[o] as i32, web_rgba[o + 1] as i32, web_rgba[o + 2] as i32);
        let (nr, ng, nb) = (native_rgba[o] as i32, native_rgba[o + 1] as i32, native_rgba[o + 2] as i32);
        let dr = (wr - nr).abs();
        let dg = (wg - ng).abs();
        let db = (wb - nb).abs();
        let d = dr.max(dg).max(db);
        max_delta = max_delta.max(d);
        if d > STRICT_CHANNEL_TOLERANCE {
            strict_mismatched += 1;
        }
        if d > CHANNEL_TOLERANCE {
            mismatched += 1;
            diff_rgba[o] = 255;
            diff_rgba[o + 1] = 0;
            diff_rgba[o + 2] = 0;
            diff_rgba[o + 3] = 255;
        } else {
            // Dimmed grayscale of the web reference, so a diff image still shows
            // roughly *what* was there, with mismatches unmissable in red on top.
            let gray = ((wr + wg + wb) / 3 / 3) as u8;
            diff_rgba[o] = gray;
            diff_rgba[o + 1] = gray;
            diff_rgba[o + 2] = gray;
            diff_rgba[o + 3] = 255;
        }
    }

    write_png(out_diff, ww, wh, &diff_rgba);

    DiffReport {
        name: name.to_string(),
        width: ww,
        height: wh,
        total_pixels: total,
        mismatched_pixels: mismatched,
        mismatched_pct: (mismatched as f64) / (total as f64) * 100.0,
        strict_mismatched_pixels: strict_mismatched,
        strict_mismatched_pct: (strict_mismatched as f64) / (total as f64) * 100.0,
        max_channel_delta: max_delta.min(255) as u8,
    }
}

fn load_manifest(path: &PathBuf) -> std::collections::BTreeMap<String, FixtureEntry> {
    let text = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("read {path:?}: {e}"));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("parse {path:?}: {e}"))
}

/// The three families **both** backends render with, read from the one directory
/// `phase_1/webruntime/scripts/stage_fonts.sh` materialises.
///
/// Reading the identical files the WebView side loads via `@font-face` is what makes
/// this harness measure ZKite's rendering rather than the difference between two
/// typefaces. Before it, native shaped DejaVu Sans while Chromium resolved
/// `sans-serif` to Noto Sans on this host, so `text`'s number was dominated by a
/// font mismatch no code change could ever close — see ../PLAN.md §9.4, which
/// measured that directly (forcing the page to DejaVu took `text` from 13.41% to
/// 5.00%) and left it open. A second, bold face per family is required so
/// `font-bold` has something heavier to select (§9.2 item 1).
fn font_set() -> zkite_renderer::FontSet {
    let dir = std::env::var("ZKITE_FONT_DIR").unwrap_or_else(|_| {
        let here = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        here.join("../../phase_1/webruntime/fonts").to_string_lossy().into_owned()
    });
    let faces = |stem: &str| {
        let read = |name: String| {
            let path = std::path::Path::new(&dir).join(&name);
            std::fs::read(&path).unwrap_or_else(|e| {
                panic!("couldn't read {} ({e}) — run phase_1/webruntime/scripts/stage_fonts.sh", path.display())
            })
        };
        zkite_renderer::FontFaces::new(read(format!("{stem}.ttf")), read(format!("{stem}-bold.ttf")))
    };
    zkite_renderer::FontSet {
        sans: faces("sans"),
        serif: Some(faces("serif")),
        mono: Some(faces("mono")),
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("render-all") => {
            let manifest_path = PathBuf::from(&args[2]);
            let out_dir = PathBuf::from(&args[3]);
            // Optional 4th arg, defaulting to 1.0 so every existing invocation and
            // every recorded number in ../PLAN.md means exactly what it did before.
            let dpr = args.get(4).map(|s| s.parse().expect("dpr must be a number")).unwrap_or(1.0f32);
            let manifest = load_manifest(&manifest_path);
            for (name, entry) in &manifest {
                let out = out_dir.join(format!("{name}_native.png"));
                render_one(entry, &out, dpr);
                let (cw, ch) = scaled_size(entry.width, entry.height, dpr);
                println!("rendered {name} -> {out:?} ({cw}x{ch} @{dpr}x)");
            }
        }
        Some("diff-all") => {
            let manifest_path = PathBuf::from(&args[2]);
            let out_dir = PathBuf::from(&args[3]);
            let manifest = load_manifest(&manifest_path);
            let mut reports = Vec::new();
            for name in manifest.keys() {
                let web = out_dir.join(format!("{name}_web.png"));
                let native = out_dir.join(format!("{name}_native.png"));
                let diff_out = out_dir.join(format!("{name}_diff.png"));
                let report = diff_one(name, &web, &native, &diff_out);
                println!(
                    "{name:<12} {:>6.2}%  strict {:>6.2}%  max_delta={:<3} ({} / {} px)",
                    report.mismatched_pct,
                    report.strict_mismatched_pct,
                    report.max_channel_delta,
                    report.mismatched_pixels,
                    report.total_pixels
                );
                reports.push(report);
            }
            let report_path = out_dir.join("report.json");
            std::fs::write(&report_path, serde_json::to_string_pretty(&reports).unwrap()).unwrap();
            println!("wrote {report_path:?}");
        }
        _ => {
            eprintln!("usage: conformance render-all <manifest.json> <out_dir>");
            eprintln!("       conformance diff-all <manifest.json> <out_dir>");
            std::process::exit(2);
        }
    }
}
