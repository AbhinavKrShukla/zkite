//! Host-only, headless-GPU check on one specific `vello_hybrid` usage question that
//! came up while investigating ../PLAN.md §7.6, and the regression guard it turned
//! into. **Read the result before the rationale: the hypothesis this crate was built
//! to test was wrong.**
//!
//! **The question.** `gpu_harness::render_thread::reconfigure()` builds a brand-new
//! `vello_hybrid::Renderer` on every surface resize (its `RenderTargetConfig` is
//! fixed-size at construction, so a resize genuinely needs a new one) while continuing
//! to reuse the single `vello_hybrid::Resources` created once in `try_init_gpu`. That
//! looks like a contract violation: `Resources`'s own doc comment says "You should
//! create one such instance per renderer." `Resources` owns the glyph-atlas
//! *bookkeeping* (which glyphs are resident, at which atlas slot) while the actual
//! atlas *texture* lives inside `Renderer`. If the bookkeeping said "already
//! uploaded" while the new `Renderer`'s texture was in fact empty, every
//! already-seen glyph would silently render blank after a resize — which is very
//! close to §7.6's reported symptom, and a resize is exactly what an IME show/hide
//! triggers.
//!
//! **The answer: it does not happen.** Pass 2 below renders through a *new*
//! `Renderer` against a *reused, already-primed* `Resources` and the glyph comes
//! back at full fidelity — byte-identical ink coverage to the freshly-paired
//! baseline. `vello_hybrid` evidently re-establishes whatever the new renderer needs
//! rather than trusting stale residency. So this is **not** a live bug in
//! `gpu_harness`, and it was **not** §7.6's cause.
//!
//! (§7.6's real cause turned out to have nothing to do with the GPU at all: the
//! SurfaceView was being repositioned by the status-bar height after an IME
//! show/hide cycle, drawing the top of the tree underneath the status bar. See
//! ../PLAN.md §7.14.)
//!
//! **Why this crate is kept anyway.** The reuse pattern is undocumented-as-safe —
//! the upstream doc still says one `Resources` per `Renderer`, and `gpu_harness`
//! deliberately does otherwise. That makes it exactly the kind of thing a future
//! `vello_hybrid` upgrade could silently break, with a failure mode (text vanishing
//! only after a resize, only on device) that is brutal to diagnose from scratch —
//! this investigation is the proof. So the assertions are inverted from how they
//! started: this now **guards** the safety of the reuse rather than trying to
//! demonstrate its danger, and fails loudly if a version bump ever changes it.
//!
//! It is also the first host-side harness in this repo that drives the real
//! `vello_hybrid`/`wgpu` GPU path at all (headless, render-to-texture, no window and
//! no device required — a plain software Vulkan ICD such as lvp/llvmpipe suffices),
//! which is reusable for any future GPU-path question that previously would have
//! needed a phone.
//!
//! Run with `cargo run --release --offline`.

use zkite_renderer::Renderer as ZKiteRenderer;
use kurbo::Rect;
use vello_hybrid::{RenderSize, RenderTargetConfig, Renderer, Resources, Scene, TextureBindings};

const WIDTH: u32 = 128;
const HEIGHT: u32 = 96;
const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
/// `bg-slate-950`-ish, matching `gpu_harness`'s own reference app background — any
/// solid color works; what matters is that it's distinguishable from the white glyph
/// fill below.
const BG: (u8, u8, u8, u8) = (2, 6, 23, 255);

fn font_paths() -> (String, String) {
    let regular = std::env::var("ZKITE_SPIKE_FONT")
        .unwrap_or_else(|_| "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf".to_string());
    let bold = std::env::var("ZKITE_SPIKE_FONT_BOLD")
        .unwrap_or_else(|_| "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf".to_string());
    (regular, bold)
}

/// Shapes a single "0" glyph (`text-7xl`-sized, matching the reference counter app)
/// through the real `zkite_renderer` pipeline — the same measure/shape path
/// `gpu_harness::render_thread::redraw()` uses for every text leaf — and returns its
/// absolute-space `GlyphRun`s, exactly as `paint_nodes_into_scene`/`paint_cpu_fallback`
/// consume them (glyph coordinates already shifted by the node's own rect origin).
fn shape_zero_glyph() -> Vec<zkite_renderer::GlyphRun> {
    let (regular, bold) = font_paths();
    let font_bytes = std::fs::read(&regular).unwrap_or_else(|e| panic!("reading {regular}: {e}"));
    let bold_bytes = std::fs::read(&bold).unwrap_or_else(|e| panic!("reading {bold}: {e}"));
    let mut r = ZKiteRenderer::new(zkite_renderer::FontSet::sans_only(font_bytes, bold_bytes))
        .expect("Renderer::new");
    let root = r.text("text-7xl font-bold text-white tabular-nums", "0");
    r.layout(root, WIDTH as f32, HEIGHT as f32);
    let nodes = r.positioned(root, &Default::default());
    let n = nodes.iter().find(|n| n.node == root).expect("root node must be positioned");
    let (x, y, _w, _h) = n.rect;
    let runs = n.text.as_ref().expect("text leaf must have shaped runs");
    assert!(!runs.is_empty(), "the \"0\" glyph must actually shape to at least one run");
    runs.iter()
        .map(|run| zkite_renderer::GlyphRun {
            font: run.font.clone(),
            font_size: run.font_size,
            glyphs: run.glyphs.iter().map(|g| zkite_renderer::PositionedGlyph { id: g.id, x: g.x + x, y: g.y + y }).collect(),
        })
        .collect()
}

/// Builds the scene `paint_nodes_into_scene`/`paint_cpu_fallback` would build for a
/// one-node tree: a full-canvas background fill, then the glyph run on top — same
/// paint order (backgrounds first, then all text) as the real render thread, and the
/// same call shape (`scene.glyph_run(resources, &run.font)...fill_glyphs(...)`).
/// `resources` is whichever `Resources` this simulated redraw is about to render
/// with — mirrors `paint_gpu` building a *fresh* `Scene` every redraw
/// (`vello_hybrid::Scene::new(..)`) against the render thread's one persistent
/// `gpu.resources`, which is exactly the pairing this repro needs to get right to
/// reproduce (or fix) the bug faithfully rather than testing a shape that couldn't
/// occur in the real code.
fn build_scene(resources: &mut Resources, runs: &[zkite_renderer::GlyphRun]) -> Scene {
    let mut scene = Scene::new(WIDTH as u16, HEIGHT as u16);
    scene.set_paint(peniko::Color::from_rgba8(BG.0, BG.1, BG.2, BG.3));
    scene.fill_rect(&Rect::new(0.0, 0.0, WIDTH as f64, HEIGHT as f64));
    scene.set_paint(peniko::Color::from_rgba8(255, 255, 255, 255));
    for run in runs {
        let glyphs: Vec<glifo::Glyph> = run.glyphs.iter().map(|g| glifo::Glyph { id: g.id, x: g.x, y: g.y }).collect();
        scene.glyph_run(resources, &run.font).font_size(run.font_size).hint(true).fill_glyphs(glyphs.into_iter());
    }
    scene
}

fn main() {
    let instance = wgpu::Instance::default();
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::None,
        force_fallback_adapter: false,
        compatible_surface: None,
    }))
    .expect("request_adapter (need a Vulkan ICD on this host — lvp/llvmpipe software rasterizer is enough)");
    let info = adapter.get_info();
    println!("adapter: {} ({:?}, backend {:?})", info.name, info.device_type, info.backend);
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("repro-atlas-lifetime"),
        required_features: wgpu::Features::empty(),
        required_limits: adapter.limits(),
        ..Default::default()
    }))
    .expect("request_device");

    let runs = shape_zero_glyph();

    // Pass 1 — baseline: one Renderer, one Resources, freshly paired (the correct,
    // documented usage). The glyph must render.
    let mut resources_a = Resources::new();
    let mut renderer_a = Renderer::new(&device, &RenderTargetConfig { format: FORMAT, width: WIDTH, height: HEIGHT });
    let scene_1 = build_scene(&mut resources_a, &runs);
    let pixels_1 = render_and_readback(&device, &queue, &mut renderer_a, &mut resources_a, &scene_1);
    let ink_1 = count_ink_pixels(&pixels_1);
    println!("pass 1 (baseline, fresh Renderer+Resources pair): {ink_1} ink pixels");
    assert!(ink_1 > 50, "the \"0\" glyph must actually render on a freshly-paired Renderer+Resources (got {ink_1} ink pixels)");

    // Pass 2 — the one that matters: a NEW Renderer (simulating
    // gpu_harness::reconfigure() after a Command::Resize) against the SAME Resources
    // reused across it — exactly what render_thread.rs's Gpu struct does today
    // (hybrid_renderer is replaced, resources is not). A fresh Scene is built (a real
    // redraw always builds one), but against the same, already-primed resources_a,
    // whose glyph cache already considers this glyph resident from pass 1. If
    // vello_hybrid trusted that residency blindly, the new renderer's own empty atlas
    // texture would be sampled and the glyph would come back blank.
    let mut renderer_b = Renderer::new(&device, &RenderTargetConfig { format: FORMAT, width: WIDTH, height: HEIGHT });
    let scene_2 = build_scene(&mut resources_a, &runs);
    let pixels_2 = render_and_readback(&device, &queue, &mut renderer_b, &mut resources_a, &scene_2);
    let ink_2 = count_ink_pixels(&pixels_2);
    println!("pass 2 (new Renderer, REUSED Resources):        {ink_2} ink pixels");
    assert_eq!(
        ink_2, ink_1,
        "reusing a vello_hybrid::Resources across a newly-constructed Renderer must keep rendering \
         already-cached glyphs at full fidelity. It did when this guard was written (see this \
         crate's module doc); if this now fails, `gpu_harness::render_thread::reconfigure()` — \
         which rebuilds the Renderer on every surface resize but deliberately keeps one \
         long-lived Resources — has become unsound, and text will silently vanish on device \
         after any IME show/hide or rotation. Recreate `Gpu::resources` alongside \
         `Gpu::hybrid_renderer` there."
    );

    // Pass 3 — the control: a new Renderer *and* a new Resources, paired fresh,
    // exactly like pass 1. Establishes that pass 2's result is really about the
    // reuse and not about anything else drifting between passes.
    let mut resources_c = Resources::new();
    let mut renderer_c = Renderer::new(&device, &RenderTargetConfig { format: FORMAT, width: WIDTH, height: HEIGHT });
    let scene_3 = build_scene(&mut resources_c, &runs);
    let pixels_3 = render_and_readback(&device, &queue, &mut renderer_c, &mut resources_c, &scene_3);
    let ink_3 = count_ink_pixels(&pixels_3);
    println!("pass 3 (control: new Renderer, FRESH Resources):  {ink_3} ink pixels");
    assert_eq!(ink_3, ink_1, "a freshly-paired Renderer+Resources must match the baseline exactly");

    println!();
    println!("OK: a vello_hybrid::Resources may safely outlive the Renderer it was first used");
    println!("with — already-cached glyphs still render at full fidelity through a newly");
    println!("constructed Renderer. gpu_harness::render_thread::reconfigure()'s reuse of one");
    println!("long-lived Resources across per-resize Renderers is therefore sound.");
    println!("This was NOT ../PLAN.md §7.6's cause — see §7.14 for what actually was.");
}

/// Renders `scene` through `renderer`/`resources` into a fresh off-screen texture and
/// reads the result back to a plain RGBA byte vector — the headless equivalent of
/// `gpu_harness::render_thread::paint_gpu`'s `surface.get_current_texture()`/
/// `frame.present()` tail, with a texture-to-buffer copy standing in for presentation
/// (there is no display, and none is needed to inspect the pixels).
fn render_and_readback(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    renderer: &mut Renderer,
    resources: &mut Resources,
    scene: &Scene,
) -> Vec<u8> {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("repro-target"),
        size: wgpu::Extent3d { width: WIDTH, height: HEIGHT, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("repro-frame") });
    renderer
        .render(scene, resources, device, queue, &mut encoder, &RenderSize { width: WIDTH, height: HEIGHT }, &view, &TextureBindings::new())
        .expect("vello_hybrid render");

    // Bytes-per-row must be 256-aligned for a buffer<-texture copy — pad if needed,
    // then strip the padding back out below.
    let unpadded_bpr = WIDTH * 4;
    let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let padded_bpr = unpadded_bpr.div_ceil(align) * align;

    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("repro-readback"),
        size: (padded_bpr * HEIGHT) as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo { texture: &texture, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
        wgpu::TexelCopyBufferInfo {
            buffer: &readback,
            layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(padded_bpr), rows_per_image: Some(HEIGHT) },
        },
        wgpu::Extent3d { width: WIDTH, height: HEIGHT, depth_or_array_layers: 1 },
    );
    queue.submit([encoder.finish()]);

    let slice = readback.slice(..);
    slice.map_async(wgpu::MapMode::Read, |r| r.expect("buffer map failed"));
    device.poll(wgpu::PollType::wait_indefinitely()).expect("device.poll");

    let data = slice.get_mapped_range();
    let mut out = vec![0u8; (unpadded_bpr * HEIGHT) as usize];
    for row in 0..HEIGHT as usize {
        let src = &data[row * padded_bpr as usize..row * padded_bpr as usize + unpadded_bpr as usize];
        out[row * unpadded_bpr as usize..(row + 1) * unpadded_bpr as usize].copy_from_slice(src);
    }
    drop(data);
    readback.unmap();
    out
}

/// Counts pixels that are clearly not the background color — a crude but sufficient
/// "is any glyph ink actually visible" check for a scene that only ever contains the
/// background fill and one white glyph run.
fn count_ink_pixels(rgba: &[u8]) -> usize {
    rgba.chunks_exact(4)
        .filter(|px| {
            let (r, g, b) = (px[0], px[1], px[2]);
            r.abs_diff(BG.0) > 16 || g.abs_diff(BG.1) > 16 || b.abs_diff(BG.2) > 16
        })
        .count()
}
