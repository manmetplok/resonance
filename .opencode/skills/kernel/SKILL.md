---
name: kernel
description: "Use for OS/kernel/bare-metal/no_std development — memory, drivers, interrupts, boot."
---

# Kernel / bare-metal

Freestanding systems code: no OS, no std.

## Setup
- `#![no_std]` + `#![no_main]`; provide a `panic_handler` and entry point; use a target spec / linker script.
- No heap until an allocator is initialized; prefer stack and static allocation early in boot.

## Practices
- MMIO via `read_volatile`/`write_volatile`; never let the compiler reorder/elide device access.
- Minimize `unsafe`; document each block with `// SAFETY:`. Keep unsafe at the lowest layer behind safe APIs.
- Interrupts: set up IDT/handlers carefully; keep ISRs short; mind reentrancy.
- Concurrency: atomics and spinlocks (not `std::sync`); disable interrupts around critical sections when needed.
- Test in an emulator (e.g. QEMU); add unit tests for pure logic where possible.
