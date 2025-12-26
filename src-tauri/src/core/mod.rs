//! Tauri-specific wrappers for insight-core.
//!
//! Provides a BootPhaseEmitter implementation for Tauri's AppHandle and
//! re-exports the core types.

use tauri::{AppHandle, Emitter};

// Re-export everything from insight-core
pub use insight_core::*;

/// Wrapper that implements BootPhaseEmitter for Tauri's AppHandle
pub struct TauriBootEmitter<'a>(pub &'a AppHandle);

impl<'a> insight_core::BootPhaseEmitter for TauriBootEmitter<'a> {
    fn emit_boot_phase(&self, phase: insight_core::BootPhase) {
        let _ = self.0.emit("boot-phase", phase);
    }
}
