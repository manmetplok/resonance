/// Flush denormals to zero to prevent CPU spikes in audio processing.
///
/// Denormal floating-point numbers can cause massive slowdowns in FPU operations.
/// This sets the FTZ (Flush-To-Zero) and DAZ (Denormals-Are-Zero) bits on x86/x86_64.
#[inline]
pub fn flush_denormals() {
    #[cfg(target_arch = "x86_64")]
    unsafe {
        std::arch::x86_64::_mm_setcsr(std::arch::x86_64::_mm_getcsr() | 0x8040);
    }
    #[cfg(target_arch = "x86")]
    unsafe {
        std::arch::x86::_mm_setcsr(std::arch::x86::_mm_getcsr() | 0x8040);
    }
}
