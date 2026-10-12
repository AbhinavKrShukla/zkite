//! Decoding and caching the bitmaps an `Image` leaf paints (../PLAN.md §35).
//!
//! `zkite_renderer` deliberately stops at the `src` string (see `Renderer::image`), so
//! everything from "a path" to "pixels a rasteriser can sample" lives here. That split
//! is the same one the whole phase draws between layout and paint.
//!
//! # Why this is its own crate
//!
//! Two reasons, and either alone would be enough. `gpu_harness` **does not build for
//! the host** — its `build.rs` compiles gametextinput's C++ against NDK headers — so
//! nothing placed in it can carry a test (../PLAN.md §15.5), and a decoder is exactly
//! the kind of code where an untested channel swap survives for months. And the
//! *conformance harness* needs the identical decode to render an image fixture with
//! `vello_cpu` on the host; sharing this crate is what stops the harness measuring a
//! second decoder against Chromium and calling the result ZKite's.
//!
//! # The two paint paths need different things from the same bitmap
//!
//! `vello_cpu` samples an `ImageSource::Pixmap(Arc<Pixmap>)` directly. `vello_hybrid`'s
//! wgpu backend **cannot**: its `encode_image` path matches only
//! `ImageSource::OpaqueId`, so a pixmap has to be uploaded into the renderer's texture
//! atlas first and referred to by the `ImageId` that comes back. One decode therefore
//! feeds two representations. This crate owns the decode and the `ImageId` *bookkeeping*
//! but never touches wgpu — the upload itself is `gpu_harness/src/images.rs`, which is
//! the only place a `device`/`queue`/`encoder` exists.
//!
//! # Failure is cached, and that is the point
//!
//! A missing file, a corrupt file and an unsupported format all produce `Entry::Failed`,
//! which is **remembered**. Without that, a typo'd `src` costs a failed `open()` on
//! every node, on every frame, at 60fps — and the app that most needs to stay smooth
//! is the one with the bug in it. A failed image paints nothing, so the node degrades
//! to the empty box `Image` has always drawn (../PLAN.md §15.1), rather than panicking
//! on a string that came from user code. Same call `Renderer::icon` makes for an
//! unknown sprite name.
//!
//! # Paths are confined to the app directory
//!
//! `src` comes from user Python, but a ZKite app is not necessarily the only thing that
//! wrote into `app_dir`, and `zkite dev` pushes files into it over a socket. `resolve`
//! rejects absolute paths and any path that escapes the root, so `Image(src="../../..
//! /data/data/<other app>/x.png")` cannot be used to read a file the app author did not
//! ship. Rejected paths take the ordinary `Failed` route — no panic, no log spam.

use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use rustc_hash::FxHashMap;
use vello_common::paint::ImageId;
use vello_common::pixmap::Pixmap;

/// A decoded bitmap, in the one representation both paint paths can start from.
pub struct Decoded {
    pub pixmap: Arc<Pixmap>,
    pub width: u32,
    pub height: u32,
}

enum Entry {
    Ready(Arc<Decoded>),
    /// Tried once, will not be tried again — see the module doc.
    Failed,
}

/// Everything an `Image` needs between its `src` and a paint call, warm across frames.
///
/// **Keyed by `src` exactly as written**, not by canonical path: two nodes with the
/// same `src` share one decode and one atlas slot, which is the case a list of rows
/// with the same avatar placeholder hits immediately. Two spellings of the same file
/// decode twice; that is a bounded waste and the alternative (canonicalising on every
/// lookup) is a syscall per node per frame.
pub struct ImageStore {
    root: PathBuf,
    cache: FxHashMap<Arc<str>, Entry>,
    /// Atlas handles for the GPU path. Separate from `cache` because it is *GPU
    /// lifetime* state: a lost surface drops the renderer and every `ImageId` in it,
    /// while the decoded pixmaps stay perfectly valid. `invalidate_gpu` exists for
    /// exactly that moment.
    ///
    /// Held here, rather than in `gpu_harness` beside the code that fills it, so that
    /// "is this already uploaded" and "is this already decoded" cannot drift apart —
    /// they are answered by two maps with one owner and one lifetime.
    gpu_ids: FxHashMap<Arc<str>, ImageId>,
    /// Counters, read by the `images:` log line and by the device measurements in
    /// ../PLAN.md §35.4 — the cheapest way to prove a cache is actually a cache.
    pub decodes: u64,
    pub failures: u64,
}

impl ImageStore {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        ImageStore {
            root: root.into(),
            cache: FxHashMap::default(),
            gpu_ids: FxHashMap::default(),
            decodes: 0,
            failures: 0,
        }
    }

    /// The decoded bitmap for `src`, decoding it on first use. `None` if it has failed
    /// (now or on any earlier frame).
    pub fn get(&mut self, src: &Arc<str>) -> Option<Arc<Decoded>> {
        if !self.cache.contains_key(src) {
            let entry = match load(&self.root, src) {
                Some(d) => {
                    self.decodes += 1;
                    Entry::Ready(Arc::new(d))
                }
                None => {
                    self.failures += 1;
                    Entry::Failed
                }
            };
            self.cache.insert(src.clone(), entry);
        }
        match self.cache.get(src) {
            Some(Entry::Ready(d)) => Some(d.clone()),
            _ => None,
        }
    }

    /// The atlas handle for `src` on the GPU path, if one has been uploaded.
    pub fn gpu_id(&self, src: &Arc<str>) -> Option<ImageId> {
        self.gpu_ids.get(src).copied()
    }

    /// Whether the GPU path still has to upload `src` this frame.
    pub fn needs_upload(&self, src: &Arc<str>) -> bool {
        !self.gpu_ids.contains_key(src)
    }

    /// Records the handle `vello_hybrid` returned for an uploaded pixmap.
    ///
    /// Called only after the upload's command encoder has been **submitted** — see
    /// `gpu_harness/src/images.rs`. Recording it earlier would leave an id here
    /// pointing at an atlas slot that was never written, if the frame is dropped
    /// between the two.
    pub fn record_gpu_id(&mut self, src: Arc<str>, id: ImageId) {
        self.gpu_ids.insert(src, id);
    }

    /// Gives up every atlas handle — returning them so the caller can free the slots —
    /// without touching the decoded pixmaps. The next frame re-uploads.
    ///
    /// **This is not defensive; it is load-bearing, and it was found on a device**
    /// (../PLAN.md §35.5). The atlas *texture* lives inside `vello_hybrid::Renderer`,
    /// while the residency *bookkeeping* here (and in `vello_hybrid::Resources`) lives
    /// beside it — and `gpu_harness::render_thread::reconfigure` builds a brand-new
    /// `Renderer` on every surface resize, **twice on every launch**, because Android
    /// delivers a `Resize` straight after `NewWindow`. So an `ImageId` recorded during
    /// the first frame refers to a texture that no longer exists by the second.
    ///
    /// The symptom is not a crash and not a missing image: the scene still resolves
    /// the id, samples a slot nothing ever wrote, and paints an **opaque black
    /// rectangle** exactly where the picture should be. Every image on the phone was
    /// black while the identical code was pixel-perfect against Chromium on the host,
    /// because the host harness renders one frame and never resizes.
    ///
    /// `phase_3/repro_atlas_lifetime` establishes that *glyphs* survive this, because
    /// `vello_hybrid` owns their residency and re-establishes it. Images are the case
    /// where the bookkeeping is ours, so the invalidation has to be ours too.
    ///
    /// Callers must pass the returned ids to `Renderer::destroy_image` **on the
    /// outgoing renderer, before it is dropped** — the slots are allocated in the
    /// shared `Resources`, which outlives the renderer, so dropping the handles
    /// without freeing them leaks an atlas slot per image per resize (and a resize is
    /// what an IME show/hide does).
    #[must_use = "the returned ids must be freed with Renderer::destroy_image or the atlas leaks"]
    pub fn drain_gpu_ids(&mut self) -> Vec<ImageId> {
        self.gpu_ids.drain().map(|(_, id)| id).collect()
    }

    /// Number of images currently held decoded — for the log line and the tests.
    pub fn resident(&self) -> usize {
        self.cache.values().filter(|e| matches!(e, Entry::Ready(_))).count()
    }
}

/// The two `src` forms ZKite understands, resolved to bytes and decoded.
///
/// A `data:` URI is supported because it is the **only** `src` that works identically
/// on both backends with no staging at all — the WebView decodes one natively — which
/// makes it the form the conformance harness can compare pixel-for-pixel without
/// teaching the harness where an app directory is. It is also what a caller who has
/// just fetched an image over the network already holds.
///
/// Everything else is a path relative to the app directory.
fn load(root: &Path, src: &str) -> Option<Decoded> {
    if let Some(rest) = src.strip_prefix("data:") {
        // `data:[<mime>][;base64],<payload>`. Only the base64 form is accepted: a
        // percent-encoded binary PNG is legal in the grammar and is not something any
        // real caller produces, and guessing wrong there yields a corrupt decode
        // rather than an error.
        let (meta, payload) = rest.split_once(',')?;
        if !meta.ends_with(";base64") {
            return None;
        }
        return decode_bytes(&base64_decode(payload)?);
    }
    let path = resolve(root, src)?;
    decode_bytes(&std::fs::read(path).ok()?)
}

/// Standard base64, no line breaks, `=` padding optional.
///
/// Hand-rolled rather than adding a crate: it is fifteen lines, this is the only
/// caller, and a dependency here would also have to cross-compile to four Android
/// ABIs. Rejects any character outside the alphabet instead of skipping it, so a
/// truncated or mangled URI fails rather than decoding to plausible garbage.
fn base64_decode(s: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(s.len() / 4 * 3);
    let (mut acc, mut bits) = (0u32, 0u32);
    for b in s.bytes() {
        let v = match b {
            b'A'..=b'Z' => b - b'A',
            b'a'..=b'z' => b - b'a' + 26,
            b'0'..=b'9' => b - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            b'=' => break,
            _ => return None,
        } as u32;
        acc = (acc << 6) | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    Some(out)
}

/// Turns a user-supplied `src` into a path inside `root`, or `None` if it tries to
/// leave (see the module doc).
///
/// Rejected, deliberately: absolute paths, anything containing a `..` that escapes,
/// Windows prefixes and root components. A `.` component is harmless and allowed, so
/// `./logo.png` — which is how a lot of people write it — works.
///
/// **This is a lexical check, not a resolved one**, so a *symlink* inside `root`
/// pointing outside it would still be followed. That is accepted rather than
/// overlooked: `app_dir`'s contents come from this framework's own zip unpack and from
/// `zkite dev`'s socket push, neither of which creates symlinks, and canonicalising
/// would cost a syscall per node per frame on the path that has to stay at 60fps.
/// Recorded in ../PLAN.md §35.6 so it is a known boundary rather than an assumption.
fn resolve(root: &Path, src: &str) -> Option<PathBuf> {
    if src.is_empty() {
        return None;
    }
    let candidate = Path::new(src);
    let mut depth = 0i32;
    for c in candidate.components() {
        match c {
            Component::Normal(_) => depth += 1,
            Component::CurDir => {}
            Component::ParentDir => {
                depth -= 1;
                // Checked *as we walk* rather than on the final depth, so
                // `a/../../b` is rejected even though it ends at depth 0.
                if depth < 0 {
                    return None;
                }
            }
            Component::RootDir | Component::Prefix(_) => return None,
        }
    }
    Some(root.join(candidate))
}

/// Decodes one image, sniffing the format from its magic bytes rather than from a
/// filename or a `data:` URI's declared MIME type — both are things a human types, a
/// `.png` that is really a JPEG is what happens when someone renames a download, and
/// the bytes cannot be wrong about themselves.
fn decode_bytes(bytes: &[u8]) -> Option<Decoded> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        decode_png(bytes)
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        decode_jpeg(bytes)
    } else {
        None
    }
}

fn decode_png(bytes: &[u8]) -> Option<Decoded> {
    // Wrapped in a `Cursor`: png 0.18's `Decoder` requires `Read + Seek`, and a bare
    // `&[u8]` gives it only `Read`.
    let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    // png defaults to IDENTITY: packed grayscale remains packed, palettes remain
    // indices, and 16-bit samples remain two bytes. Expand before interpreting any
    // channels, including tRNS transparency on RGB and grayscale inputs.
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().ok()?;
    checked_pixel_count(reader.info().width, reader.info().height)?;
    // `output_buffer_size` is `None` for a header whose dimensions overflow `usize` —
    // a corrupt file, so it takes the ordinary failure route rather than panicking.
    let mut buf = vec![0; reader.output_buffer_size()?];
    let info = reader.next_frame(&mut buf).ok()?;
    let (w, h) = (info.width, info.height);
    let rgba: Vec<[u8; 4]> = match info.color_type {
        png::ColorType::Rgba => buf[..info.buffer_size()].chunks_exact(4).map(|c| [c[0], c[1], c[2], c[3]]).collect(),
        png::ColorType::Rgb => buf[..info.buffer_size()].chunks_exact(3).map(|c| [c[0], c[1], c[2], 255]).collect(),
        png::ColorType::Grayscale => buf[..info.buffer_size()].iter().map(|&g| [g, g, g, 255]).collect(),
        png::ColorType::GrayscaleAlpha => {
            buf[..info.buffer_size()].chunks_exact(2).map(|c| [c[0], c[0], c[0], c[1]]).collect()
        }
        // `read_info` expands a palette to RGB/RGBA, so this arm is unreachable in
        // practice; returning `None` rather than `unreachable!()` keeps a future png
        // release that changes that from panicking inside a render thread.
        png::ColorType::Indexed => return None,
    };
    to_decoded(w, h, &rgba)
}

fn decode_jpeg(bytes: &[u8]) -> Option<Decoded> {
    use zune_jpeg::zune_core::{colorspace::ColorSpace, options::DecoderOptions};

    // `ZCursor`, not the slice directly: `JpegDecoder::new` takes anything
    // implementing `ZByteReaderTrait`, and a bare `&[u8]` does not (it has no `Seek`).
    // `ZCursor` is zune's own in-memory adapter for exactly this.
    // State the output format explicitly for both grayscale and color inputs.
    let options = DecoderOptions::default().jpeg_set_out_colorspace(ColorSpace::RGB);
    let mut decoder = zune_jpeg::JpegDecoder::new_with_options(
        zune_jpeg::zune_core::bytestream::ZCursor::new(bytes), options);
    decoder.decode_headers().ok()?;
    let (w, h) = decoder.dimensions()?;
    let count = checked_pixel_count(u32::try_from(w).ok()?, u32::try_from(h).ok()?)?;
    // JPEG has no alpha channel at all, so RGB out and 255 in.
    let pixels = decoder.decode().ok()?;
    if pixels.len() != count.checked_mul(3)? {
        return None;
    }
    let rgba: Vec<[u8; 4]> = pixels.chunks_exact(3).map(|c| [c[0], c[1], c[2], 255]).collect();
    to_decoded(w as u32, h as u32, &rgba)
}

/// A single decoded RGBA image is limited to 64 MiB. Check headers before asking
/// either decoder for a pixel allocation; a compressed image can be tiny on disk.
/// Larger assets should be resized for display before bundling them with the app.
const MAX_IMAGE_PIXELS: usize = 16 * 1024 * 1024;

fn checked_pixel_count(w: u32, h: u32) -> Option<usize> {
    if w == 0 || h == 0 || w > u16::MAX as u32 || h > u16::MAX as u32 {
        return None;
    }
    let pixels = (w as usize).checked_mul(h as usize)?;
    (pixels <= MAX_IMAGE_PIXELS).then_some(pixels)
}

/// Straight (unpremultiplied) RGBA8 -> the premultiplied `Pixmap` both vello backends
/// want.
///
/// **Premultiplication is not optional here.** `vello_common::Pixmap` is documented as
/// premultiplied, and handing it straight alpha renders a haloed, too-bright edge
/// wherever a PNG is partially transparent — which is every antialiased logo edge, i.e.
/// exactly the case a user notices first and cannot describe.
fn to_decoded(w: u32, h: u32, rgba: &[[u8; 4]]) -> Option<Decoded> {
    use vello_common::color::PremulRgba8;
    // Never clamp dimensions: doing so crops a flat buffer with the wrong row
    // stride. Validate the complete shape before Vello's asserting constructor.
    let pixels = checked_pixel_count(w, h)?;
    if rgba.len() != pixels {
        return None;
    }
    let mut may_have_transparency = false;
    let buf: Vec<PremulRgba8> = rgba
        .iter()
        .map(|&[r, g, b, a]| {
            may_have_transparency |= a != 255;
            let m = |c: u8| ((u16::from(a) * u16::from(c)) / 255) as u8;
            PremulRgba8 { r: m(r), g: m(g), b: m(b), a }
        })
        .collect();
    let pixmap = Pixmap::from_parts_with_opacity(buf, w as u16, h as u16, may_have_transparency);
    Some(Decoded { pixmap: Arc::new(pixmap), width: w, height: h })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn encoded_png(color: png::ColorType, depth: png::BitDepth, pixels: &[u8], palette: Option<&[u8]>, alpha: Option<&[u8]>) -> Vec<u8> {
        let mut bytes = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut bytes, 2, 1);
            encoder.set_color(color);
            encoder.set_depth(depth);
            if let Some(palette) = palette {
                encoder.set_palette(palette);
            }
            if let Some(alpha) = alpha {
                encoder.set_trns(alpha);
            }
            encoder.write_header().unwrap().write_image_data(pixels).unwrap();
        }
        bytes
    }

    #[test]
    fn indexed_png_expands_its_palette_and_transparency() {
        let bytes = encoded_png(png::ColorType::Indexed, png::BitDepth::Eight,
            &[0, 1], Some(&[255, 0, 0, 0, 255, 0]), Some(&[255, 128]));
        let decoded = decode_bytes(&bytes).expect("ordinary paletted PNG must decode");
        let pixels = decoded.pixmap.data();
        assert_eq!((pixels[0].r, pixels[0].g, pixels[0].b, pixels[0].a), (255, 0, 0, 255));
        assert_eq!((pixels[1].r, pixels[1].g, pixels[1].b, pixels[1].a), (0, 128, 0, 128));
    }

    #[test]
    fn one_bit_grayscale_png_expands_before_pixmap_construction() {
        let bytes = encoded_png(png::ColorType::Grayscale, png::BitDepth::One,
            &[0b0100_0000], None, None);
        let decoded = decode_bytes(&bytes).expect("packed grayscale PNG must decode");
        assert_eq!(decoded.pixmap.data()[0].r, 0);
        assert_eq!(decoded.pixmap.data()[1].r, 255);
    }

    #[test]
    fn sixteen_bit_png_uses_high_bytes_without_interleaving_channels() {
        let bytes = encoded_png(png::ColorType::Rgb, png::BitDepth::Sixteen,
            &[255, 255, 0, 0, 0, 0, 0, 0, 255, 255, 0, 0], None, None);
        let decoded = decode_bytes(&bytes).expect("16-bit RGB PNG must decode");
        let pixels = decoded.pixmap.data();
        assert_eq!((pixels[0].r, pixels[0].g, pixels[0].b), (255, 0, 0));
        assert_eq!((pixels[1].r, pixels[1].g, pixels[1].b), (0, 255, 0));
    }

    #[test]
    fn invalid_dimensions_and_short_buffers_fail_without_panicking() {
        assert!(to_decoded(2, 1, &[[255, 0, 0, 255]]).is_none());
        assert!(to_decoded(1, 1, &[[255, 0, 0, 255]; 2]).is_none());
        for (w, h) in [(0, 1), (1, 0), (65536, 1), (1, 65536), (8192, 8192), (u32::MAX, u32::MAX)] {
            assert!(checked_pixel_count(w, h).is_none(), "{w}x{h} must fail before allocation");
        }
        assert_eq!(checked_pixel_count(4096, 4096), Some(MAX_IMAGE_PIXELS));
    }

    #[test]
    fn grayscale_jpeg_produces_three_equal_channels_and_opaque_alpha() {
        let decoded = decode_bytes(include_bytes!("../tests/fixtures/gray_8x8.jpg"))
            .expect("one-component JPEG must decode");
        assert_eq!((decoded.width, decoded.height), (8, 8));
        for pixel in decoded.pixmap.data() {
            assert_eq!((pixel.r, pixel.g, pixel.b, pixel.a), (96, 96, 96, 255));
        }
    }

    #[test]
    fn rgb_jpeg_preserves_channel_order_and_opaque_alpha() {
        let decoded = decode_bytes(include_bytes!("../tests/fixtures/rgb_8x8.jpg"))
            .expect("color JPEG must decode");
        assert_eq!((decoded.width, decoded.height), (8, 8));
        for pixel in decoded.pixmap.data() {
            for (actual, expected) in [(pixel.r, 220u8), (pixel.g, 40), (pixel.b, 80)] {
                assert!(actual.abs_diff(expected) <= 2, "{actual} != {expected}");
            }
            assert_eq!(pixel.a, 255);
        }
    }

    #[test]
    fn a_src_cannot_escape_the_app_directory() {
        let root = Path::new("/data/app");
        // Ordinary, and the `./` spelling people actually write.
        assert_eq!(resolve(root, "logo.png"), Some(PathBuf::from("/data/app/logo.png")));
        assert_eq!(resolve(root, "./img/logo.png"), Some(PathBuf::from("/data/app/./img/logo.png")));
        assert_eq!(resolve(root, "img/../logo.png"), Some(PathBuf::from("/data/app/img/../logo.png")));
        // Every one of these must be refused.
        for bad in [
            "",
            "/etc/passwd",
            "../secrets.png",
            "../../data/data/com.other.app/x.png",
            "img/../../escape.png",
            // Ends at depth 0 but *passes through* -1 on the way, which a check on
            // the final depth alone would let through.
            "a/../../b/logo.png",
        ] {
            assert_eq!(resolve(root, bad), None, "{bad:?} should be rejected");
        }
    }

    /// A failed lookup is remembered, so a typo'd `src` costs one filesystem hit
    /// rather than one per frame.
    #[test]
    fn a_failed_image_is_attempted_exactly_once() {
        let dir = std::env::temp_dir().join("zkite-imagestore-miss");
        let mut store = ImageStore::new(&dir);
        let src: Arc<str> = Arc::from("nope.png");
        for _ in 0..5 {
            assert!(store.get(&src).is_none());
        }
        assert_eq!(store.failures, 1, "five lookups, one attempt");
        assert_eq!(store.resident(), 0);
    }

    /// The cache half of the same claim, plus the sharing that makes a list of rows
    /// with one repeated image cost one decode.
    #[test]
    fn a_decoded_image_is_decoded_once_and_shared() {
        let dir = std::env::temp_dir().join("zkite-imagestore-hit");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("solid.png");
        std::fs::write(&path, png_2x1_red_green()).unwrap();

        let mut store = ImageStore::new(&dir);
        let src: Arc<str> = Arc::from("solid.png");
        let first = store.get(&src).expect("decodes");
        for _ in 0..4 {
            let again = store.get(&src).expect("stays decoded");
            assert!(Arc::ptr_eq(&first, &again), "every lookup shares one decode");
        }
        assert_eq!(store.decodes, 1, "five lookups, one decode");
        assert_eq!(store.resident(), 1);
        assert_eq!((first.width, first.height), (2, 1));
    }

    /// Decoding is checked against *known pixel values*, not just against "it did not
    /// error" — a decoder that returned a plausible-sized buffer of zeroes would pass
    /// every other test in this file.
    ///
    /// Also pins the channel order (a red pixel must be red, not blue), which is the
    /// mistake that produces a picture that looks right until someone photographs a
    /// sunset.
    #[test]
    fn decoding_produces_the_expected_pixels_in_the_expected_channel_order() {
        let dir = std::env::temp_dir().join("zkite-imagestore-pixels");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("rg.png"), png_2x1_red_green()).unwrap();
        let mut store = ImageStore::new(&dir);
        let d = store.get(&Arc::from("rg.png")).expect("decodes");
        let px = d.pixmap.data();
        assert_eq!((px[0].r, px[0].g, px[0].b, px[0].a), (255, 0, 0, 255), "pixel 0 is red");
        assert_eq!((px[1].r, px[1].g, px[1].b, px[1].a), (0, 255, 0, 255), "pixel 1 is green");
    }

    /// Half-transparent white must premultiply to half-grey, not stay 255.
    ///
    /// This is the one decode property with no visible symptom on an opaque image and
    /// a very visible one on every antialiased logo edge, so it gets its own test with
    /// a hand-computed expectation rather than being covered incidentally.
    #[test]
    fn alpha_is_premultiplied_on_the_way_in() {
        let straight = [[255u8, 255, 255, 128]];
        let d = to_decoded(1, 1, &straight).expect("valid one-pixel buffer");
        let p = d.pixmap.data()[0];
        // (128 * 255) / 255 == 128, exactly.
        assert_eq!((p.r, p.g, p.b, p.a), (128, 128, 128, 128));
        assert!(d.pixmap.may_have_transparency());
    }

    /// A file that is not an image at all, and one whose bytes are truncated
    /// mid-stream, both fail rather than panicking inside the render thread.
    #[test]
    fn a_non_image_and_a_truncated_image_both_fail_cleanly() {
        let dir = std::env::temp_dir().join("zkite-imagestore-junk");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("notes.txt"), b"this is not a png").unwrap();
        let good = png_2x1_red_green();
        std::fs::write(dir.join("cut.png"), &good[..good.len() / 2]).unwrap();
        let mut store = ImageStore::new(&dir);
        assert!(store.get(&Arc::from("notes.txt")).is_none());
        assert!(store.get(&Arc::from("cut.png")).is_none());
        assert_eq!(store.failures, 2);
    }

    /// A `data:` URI decodes to the same pixels the file form does — which is what
    /// lets the conformance harness compare ZKite's decode against Chromium's on
    /// identical bytes, with no app directory involved (../PLAN.md §35.4).
    #[test]
    fn a_data_uri_decodes_to_the_same_pixels_as_the_file_form() {
        let dir = std::env::temp_dir().join("zkite-imagestore-datauri");
        std::fs::create_dir_all(&dir).unwrap();
        let bytes = png_2x1_red_green();
        std::fs::write(dir.join("rg.png"), &bytes).unwrap();

        let mut store = ImageStore::new(&dir);
        let from_file = store.get(&Arc::from("rg.png")).expect("file decodes");
        let uri: Arc<str> = Arc::from(format!("data:image/png;base64,{}", b64(&bytes)).as_str());
        let from_uri = store.get(&uri).expect("data uri decodes");

        assert_eq!((from_uri.width, from_uri.height), (from_file.width, from_file.height));
        assert_eq!(from_uri.pixmap.data(), from_file.pixmap.data(), "identical bytes in, identical pixels out");
        // Two different `src` strings, so two decodes — the cache is keyed by `src`
        // as written and makes no attempt to notice they are the same image.
        assert_eq!(store.decodes, 2);
    }

    /// The `data:` forms that must be refused rather than decoded into garbage.
    ///
    /// The non-base64 arm matters most: `data:image/png,<percent-encoded>` is legal in
    /// the grammar, and treating its payload as base64 yields a plausible-length
    /// buffer of the wrong bytes — a corrupt image rather than an error.
    #[test]
    fn malformed_data_uris_are_refused() {
        let mut store = ImageStore::new(std::env::temp_dir());
        let good = b64(&png_2x1_red_green());
        for bad in [
            "data:image/png,notbase64".to_string(),
            "data:image/png;base64".to_string(), // no comma at all
            // Truncated into the middle of `IDAT`. **Half, not a few characters**:
            // clipping only the tail still decodes, because a PNG whose `IEND` is
            // missing but whose image data is complete is accepted by the `png`
            // crate. Worth knowing — "the file was cut short" is not by itself a
            // decode failure, so a partial download can yield a real image.
            format!("data:image/png;base64,{}", &good[..good.len() / 2]),
            // Junk *inside* the payload. It has to go before the `=` padding to be
            // reachable at all: the decoder stops at the first `=`, as every base64
            // decoder does, so trailing garbage is invisible to it. The first version
            // of this case appended `!!!` to the end and passed for that reason
            // rather than for the reason intended — recorded per ../PLAN.md's standing
            // rule that a control which does not discriminate is worth writing down.
            format!("data:image/png;base64,{}!!!{}", &good[..8], &good[8..]),
            "data:,".to_string(),
        ] {
            assert!(store.get(&Arc::from(bad.as_str())).is_none(), "{bad:?} should be refused");
        }
    }

    /// Standard base64 for the tests above — again written here rather than pulled in,
    /// so a bug shared between encoder and decoder cannot hide.
    fn b64(bytes: &[u8]) -> String {
        const A: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut out = String::new();
        for chunk in bytes.chunks(3) {
            let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
            let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
            for i in 0..4 {
                if i <= chunk.len() {
                    out.push(A[((n >> (18 - 6 * i)) & 63) as usize] as char);
                } else {
                    out.push('=');
                }
            }
        }
        out
    }

    /// A 2x1 PNG: one opaque red pixel, one opaque green one. Written by hand so the
    /// decode tests above depend on no encoder of ours — a bug shared between a writer
    /// and a reader is invisible to a round-trip test.
    fn png_2x1_red_green() -> Vec<u8> {
        fn chunk(tag: &[u8], data: &[u8]) -> Vec<u8> {
            let mut out = (data.len() as u32).to_be_bytes().to_vec();
            out.extend_from_slice(tag);
            out.extend_from_slice(data);
            out.extend_from_slice(&crc32(&[tag, data].concat()).to_be_bytes());
            out
        }
        fn crc32(bytes: &[u8]) -> u32 {
            let mut table = [0u32; 256];
            for (i, e) in table.iter_mut().enumerate() {
                let mut c = i as u32;
                for _ in 0..8 {
                    c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
                }
                *e = c;
            }
            let mut c = 0xFFFF_FFFFu32;
            for &b in bytes {
                c = table[((c ^ b as u32) & 0xFF) as usize] ^ (c >> 8);
            }
            c ^ 0xFFFF_FFFF
        }
        // width 2, height 1, bit depth 8, colour type 6 (RGBA), no interlace.
        let ihdr = [0, 0, 0, 2, 0, 0, 0, 1, 8, 6, 0, 0, 0];
        // One scanline: filter 0, then two RGBA pixels.
        let raw = [0u8, 255, 0, 0, 255, 0, 255, 0, 255];
        // Stored (uncompressed) deflate block inside a zlib wrapper, so this needs no
        // compressor: 0x78 0x01, then one final stored block, then adler32.
        let mut z = vec![0x78, 0x01, 0x01, raw.len() as u8, 0x00, !(raw.len() as u8), 0xFF];
        z.extend_from_slice(&raw);
        let (mut a, mut b) = (1u32, 0u32);
        for &byte in raw.iter() {
            a = (a + byte as u32) % 65521;
            b = (b + a) % 65521;
        }
        z.extend_from_slice(&(((b << 16) | a) as u32).to_be_bytes());

        let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
        out.extend(chunk(b"IHDR", &ihdr));
        out.extend(chunk(b"IDAT", &z));
        out.extend(chunk(b"IEND", &[]));
        out
    }
}
