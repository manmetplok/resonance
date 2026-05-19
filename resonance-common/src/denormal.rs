/// Flush denormals to zero to prevent CPU spikes in audio processing.
///
/// Denormal floating-point numbers can cause massive slowdowns in FPU operations.
/// This sets the FTZ (Flush-To-Zero) and DAZ (Denormals-Are-Zero) bits on x86/x86_64,
/// or the FZ bit on aarch64.
#[inline]
pub fn flush_denormals() {
    // SAFETY (x86_64 / x86): we save the current MXCSR via stmxcsr,
    // OR in 0x8040 (bit 15 = FTZ, bit 6 = DAZ), and ldmxcsr it back.
    // The stack scratch uses `sub rsp, 4` / `add rsp, 4` around the
    // four bytes we wrote, so the call frame is restored before
    // returning. `preserves_flags` is correct: no instruction here
    // modifies arithmetic flags (stmxcsr/ldmxcsr/or-mem are all
    // flag-preserving on MXCSR loads). The 32-bit variant mirrors
    // this with `esp`. The whole sequence touches only the SSE
    // control register and four bytes of red-zone-equivalent stack
    // scratch, so no caller-observable state changes.
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
    // SAFETY (aarch64): `mrs ..., fpcr` reads the floating-point
    // control register into a general-purpose reg; `msr fpcr, ...`
    // writes it back. Both are system-register access instructions
    // that the kernel virtualises per-thread; they affect only this
    // thread's FP control bits. Setting bit 24 (FZ) flushes
    // denormal results to zero. No memory is touched and no flags
    // are modified.
    #[cfg(target_arch = "aarch64")]
    unsafe {
        let mut fpcr: u64;
        core::arch::asm!("mrs {}, fpcr", out(reg) fpcr);
        core::arch::asm!("msr fpcr, {}", in(reg) fpcr | (1 << 24));
    }
}
