//! The wgpu half of `Image` (../PLAN.md §35) — everything `zkite_imagestore` cannot do
//! without a GPU device.
//!
//! `zkite_imagestore` decodes and caches; this uploads. The split is not stylistic:
//! this crate does not build for the host (../PLAN.md §15.5), so every line that can
//! live in a testable crate does, and what is left here is the part that genuinely
//! needs a `Device`, a `Queue` and a `CommandEncoder`.
//!
//! # Why the GPU path needs an upload at all
//!
//! `vello_cpu` samples an `ImageSource::Pixmap` straight out of memory. `vello_hybrid`'s
//! wgpu backend does not: `render/wgpu.rs`'s image encoding matches only
//! `ImageSource::OpaqueId`, and a `Pixmap` handed to it is silently not drawn. So a
//! bitmap has to be written into the renderer's texture atlas first, and the
//! `ImageId` that comes back is what the scene actually refers to.

use std::sync::Arc;

use zkite_imagestore::ImageStore;

/// Uploads every image referenced by `nodes` that is decoded but not yet in the atlas.
///
/// Idempotent and cheap after the first frame: the common case is that every `src` is
/// already in `gpu_ids` and this allocates nothing and creates no encoder at all.
///
/// **The encoder is created lazily and submitted on its own**, rather than folded into
/// the frame's encoder. Two reasons, and the second is the one that bites. The frame
/// encoder is created *after* `get_current_texture()`, which can fail and drop the
/// whole frame — an upload batched into it would be discarded while `record_gpu_id`
/// had already recorded the handle, leaving an id pointing at an atlas slot that was
/// never written. And a frame with no new images should not pay for an encoder it
/// submits empty.
pub fn upload_pending(
    store: &mut ImageStore,
    nodes: &[zkite_renderer::PositionedNode],
    renderer: &mut vello_hybrid::Renderer,
    resources: &mut vello_hybrid::Resources,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
) {
    let mut encoder: Option<wgpu::CommandEncoder> = None;
    // Collected first so `store` is not borrowed across the upload calls, and
    // deduplicated by the `needs_upload` check plus the insert below — a list of 40
    // rows sharing one avatar uploads it once.
    let mut uploaded: Vec<(Arc<str>, vello_common::paint::ImageId)> = Vec::new();
    for n in nodes {
        let Some(src) = &n.image else { continue };
        if !store.needs_upload(src) || uploaded.iter().any(|(s, _)| s == src) {
            continue;
        }
        let Some(decoded) = store.get(src) else { continue };
        let enc = encoder.get_or_insert_with(|| {
            device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("zkite-image-upload"),
            })
        });
        let id = renderer.upload_image(resources, device, queue, enc, &decoded.pixmap);
        uploaded.push((src.clone(), id));
    }
    if let Some(enc) = encoder {
        queue.submit([enc.finish()]);
        for (src, id) in &uploaded {
            crate::logi!("images: uploaded {src} -> atlas id {}", id.as_u32());
        }
        for (src, id) in uploaded {
            store.record_gpu_id(src, id);
        }
        // Change-only, like the `frame:` line — this runs at most once per distinct
        // `src`, so it cannot land on the hot path.
        crate::logi!(
            "images: {} decoded, {} failed, {} resident",
            store.decodes,
            store.failures,
            store.resident()
        );
    }
}
