# RFC: SNlang Bare-Metal Kernel & Freestanding Architecture
**Author:** Asish Kumar Sharma (SafarNow)  
**Status:** Approved Formal RFC  
**Target Subsystems:** `compiler/`, `runtime/`, `kernel/`  
**Related Specs:**
- [FREESTANDING_KERNEL_SPEC.md](file:///Users/asishsharma/programming/assembly/docs/FREESTANDING_KERNEL_SPEC.md) (Runtime Slicing, Memory Provider, Panic Hook, Linker Script)
- [BARE_METAL_TYPE_SPEC.md](file:///Users/asishsharma/programming/assembly/docs/BARE_METAL_TYPE_SPEC.md) (`mmio<T>` Types, Escape Discipline, Extended `asm { ... }`)
- [IRQ_SAFETY_SPEC.md](file:///Users/asishsharma/programming/assembly/docs/IRQ_SAFETY_SPEC.md) (Static Interrupt Discipline, Lattice Theory, Invariants)
- [BAREMETAL_QEMU_SPEC.md](file:///Users/asishsharma/programming/assembly/docs/BAREMETAL_QEMU_SPEC.md) (Multiboot2, Drivers, Dual-Channel QEMU Test Harness)

---

## 1. Executive Summary

This Request for Comments (RFC) establishes the complete, canonical architecture for compiling and running SNlang code directly on bare-metal hardware (Ring 0 / embedded microcontrollers) with zero operating system dependencies.

By leveraging SNlang's existing foundational mathematical achievements—specifically **Pinned Lanes**, **Regional Posets**, and **Lattice-Constrained Semilattices (`lcell`)**—SNlang provides native memory safety, deterministic execution, and data-race freedom without requiring a background Garbage Collection (GC) thread or POSIX runtime.

This specification closes the bare-metal gap through 5 core pillars:
1. **The Freestanding Runtime Profile (`--freestanding`)**: Slices `runtime.c` into a libc-independent `runtime_freestanding.c`, linking with zero undefined POSIX symbols (`-ffreestanding -nostdlib`).
2. **Pluggable Page Provider Contract**: Replaces POSIX `mmap()` with a 4KB frame-allocator callback contract (`sn_set_memory_provider`), allowing lanes, arenas, and pools to carve memory directly from kernel physical allocators.
3. **The Panic Contract (`@panic_handler`)**: Replaces user-space `exit(1)` and `stderr` with a diverging handler (`!`) that dumps CPU register states (RAX..R15, CR0..CR4) and stack traces to UART/VGA, followed by CPU halt (`cli; hlt`).
4. **Memory-Mapped I/O (`mmio<T>`) & Extended Assembly**: Provides strictly-typed, volatile bus access to device registers with escape-analysis protection (`E-MMIO-ESCAPE`), and volatile inline assembly with explicit input/output register constraints.
5. **The Static Interrupt Discipline (`irq fn`)**: An industry-first compile-time verification pass that enforces non-blocking, zero-allocation interrupt handlers via transitive call-graph closure (`E-IRQ-ALLOC`, `E-IRQ-BLOCK`, `E-IRQ-INTERPROC`).

---

## 2. System Architecture & Component Interactions

```
+-----------------------------------------------------------------------------------+
|                           SNlang Bare-Metal Kernel Profile                        |
+-----------------------------------------------------------------------------------+
|  [ irq fn ISRs ]            [ mmio<T> Device Regs ]       [ asm { ... } Inlines ] |
|  Transitive non-blocking    Strictly ordered bus I/O      Direct CPU control      |
|  and zero-allocation checks (volatile loads & stores)     (CR3, GDT, EOI, CLI)    |
+-----------------------------------------------------------------------------------+
|  [ Pinned Lanes / Pools ]   [ Pluggable Allocator ]       [ @panic_handler ]      |
|  O(1) stack reclaims and    Binds physical frame provider Custom diverging hook   |
|  generational indexing      (Buddy/Bitmap)                Dumps registers/serial  |
+-----------------------------------------------------------------------------------+
|                                 HARDWARE / QEMU                                   |
|   Multiboot2 Entry | VGA Buffer (0xB8000) | 16550 UART (0x3F8) | ISA Exit (0x501) |
+-----------------------------------------------------------------------------------+
```

---

## 3. The 6-Slice Implementation Roadmap

Execution of this architecture is partitioned into 6 discrete, non-overlapping slices:

### Slice 1: Multiboot2 Header, Linker Script & Trampoline
- **Objective:** Create the bootable ELF64 binary skeleton for x86_64.
- **Artifacts:** `kernel/arch/x86_64/multiboot_header.s`, `boot.s`, `kernel.ld`.
- **Exit Criteria:** Valid Multiboot2 ELF64 image boots under QEMU and enters 64-bit Long Mode without triple-faulting.

### Slice 2: Compiler Driver (`--freestanding`) & GC Link-out
- **Objective:** Teach `snc` the bare-metal profile and link-out GC.
- **Files Touched:** `compiler/src/{driver.rs, check.rs, parser.rs, token.rs}`.
- **Exit Criteria:** `snc --freestanding` compiles with `-ffreestanding -nostdlib -mno-red-zone`; any call to `new T()` is rejected at compile time with diagnostic `[E-NO-GC-IN-FREESTANDING]`.

### Slice 3: `mmio<T>` Type System & Extended Inline Assembly
- **Objective:** Type-safe device register bus operations and register-constrained inline assembly.
- **Files Touched:** `compiler/src/{types.rs, check.rs, ir.rs, llvm.rs}`.
- **Exit Criteria:** `mmio<u8>` emits LLVM `volatile` load/store; taking its reference is rejected with `[E-MMIO-ESCAPE]`; `asm { in("rax") v }` lowers to `call void asm sideeffect`.

### Slice 4: Pure SNlang Drivers (VGA & 16550 UART)
- **Objective:** Implement hardware console drivers in native SNlang.
- **Artifacts:** `kernel/drivers/vga.sn` (VGA text buffer at `0xB8000`), `kernel/drivers/uart.sn` (16550 UART at `0x3F8`).
- **Exit Criteria:** Kernel writes formatted text to both the VGA screen and COM1 serial console.

### Slice 5: Freestanding Runtime, Panic Hook & Pluggable Page Provider
- **Objective:** Eliminate libc from `runtime.c` and establish the kernel allocator hook.
- **Artifacts:** `compiler/runtime_freestanding.c`, `kernel/panic.sn`, `kernel/memory.sn`.
- **Exit Criteria:** Out-of-bounds checks and assertions route to user's `@panic_handler`; `sn_set_memory_provider` supplies 4KB frames to pinned lanes and pools.

### Slice 6: Automated QEMU Test Harness & Dual-Channel Exit
- **Objective:** Automated, headless CI validation of bare-metal SNlang kernels.
- **Artifacts:** `kernel/arch/x86_64/exit.sn`, `scripts/qemu_runner.py`, `tests/baremetal/test_smoke.sn`.
- **Exit Criteria:** `python3 scripts/qemu_runner.py --kernel target/kernel.elf` completes in < 2.0s with exit code 0 via ISA debugcon (port `0x501`) and Bochs ACPI poweroff (port `0x604`).
