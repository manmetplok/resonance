/// Flush denormals to zero to prevent CPU spikes in audio processing.
///
/// Denormal floating-point numbers can cause massive slowdowns in FPU operations.
/// This sets the FTZ (Flush-To-Zero) and DAZ (Denormals-Are-Zero) bits on x86/x86_64,
/// or the FZ bit on aarch64.
#[inline]
pub fn flush_denormals() {
    #[cfg(target_arch = "x86_64")]
    unsafe {
        core::arch::asm!(
            "sub rsp, 4",
            "stmxcsr [rsp]",
            "or dword ptr [rsp], 0x8040",
            "ldmxcsr [rsp]",
            "add rsp, 4",
            options(preserves_flags),
        );
    }
    #[cfg(target_arch = "x86")]
    unsafe {
        core::arch::asm!(
            "sub esp, 4",
            "stmxcsr [esp]",
            "or dword ptr [esp], 0x8040",
            "ldmxcsr [esp]",
            "add esp, 4",
            options(preserves_flags),
        );
    }
    #[cfg(target_arch = "aarch64")]
    unsafe {
        let mut fpcr: u64;
        core::arch::asm!("mrs {}, fpcr", out(reg) fpcr);
        core::arch::asm!("msr fpcr, {}", in(reg) fpcr | (1 << 24));
    }
}
