//! Surface validity crosses the Android UI thread and the render command queue.
//! Invalidate immediately on destruction, even while Python/GPU startup is busy.
use std::sync::atomic::{AtomicU64, Ordering};

pub struct SurfaceLifecycle(AtomicU64);

impl SurfaceLifecycle {
    pub const fn new() -> Self { Self(AtomicU64::new(0)) }

    pub fn created(&self) -> u64 { self.0.fetch_add(1, Ordering::SeqCst) + 1 }

    pub fn destroyed(&self) { self.0.fetch_add(1, Ordering::SeqCst); }

    pub fn is_current(&self, generation: u64) -> bool {
        generation != 0 && self.0.load(Ordering::SeqCst) == generation
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn destruction_invalidates_a_window_before_its_queued_command_is_handled() {
        let life = SurfaceLifecycle::new();
        assert!(!life.is_current(0));
        let first = life.created();
        life.destroyed();
        assert!(!life.is_current(first));
        let replacement = life.created();
        assert!(life.is_current(replacement));
        assert!(!life.is_current(first));
    }

    #[test]
    fn a_new_surface_invalidates_older_surface_commands() {
        let life = SurfaceLifecycle::new();
        let old = life.created();
        let current = life.created();
        assert!(!life.is_current(old));
        assert!(life.is_current(current));
    }

    #[test]
    fn invalidation_during_slow_gpu_initialization_is_visible_without_draining_commands() {
        let life = std::sync::Arc::new(SurfaceLifecycle::new());
        let generation = life.created();
        let (tx, rx) = std::sync::mpsc::channel();
        let ui_life = life.clone();
        let ui = std::thread::spawn(move || {
            tx.send("resize").unwrap();
            tx.send("input").unwrap();
            ui_life.destroyed();
            tx.send("destroy").unwrap();
        });
        ui.join().unwrap();
        assert!(!life.is_current(generation));
        assert_eq!(rx.try_iter().collect::<Vec<_>>(), ["resize", "input", "destroy"]);
    }
}
