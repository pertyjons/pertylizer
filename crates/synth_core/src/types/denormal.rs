//! Denormal prevention via CPU flags (FTZ/DAZ).
//!
//! Denormal floating-point numbers cause severe performance degradation in
//! filter processing. Setting hardware flush-to-zero flags eliminates this
//! at the CPU level, making per-sample `flush_denormals()` calls unnecessary.

/// RAII guard that sets flush-to-zero CPU flags on creation and restores on drop.
///
/// Create this at the start of the audio callback to prevent denormal numbers
/// from causing performance issues in all filter processing.
///
/// # Platform support
///
/// - **x86_64**: Sets FTZ+DAZ via MXCSR register
/// - **aarch64**: Sets FZ via FPCR register
/// - **Other**: No-op (use `FilterState::flush_denormals()` as fallback)
///
/// # Example
/// ```ignore
/// fn audio_callback(buffer: &mut [f32]) {
///     let _guard = DenormalGuard::new();
///     // All floating-point operations in this scope are denormal-free
/// }
/// ```
#[must_use]
pub struct DenormalGuard {
    #[cfg(target_arch = "x86_64")]
    previous_mxcsr: u32,
    #[cfg(target_arch = "aarch64")]
    previous_fpcr: u64,
    #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
    _phantom: (),
}

impl DenormalGuard {
    /// Create a new denormal guard, setting flush-to-zero flags.
    #[inline]
    pub fn new() -> Self {
        core::cfg_select! {
            target_arch = "x86_64" => {
                let mut previous: u32 = 0;
                // Safety: reading/writing MXCSR is safe on x86_64 — thread-local,
                // no side effects beyond FP behavior. RAII guarantees restore.
                unsafe { core::arch::asm!("stmxcsr [{}]", in(reg) &mut previous, options(nostack)) };
                let new_val = previous | (1 << 15) | (1 << 6); // FTZ + DAZ
                unsafe {
                    core::arch::asm!("ldmxcsr [{}]", in(reg) &new_val, options(nostack, readonly))
                };
                Self {
                    previous_mxcsr: previous,
                }
            }
            target_arch = "aarch64" => {
                let previous: u64;
                // Safety: reading/writing FPCR is safe on aarch64 — thread-local,
                // no side effects beyond FP behavior. RAII guarantees restore.
                unsafe { core::arch::asm!("mrs {}, fpcr", out(reg) previous) };
                // FZ = bit 24
                unsafe { core::arch::asm!("msr fpcr, {}", in(reg) previous | (1 << 24)) };
                Self {
                    previous_fpcr: previous,
                }
            }
            _ => {
                Self { _phantom: () }
            }
        }
    }
}

impl Drop for DenormalGuard {
    #[inline]
    fn drop(&mut self) {
        core::cfg_select! {
            target_arch = "x86_64" => {
                // Safety: restoring previously saved MXCSR value.
                unsafe {
                    core::arch::asm!("ldmxcsr [{}]", in(reg) &self.previous_mxcsr, options(nostack, readonly))
                };
            }
            target_arch = "aarch64" => {
                // Safety: restoring previously saved FPCR value.
                unsafe { core::arch::asm!("msr fpcr, {}", in(reg) self.previous_fpcr) };
            }
            _ => {}
        }
    }
}

impl Default for DenormalGuard {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_denormal_guard_creates_and_drops() {
        let guard = DenormalGuard::new();
        drop(guard);
    }

    #[test]
    #[cfg(target_arch = "x86_64")]
    fn test_denormal_guard_sets_ftz_daz() {
        let mut before: u32 = 0;
        unsafe { core::arch::asm!("stmxcsr [{}]", in(reg) &mut before, options(nostack)) };

        let _guard = DenormalGuard::new();

        let mut during: u32 = 0;
        unsafe { core::arch::asm!("stmxcsr [{}]", in(reg) &mut during, options(nostack)) };
        assert_ne!(during & (1 << 15), 0, "FTZ should be set");
        assert_ne!(during & (1 << 6), 0, "DAZ should be set");

        drop(_guard);

        let mut after: u32 = 0;
        unsafe { core::arch::asm!("stmxcsr [{}]", in(reg) &mut after, options(nostack)) };
        assert_eq!(after, before, "MXCSR should be restored after drop");
    }

    #[test]
    fn test_denormal_guard_prevents_denormals() {
        let _guard = DenormalGuard::new();
        // Use black_box to prevent compile-time constant folding
        let denormal = std::hint::black_box(1.0e-40_f32);
        let result = denormal * std::hint::black_box(0.1);
        #[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
        assert_eq!(result, 0.0, "Denormal should be flushed to zero");
        #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
        let _ = result;
    }
}
