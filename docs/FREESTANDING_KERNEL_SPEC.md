# SNlang Bare-Metal Kernel & Freestanding Runtime Specification
## Architecture, ABI, Memory Provider, Panic Contract, and Linker Specification

**Version:** 1.0.0-PROPOSED  
**Target Architecture:** x86_64 bare-metal (`x86_64-unknown-none-elf`)  
**Target Profile:** Freestanding Kernel (`--freestanding`)  
**Status:** Canonical OS & Runtime Systems Architecture Specification  

---

## 1. Executive Summary

This specification establishes the architectural foundation for compiling and executing **SNlang directly on bare-metal hardware** without an underlying operating system, POSIX libc, or user-space thread scheduler. It defines the formal interfaces, compilation profiles, memory management provider contracts, panic/diagnostic dispatch, and bootloader/linker integration required to deploy SNlang as a Ring-0 kernel or hypervisor executive.

The document is divided into four core technical pillars:
1. **The Freestanding Runtime Profile (`--freestanding`)**: Slicing `runtime.c` into a zero-dependency `runtime_freestanding.c`, stripping POSIX libc, pthreads, and virtual memory system calls while retaining core language invariants, decimals, strings, generational pools, and elastic lanes.
2. **Pluggable Memory Provider Contract**: The C and SNlang ABI enabling kernel memory managers to supply physical and virtual page frames to SNlang's elastic lanes, arenas, and pools without POSIX `mmap`.
3. **The Panic Contract (`@panic_handler`)**: Defining language-level and runtime-level panic handling, replacing user-space `exit(1)` and `fprintf(stderr)` with serial console dumps (UART 16550), VGA text mode displays, register snapshots, and CPU halts (`cli; hlt`).
4. **Linker Script & Target ABI**: Defining the Multiboot2 specification conformance, 32-bit to 64-bit transition trampoline, higher-half virtual memory mapping (`-2GB` / `0xFFFFFFFF80000000`), Red-Zone elimination (`-mno-red-zone`), and kernel code model (`-mcmodel=kernel`).

---

## 2. Pillar I: The Freestanding Runtime Profile (`--freestanding`)

### 2.1 Environmental Boundary & Stripping POSIX Dependencies

User-space SNlang relies on POSIX libc (`malloc`, `free`, `mmap`, `fprintf`, `clock_gettime`) and POSIX threads (`pthread_create`, `pthread_mutex_*`). In a bare-metal environment, Ring-0 code operates without standard libraries or OS system call tables.

#### Stripped Modules & Symbols:
1. **POSIX File System & Sockets**:
   - Stripped: `open`, `read`, `write`, `close`, `poll`, `epoll`, `kqueue`, `socket`, `bind`, `connect`, `listen`, `accept`, `getaddrinfo`.
   - Stripped Runtime Symbols: `sn_file_*`, `sn_net_*`, `sn_http_*`, `sn_subproc_*`, `RxHttp`, `RxOp`.
2. **POSIX Threading & Asynchronous Concurrency**:
   - Stripped: `<pthread.h>`, `pthread_create`, `pthread_join`, `pthread_mutex_*`, `pthread_cond_*`.
   - Stripped Runtime Symbols: `g_workers`, `SpawnJob`, `g_rx_cv`, `sn_spawn_worker`, `sn_chan_select`.
   - Note: Concurrency in kernel space operates via physical CPU cores (SMP/APIC trampolines), hardware timer interrupts, and cooperative fibers, not POSIX threads.
3. **Host Virtual Memory & Heap**:
   - Stripped: `mmap`, `munmap`, `mprotect`, `VirtualAlloc`, `calloc`, `free`, default `xmalloc`.
4. **Standard I/O & Process Exit**:
   - Stripped: `printf`, `fprintf`, `stderr`, `exit`.

### 2.2 Preserved Core Language Runtime (`runtime_freestanding.c`)

The sliced runtime preserves the deterministic, memory-safe data structures of SNlang while substituting bare-metal primitives:

1. **Freestanding Memory Primitives**:
   - Provides freestanding implementations of `sn_memset`, `sn_memcpy`, `sn_memmove`, `sn_memcmp`, `sn_strlen`, and `sn_strcmp`.
   - Exposes standard symbols (`memcpy`, `memset`, `memmove`, `memcmp`) to satisfy LLVM intrinsic lowerings under `-fno-builtin`.
2. **Elastic Lanes (`SnLane`)**:
   - Preserves exact 48-byte layout required by LLVM IR direct GEP lowerings:
     ```c
     typedef struct SnLane {
         uint8_t *base;            /* Offset 0 */
         uint8_t *top;             /* Offset 8 */
         uint8_t *committed;       /* Offset 16 */
         uint8_t *limit;           /* Offset 24 */
         uint32_t initialized;     /* Offset 32 */
         uint32_t frozen_depth;    /* Offset 36 */
         struct SnLane *parent;    /* Offset 40 */
     } SnLane;
     _Static_assert(sizeof(SnLane) == 48, "llvm.rs lane stride must stay 48");
     ```
   - Global lane array `g_tls_lanes[8]` is allocated in `.bss` (or bound via per-CPU storage).
   - Allocation bump, watermark save (`sn_lane_save`), and watermark rollback (`sn_lane_restore`) operate in $O(1)$ without calling OS mmap.
3. **Immutable Strings (`Str`)**:
   - Preserves `Str` representation:
     ```c
     typedef struct Str {
         int64_t rc;
         int64_t kind;
         int64_t len;
         char chars[];
     } Str;
     ```
   - String concatenation, slicing, and equality checks remain fully operational.
4. **Generational Pools (`SnPool`)**:
   - Preserves generational handle verification `(index: u32, generation: u32)` for use-after-free prevention.
   - Replaces `pthread_mutex` with bare-metal spinlocks (with interrupt-flag preservation).
5. **Decimal Arithmetic (`dec(n)`)**:
   - Fixed-point scaling, 128-bit intermediate multiplication and division, overflow checks.

### 2.3 Compiler Driver Integration (`snc --freestanding`)

When `--freestanding` is passed to `snc`, the driver modifies compilation and code generation:

```
+-----------------------------------------------------------------------------------------+
|                                    snc Driver Pipeline                                  |
|                                                                                         |
|  [Source .sn] ---> [Parser / AST] ---> [Type Checker] ---> [IR Lowering]                |
|                                                                  |                      |
|                                                      Emit LLVM IR (.ll)                 |
|                                                                  |                      |
|                                                                  v                      |
|                                              +---------------------------------------+  |
|                                              | clang -cc1 / clang invocation         |  |
|                                              |   --target=x86_64-unknown-none-elf    |  |
|                                              |   -ffreestanding -nostdlib            |  |
|                                              |   -mno-red-zone -mcmodel=kernel       |  |
|                                              |   -mno-mmx -mno-sse -mno-sse2         |  |
|                                              |   -fno-stack-protector -fno-pie       |  |
|                                              |   runtime_freestanding.c              |  |
|                                              |   -T kernel.ld                        |  |
|                                              +---------------------------------------+  |
+-----------------------------------------------------------------------------------------+
```

---

## 3. Pillar II: Pluggable Memory Provider Contract

### 3.1 Architecture Overview

In user-space, `SnLane` reserves 256MB of address space via `mmap(MAP_PRIVATE | MAP_ANON)` and incrementally commits 64KB chunks. In a bare-metal kernel, physical memory is finite and paging must be orchestrated by the kernel's own physical and virtual memory managers.

The **Pluggable Memory Provider Contract** decouples the runtime's memory allocators from physical page allocation mechanisms.

```
       +-------------------------------------------------------------+
       |                  SNlang High-Level Program                  |
       +-------------------------------------------------------------+
               |                       |                     |
               v                       v                     v
       +---------------+       +---------------+     +---------------+
       | Elastic Lanes |       | std.arena     |     | std.pool      |
       +---------------+       +---------------+     +---------------+
               \                       |                     /
                \                      v                    /
                 +-----------------------------------------+
                 |       runtime_freestanding.c            |
                 |       Carve & Slab Allocator            |
                 +-----------------------------------------+
                                       |
                 [Page Alloc / Free Function Pointers]
                                       |
                                       v
                 +-----------------------------------------+
                 |    Kernel Physical Frame Manager (PMM)  |
                 |    or Virtual Memory Manager (VMM)      |
                 +-----------------------------------------+
```

### 3.2 The C and SNlang ABI

#### C Interface (`runtime_freestanding.h`)
```c
#ifndef RUNTIME_FREESTANDING_H
#define RUNTIME_FREESTANDING_H

#include <stddef.h>
#include <stdint.h>
#include <stdbool.h>

#define SN_PAGE_SIZE 4096ULL
#define SN_PAGE_MASK (~(SN_PAGE_SIZE - 1ULL))

/* Function pointer prototypes for page frame allocation and deallocation */
typedef void *(*sn_page_alloc_fn)(size_t page_count);
typedef void  (*sn_page_free_fn)(void *virt_addr, size_t page_count);

typedef struct SnMemoryProvider {
    sn_page_alloc_fn alloc_pages;  /* Allocates 'page_count' contiguous virtual/physical pages */
    sn_page_free_fn  free_pages;   /* Returns 'page_count' pages to the provider */
    size_t           page_size;    /* Page size (must be 4096 or 2097152) */
    uint32_t         flags;        /* Allocation flags (e.g. SN_MEM_DMA, SN_MEM_ZEROED) */
} SnMemoryProvider;

/* Registration functions */
void sn_set_memory_provider(sn_page_alloc_fn alloc_fn, sn_page_free_fn free_fn);
void sn_set_memory_provider_ex(const SnMemoryProvider *provider);
const SnMemoryProvider *sn_get_memory_provider(void);

/* Static bootstrap memory assignment (pre-paging initialization) */
void sn_lane_bind_static_buffer(int64_t lane_id, void *buf, size_t size);
void sn_lane_pin(int64_t lane_id, void *base, size_t size);

#endif /* RUNTIME_FREESTANDING_H */
```

#### SNlang Extern Interface (`std/kernel/mem.sn`)
```snlang
package std.kernel.mem

extern "C" {
    fn sn_set_memory_provider(alloc_fn: ptr, free_fn: ptr)
    fn sn_set_memory_provider_ex(provider: ptr)
    fn sn_lane_bind_static_buffer(lane_id: int, buf: ptr, size: int)
    fn sn_lane_pin(lane_id: int, base: ptr, size: int)
}

blueprint MemoryProvider {
    alloc_pages: fn(count: int) -> ptr,
    free_pages: fn(p: ptr, count: int),
    page_size: int,
    flags: int
}
```

### 3.3 Memory Carving Mechanisms

#### 1. Static Bootstrap Buffer Carving
During early kernel initialization (before the MMU page tables and page allocators are operational), the kernel designates a static BSS buffer for Lane 0:
```c
static uint8_t boot_lane0_mem[64 * 1024] __attribute__((aligned(4096)));

void kernel_early_init(void) {
    sn_lane_bind_static_buffer(0, boot_lane0_mem, sizeof(boot_lane0_mem));
}
```
`sn_lane_bind_static_buffer` configures `g_tls_lanes[0]` with `base = top = boot_lane0_mem` and `limit = base + size`. This permits early execution of SNlang constructors and region blocks without page faults.

#### 2. Dynamic Frame Carving
Once paging is initialized:
- When a lane needs to expand or allocate memory beyond `committed`:
  - `sn_lane_ensure` requests initial frames (e.g. 16 pages = 64KB) via `provider->alloc_pages(16)`.
  - When `next > l->limit`, the runtime requests an extent of pages from `provider->alloc_pages(pages_needed)` and maps or chains the memory.
- When `sn_lane_restore` rewinds past a page boundary:
  - If dynamic reclamation is enabled, trailing complete pages are returned to the provider via `provider->free_pages(unwind_base, page_count)`.

#### 3. Pinned Lanes (`sn_lane_pin`)
In critical kernel code (Interrupt Service Routines, Timer Ticks, Page Fault Handlers, Scheduler Switches), page allocation calls or page faults are fatal.
- `sn_lane_pin(lane_id, base, size)` designates a pre-faulted, physically contiguous, non-swappable frame region.
- Allocations on a pinned lane are strictly guaranteed $O(1)$ pointer bumps with zero page allocator invocations.

#### 4. Arenas and Generational Pools
- **Arenas (`std.arena`)**: Arenas carve sub-allocations directly from their underlying lane watermark. Resetting an arena restores the watermark.
- **Generational Pools (`std.pool`)**: `SnPool` stores slots `(value, gen)`. In freestanding mode, `sn_pool_grow` allocates a slot array block from `provider->alloc_pages(n)` and copies existing items, releasing the old block back to the provider.

---

## 4. Pillar III: The Panic Contract

### 4.1 Specification: `@panic_handler`

In user-space, a panic or contract assert failure calls `fprintf(stderr, ...)` and terminates the process with `exit(1)`. In bare metal, there is no parent process. The kernel must handle the fault by dumping diagnostics to hardware interfaces and halting the CPU safely.

#### Language Syntax
SNlang introduces the `@panic_handler` decorator:

```snlang
@panic_handler
fn kernel_panic(file: str, line: int, msg: str) -> ! {
    vga_print_panic(file, line, msg);
    serial_print_panic(file, line, msg);
    dump_registers_and_halt();
}
```

#### Verification & Lowering Rules:
1. **Uniqueness**: Exactly one function in the root compilation unit may be decorated with `@panic_handler`.
2. **Signature**: Must accept `(file: str, line: int, msg: str)` (or C pointers `(ptr, int, ptr)`).
3. **Return Type**: Must have diverging return type `!` (`never`), indicating it does not return.
4. **Symbol Export**: The compiler lowers `@panic_handler` to the well-known external symbol `__sn_user_panic_handler`.

### 4.2 Panic Funnel & Fault Routing

All runtime assertion failures, bound check violations, null checks, and explicit `panic(...)` calls funnel through `sn_panic_internal`:

```c
__attribute__((noreturn))
void sn_panic_internal(const char *file, int64_t line, const char *msg) {
    /* Step 1: Disable CPU interrupts to freeze kernel state */
    __asm__ volatile ("cli");

    /* Step 2: Invoke user-defined panic handler if linked */
    extern void __sn_user_panic_handler(const char *, int64_t, const char *) __attribute__((weak));
    if (__sn_user_panic_handler) {
        __sn_user_panic_handler(file, line, msg);
    }

    /* Step 3: Default hardware fallback diagnostics */
    sn_default_panic_diagnostics(file, line, msg);

    /* Step 4: Indefinite CPU Halt */
    sn_cpu_halt();
}
```

### 4.3 Bare-Metal Diagnostics & CPU Halt

#### 1. Serial Port Driver (UART 16550, Port `0x3F8`)
The freestanding runtime includes an autonomous, zero-allocation polled serial driver:
- Configures 115200 baud, 8 data bits, no parity, 1 stop bit (8-N-1).
- Transmits characters by polling the Line Status Register (LSR, `0x3FD`, bit 5).
- Output format:
  ```
  [PANIC] File: kernel/main.sn, Line: 104
  [PANIC] Reason: Assertion failed in scheduler (lane 0 corrupted)
  ```

#### 2. VGA Text Mode Buffer (`0xFFFFFFFF800B8000` / Physical `0xB8000`)
- Renders an 80x25 Red Screen of Death:
  - Background: Red (`0x4`), Foreground: Bright White (`0xF`) -> Attribute byte `0x4F`.
  - Clears the screen and draws an ASCII error dialog displaying the filename, line number, message, and register dump.

#### 3. Register Snapshot & Stack Unwind
- Captures general purpose registers via assembly push into a `PanicFrame`:
  `RAX`, `RBX`, `RCX`, `RDX`, `RSI`, `RDI`, `RBP`, `RSP`, `R8`–`R15`, `RIP`, `RFLAGS`, `CR0`, `CR2`, `CR3`.
- Unwinds frame pointers (`RBP` walk) to print up to 16 return addresses (`RIP`).

#### 4. Safe CPU Halt (`cli; hlt`)
To prevent random memory execution, race conditions, or reboot loops:
```c
__attribute__((noreturn))
void sn_cpu_halt(void) {
    __asm__ volatile (
        "cli\n\t"
        "1:\n\t"
        "hlt\n\t"
        "jmp 1b\n\t"
        : : : "memory"
    );
    __builtin_unreachable();
}
```

---

## 5. Pillar IV: Linker Script & Target ABI

### 5.1 Memory Architecture: Higher-Half Kernel (-2GB / `0xFFFFFFFF80000000`)

#### Memory Map Layout
The kernel employs a **Higher-Half Kernel** design using the top 2GB virtual memory model (`-mcmodel=kernel`):

```
+-----------------------------------------------------------------------------------------+
|                                x86_64 Virtual Address Space                             |
+-----------------------------------------------------------------------------------------+
|  0x0000000000000000 - 0x00007FFFFFFFFFFF (128 TB)  | User-space / Processes (Lower-Half)|
+----------------------------------------------------+------------------------------------+
|  0x0000800000000000 - 0xFFFF7FFFFFFFFFFF           | Non-canonical address hole (traps) |
+----------------------------------------------------+------------------------------------+
|  0xFFFF800000000000 - 0xFFFFFFFEFFFFFFFF           | Direct physical mapping / I/O      |
+----------------------------------------------------+------------------------------------+
|  0xFFFFFFFF80000000 - 0xFFFFFFFFFFFFFFFF (2 GB)    | Kernel Code & Data (-2 GB Base)    |
|    0xFFFFFFFF80000000                              | KERNEL_VIRT_BASE                   |
|    0xFFFFFFFF80100000                              | Kernel Image Entry (1 MB physical) |
+-----------------------------------------------------------------------------------------+
```

#### Why `-mcmodel=kernel` (-2GB)?
In the x86-64 System V ABI, the kernel code model asserts that all kernel code, rodata, data, and bss symbols are positioned within `[0xFFFFFFFF80000000, 0xFFFFFFFFFFFFFFFF]`. This guarantees:
1. All symbol addresses sign-extend into 32-bit signed immediate fields.
2. Direct 32-bit `RIP`-relative addressing (`[rip + disp32]`) is valid across the entire kernel image.
3. Code size remains compact with zero 64-bit absolute address indirection penalties.

### 5.2 Multiboot2 Specification Conformance

The kernel image includes a Multiboot2 header embedded within the first 32,768 bytes, aligned to an 8-byte boundary.

#### Header Definition (`boot/multiboot2_header.S`)
```assembly
#define MULTIBOOT2_HEADER_MAGIC         0xE85250D6
#define MULTIBOOT_ARCHITECTURE_I386     0

.section .multiboot, "a"
.align 8
multiboot_header:
    .long MULTIBOOT2_HEADER_MAGIC
    .long MULTIBOOT_ARCHITECTURE_I386
    .long multiboot_header_end - multiboot_header
    .long -(MULTIBOOT2_HEADER_MAGIC + MULTIBOOT_ARCHITECTURE_I386 + (multiboot_header_end - multiboot_header))

    /* Tag: Information Request (Request memory map and ELF symbols) */
    .short 1                    /* type = information request */
    .short 0                    /* flags */
    .long 16                    /* size = 16 bytes */
    .long 4                     /* basic memory info */
    .long 6                     /* memory map */

    /* Tag: Module alignment */
    .short 6                    /* type = module alignment */
    .short 0                    /* flags */
    .long 8                     /* size = 8 bytes */

    /* Tag: End Tag */
    .short 0                    /* type = end */
    .short 0                    /* flags */
    .long 8                     /* size = 8 bytes */
multiboot_header_end:
```

### 5.3 32-Bit to 64-Bit Bootstrap Trampoline (`boot.S`)

Multiboot2 enters in **32-bit protected mode** at physical address `0x00100000` (1MB). The bootstrap code establishes initial 4-level paging, switches to 64-bit long mode, jumps to the higher half, and discards the low identity map.

```assembly
.section .boot.text, "ax"
.code32
.global _start
_start:
    /* Disable interrupts */
    cli
    cld

    /* Save Multiboot2 information pointer (EBX) and magic (EAX) */
    mov %eax, %esi
    mov %ebx, %edi

    /* Set up initial boot page tables:
       1. Identity map 0x00000000 - 0x3FFFFFFF (1GB)
       2. Higher-half map 0xFFFFFFFF80000000 - 0xFFFFFFFFFFFFFFFF (1GB) */
    call setup_boot_page_tables

    /* Enable PAE in CR4 */
    mov %cr4, %eax
    or $(1 << 5), %eax
    mov %eax, %cr4

    /* Set Long Mode Enable (LME) bit in IA32_EFER MSR (0xC0000080) */
    mov $0xC0000080, %ecx
    rdmsr
    or $(1 << 8), %eax
    wrmsr

    /* Load CR3 with address of boot_pml4 */
    mov $boot_pml4, %eax
    mov %eax, %cr3

    /* Enable Paging (PG) and Protection (PE) in CR0 */
    mov %cr0, %eax
    or $(1 << 31) | (1 << 0), %eax
    mov %eax, %cr0

    /* Far jump into 64-bit code segment */
    lgdt gdt64_pointer
    ljmp $0x08, $long_mode_start

.code64
long_mode_start:
    /* Reset segment registers to null / 64-bit data */
    mov $0x10, %ax
    mov %ax, %ds
    mov %ax, %es
    mov %ax, %fs
    mov %ax, %gs
    mov %ax, %ss

    /* Jump to Higher-Half Kernel entry */
    mov $higher_half_entry, %rax
    jmp *%rax

.section .text
higher_half_entry:
    /* Setup higher-half stack */
    mov $boot_stack_top, %rsp

    /* Pass Multiboot2 pointers into SNlang / C main */
    /* EDI = magic, RSI = multiboot_info_phys */
    call kernel_main

    /* Fallback halt if kernel_main returns */
    cli
1:  hlt
    jmp 1b
```

### 5.4 The Linker Script (`kernel.ld`)

```ld
/* SNlang Bare-Metal Higher-Half Linker Script */
OUTPUT_FORMAT("elf64-x86-64")
OUTPUT_ARCH(i386:x86-64)
ENTRY(_start)

KERNEL_PHYS_BASE = 0x00100000;          /* 1 MB physical load address */
KERNEL_VIRT_OFFSET = 0xFFFFFFFF80000000;  /* -2 GB higher-half base */
KERNEL_VIRT_BASE = KERNEL_VIRT_OFFSET + KERNEL_PHYS_BASE;

SECTIONS
{
    /* Physical load location for 32-bit bootstrap */
    . = KERNEL_PHYS_BASE;

    _kernel_phys_start = .;

    /* Multiboot2 header must reside in first 32 KB */
    .multiboot : {
        KEEP(*(.multiboot))
    }

    /* 32-bit bootstrap code & temporary page tables */
    .boot.text : {
        *(.boot.text)
    }

    .boot.data : {
        *(.boot.data)
    }

    .boot.bss : {
        *(.boot.bss)
    }

    /* Transition to higher-half virtual address space */
    . = ALIGN(4K) + KERNEL_VIRT_OFFSET;

    _kernel_virt_start = .;

    /* 64-bit Kernel Code */
    .text ALIGN(4K) : AT(ADDR(.text) - KERNEL_VIRT_OFFSET) {
        *(.text .text.*)
    }

    /* Read-only Data (strings, constants) */
    .rodata ALIGN(4K) : AT(ADDR(.rodata) - KERNEL_VIRT_OFFSET) {
        *(.rodata .rodata.*)
    }

    /* Read-write Data (global variables, lane tables) */
    .data ALIGN(4K) : AT(ADDR(.data) - KERNEL_VIRT_OFFSET) {
        *(.data .data.*)
    }

    /* Uninitialized Data and Kernel Stack */
    .bss ALIGN(4K) : AT(ADDR(.bss) - KERNEL_VIRT_OFFSET) {
        *(COMMON)
        *(.bss .bss.*)
        . = ALIGN(16);
        . += 0x10000; /* 64 KB Boot Stack */
        boot_stack_top = .;
    }

    _kernel_virt_end = .;
    _kernel_phys_end = . - KERNEL_VIRT_OFFSET;

    /* Discard unnecessary runtime sections */
    /DISCARD/ : {
        *(.comment)
        *(.note.*)
        *(.eh_frame)
    }
}
```

### 5.5 Target ABI Rules

1. **Red Zone Disabled (`-mno-red-zone`)**:
   - The System V AMD64 ABI reserves 128 bytes below `%rsp` for leaf function optimizations (the "Red Zone").
   - **Crucial Invariant**: In kernel mode, hardware interrupts (timer ticks, keyboards, exceptions) push the interrupt frame (`SS`, `RSP`, `RFLAGS`, `CS`, `RIP`, Error Code) directly onto the current stack. If the Red Zone is enabled, the CPU interrupt mechanism will overwrite live local variables stored below `%rsp`!
   - Every freestanding SNlang compilation unit **must** pass `-mno-red-zone`.
2. **Kernel Code Model (`-mcmodel=kernel`)**:
   - Limits all symbols to the high 2GB (`0xFFFFFFFF80000000`–`0xFFFFFFFFFFFFFFFF`).
   - Generates 32-bit sign-extended immediate instructions for absolute symbols and rip-relative offsets.
3. **Floating Point and Vector Registers (`-mno-mmx -mno-sse -mno-sse2`)**:
   - Kernel interrupt handlers should not incur the overhead of saving and restoring 512-byte `FXSAVE` / `XSAVE` vector states on every context switch.
   - Core runtime and kernel modules compile with soft-float or disabled vector registers.
4. **Stack Alignment & Protection**:
   - Compiler enforces 16-byte stack alignment before function calls (`-mpreferred-stack-boundary=4`).
   - Stack canary protection (`-fno-stack-protector`) is disabled until the kernel establishes thread-local storage (TLS) or the `%fs` segment base for `__stack_chk_guard`.

---

## 6. Implementation Plan & Verifier Milestones

| Phase | Milestone | Deliverables | Verification Criteria |
|---|---|---|---|
| **Phase 1** | Freestanding Runtime Slicing | `runtime_freestanding.c`, `runtime_freestanding.h` | Compiles with `clang -ffreestanding -nostdlib -c` with zero warnings; 48-byte `SnLane` assertions pass. |
| **Phase 2** | Compiler Driver Expansion | `compiler/src/driver.rs`, `compiler/src/main.rs` (`--freestanding`) | `snc --freestanding test.sn` correctly invokes clang with `-ffreestanding -nostdlib -mno-red-zone -mcmodel=kernel`. |
| **Phase 3** | Pluggable Memory Provider | `sn_set_memory_provider`, `sn_lane_bind_static_buffer`, `sn_lane_pin` | Unit test allocating 10,000 objects across 3 pinned and dynamic lanes using mock kernel page allocator. |
| **Phase 4** | Panic Contract & Hardware Drivers | `@panic_handler` AST/IR lowering, UART 16550 & VGA panic output | Simulated panic triggers serial output and CPU halt loop in QEMU bare-metal test environment. |
| **Phase 5** | Multiboot2 & Linker Script | `kernel.ld`, `boot.S`, Multiboot2 header | Produces valid ELF64 booted successfully by GRUB / QEMU (`qemu-system-x86_64 -kernel kernel.elf -serial stdio`). |

---

## 7. Architectural Conclusion

By decoupling memory allocation from POSIX `mmap` through the **Pluggable Memory Provider Contract**, formalizing hardware-level diagnostics through the **Panic Contract**, enforcing strict compile-time kernel invariants (`-mcmodel=kernel -mno-red-zone`), and structuring higher-half loading via **Multiboot2**, SNlang achieves full Ring-0 bare-metal capability. The language's deterministic compile-time memory guarantees (Elastic Lanes, Poset Regions, and Generational Pools) run natively on bare-metal silicon with zero GC pause times and predictable, sub-microsecond latency.
