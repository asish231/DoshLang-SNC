# SNlang Bare-Metal Kernel & QEMU Verification Architecture
**Technical Specification & Slice-by-Slice Implementation Plan**
**Author:** Embedded & Hardware Verification Engineer (`qemu_engineer`)  
**Target:** SNlang v0.2+ Bare-Metal Microkernel Subsystem (`x86_64`)  

---

## Executive Summary

This document establishes the formal engineering design for booting and verifying SNlang directly on bare-metal silicon and the QEMU hypervisor without an underlying operating system, POSIX libc, or user-space thread scheduler. It specifies:
1. **Multiboot2 Specification Header & Bootstrap Trampoline:** Compliant with the Multiboot2 specification for GRUB2 and QEMU direct ELF loading, transitioning the CPU from 32-bit protected mode to 64-bit long mode with identity/higher-half paging.
2. **Language Entry Point:** The `@entry fn kernel_main(magic: u64, mbi_ptr: u64) -> !` contract in SNlang.
3. **Hardware Device Drivers in Pure SNlang:** Memory-mapped I/O (`mmio<u8>`) drivers for the VGA 80x25 text mode buffer (`0xB8000`) and the 16550 UART serial controller (`0x3F8`), providing zero-dependency console output.
4. **Automated Headless QEMU Test Harness:** Zero-display headless execution with dual-channel exit signaling (ISA debug-exit port `0x501` and Bochs ACPI poweroff port `0x604`), guarded by a deterministic Python CI watchdog.
5. **6-Slice Implementation Roadmap:** Exact file touch lists, dependencies, and acceptance criteria across `compiler/`, `runtime/`, `kernel/`, and `tests/`.

---

## 1. Minimal Bootable Kernel Architecture

```
                      +--------------------------------------------------+
                      |               QEMU / GRUB2 Loader                |
                      +--------------------------------------------------+
                                                |
                                                | Direct ELF Load (-kernel)
                                                v
                      +--------------------------------------------------+
                      |         Multiboot2 Header (.multiboot)           |
                      |   Magic: 0xe85250d6, Arch: 0, Checksum, Tags     |
                      +--------------------------------------------------+
                                                |
                                                v
                      +--------------------------------------------------+
                      |       32-Bit Assembly Trampoline (boot.s)        |
                      |  1. Validate Multiboot2 magic (0x36d76289)       |
                      |  2. Verify CPUID Long Mode (EFER.LME)            |
                      |  3. Construct 4-level Paging Tables (PML4, PDPT) |
                      |  4. Enable PAE (CR4.PAE = 1)                     |
                      |  5. Enable Long Mode (EFER.LME = 1)              |
                      |  6. Enable Paging (CR0.PG = 1, CR0.PE = 1)       |
                      |  7. Load 64-Bit GDT (Code64: 0x08, Data64: 0x10) |
                      |  8. Far Jump: ljmp $0x08, $long_mode_entry       |
                      +--------------------------------------------------+
                                                |
                                                v
                      +--------------------------------------------------+
                      |        64-Bit Long Mode Transition (boot.s)      |
                      |  1. Reload DS, ES, FS, GS, SS = 0x10             |
                      |  2. Initialize Kernel Stack RSP = stack_top      |
                      |  3. Pass System V ABI Args:                      |
                      |     RDI = Magic (0x36d76289)                     |
                      |     RSI = Multiboot2 Info Struct Physical Addr   |
                      |  4. Call SNlang Entry Point                      |
                      +--------------------------------------------------+
                                                |
                                                v
                      +--------------------------------------------------+
                      |          SNlang Kernel Entry (main.sn)           |
                      |     @entry fn kernel_main(magic, mbi) -> !       |
                      |  - UART 16550 Serial Driver (0x3F8)              |
                      |  - VGA Text Buffer Driver (0xB8000)              |
                      |  - Pluggable Memory Provider Hook                |
                      |  - Bare-Metal Assertions & Test Suite            |
                      |  - Deterministic QEMU Exit (0x501 / 0x604)       |
                      +--------------------------------------------------+
```

### 1.1 Multiboot2 Specification Header

The Multiboot2 specification (v2.0) requires an 8-byte aligned header placed entirely within the first 32,768 bytes (32 KiB) of the binary image.

#### 1.1.1 Binary Header Structure

| Field Offset | Field Name | Type | Value | Description |
|---|---|---|---|---|
| `0x00` | `magic` | `u32` | `0xE85250D6` | Multiboot2 specification magic constant |
| `0x04` | `architecture` | `u32` | `0` | i386 32-bit protected mode boot state |
| `0x08` | `header_length` | `u32` | `header_end - header_start` | Total length of header including all tags |
| `0x0C` | `checksum` | `u32` | `-(magic + architecture + header_length)` | Sum of fields + checksum == 0 (mod 2^32) |
| `0x10` | Tags | variable | (See Tag Definitions) | Information requests, console, end tag |

#### 1.1.2 Tag Definitions
1. **Information Request Tag (Type 1):** Requests that the bootloader provide the Memory Map (`tag 6`) and ELF Symbols (`tag 9`).
2. **Console Flags Tag (Type 4):** Requests EGA text console support (flags = 3).
3. **End Tag (Type 0):** Mandatory terminator (type = 0, flags = 0, size = 8).

#### 1.1.3 Assembly Implementation (`kernel/arch/x86_64/multiboot_header.s`)

```gas
/* Multiboot2 Header for SNlang Bare-Metal Kernel */
.section .multiboot_header, "a"
.align 8
header_start:
    /* Magic number (Multiboot2) */
    .long 0xe85250d6
    /* Architecture: 0 = 32-bit (protected) mode i386 */
    .long 0
    /* Header length */
    .long header_end - header_start
    /* Checksum */
    .long 0x100000000 - (0xe85250d6 + 0 + (header_end - header_start))

    /* Tag: Information Request */
    .align 8
tag_info_req_start:
    .short 1                    /* Type = 1 (Information Request) */
    .short 0                    /* Flags = 0 */
    .long tag_info_req_end - tag_info_req_start /* Size */
    .long 6                     /* Request Tag 6: Memory Map */
    .long 9                     /* Request Tag 9: ELF Symbols */
tag_info_req_end:

    /* Tag: Console Flags */
    .align 8
tag_console_start:
    .short 4                    /* Type = 4 (Console Flags) */
    .short 0                    /* Flags = 0 */
    .long tag_console_end - tag_console_start /* Size */
    .long 3                     /* Console flags: text console allowed */
tag_console_end:

    /* Mandatory Terminator Tag */
    .align 8
tag_end_start:
    .short 0                    /* Type = 0 (End) */
    .short 0                    /* Flags = 0 */
    .long 8                     /* Size = 8 */
tag_end_end:
header_end:
```

---

### 1.2 Entry Point & 64-Bit Trampoline

#### 1.2.1 Hand-off Machine State from Multiboot2 Loader
Upon transfer from GRUB or `qemu -kernel`:
- **CPU Mode:** 32-bit Protected Mode with Paging Disabled (`CR0.PE = 1`, `CR0.PG = 0`).
- **Registers:**
  - `EAX`: `0x36D76289` (Multiboot2 magic verification).
  - `EBX`: 32-bit physical base address of the Multiboot2 Information Structure (MBI).
- **Interrupts:** Disabled (`IF = 0`).
- **A20 Gate:** Enabled.

#### 1.2.2 Trampoline Assembly (`kernel/arch/x86_64/boot.s`)

The trampoline initializes 4-level paging (PML4, PDPT, PD) identity-mapping the first 1 GiB using 2 MiB huge pages, enables Long Mode (`EFER.LME`), enables Paging, loads a 64-bit GDT, and far-jumps into long mode.

```gas
.code32
.section .boot, "ax"
.global _start
.extern kernel_main

_start:
    /* 1. Ensure interrupts are strictly disabled */
    cli

    /* 2. Validate Multiboot2 Magic Number in EAX */
    cmp $0x36d76289, %eax
    jne .no_multiboot

    /* 3. Save MBI pointer (EBX) into EDI across CPUID calls */
    mov %ebx, %edi

    /* 4. Verify CPUID and Long Mode support */
    call check_cpuid
    call check_long_mode

    /* 5. Set up bootstrap page tables */
    call setup_page_tables

    /* 6. Enable PAE in CR4 (Bit 5) */
    mov %cr4, %eax
    or $(1 << 5), %eax
    mov %eax, %cr4

    /* 7. Load PML4 base into CR3 */
    mov $boot_pml4, %eax
    mov %eax, %cr3

    /* 8. Enable Long Mode in EFER MSR (0xC0000080, Bit 8: LME) */
    mov $0xc0000080, %ecx
    rdmsr
    or $(1 << 8), %eax
    wrmsr

    /* 9. Enable Paging (CR0.PG = Bit 31) and Protection (CR0.PE = Bit 0) */
    mov %cr0, %eax
    or $(1 << 31 | 1 << 0), %eax
    mov %eax, %cr0

    /* 10. Load 64-bit Global Descriptor Table */
    lgdt (gdt64_ptr)

    /* 11. Far Jump to 64-bit long mode code segment (Selector 0x08) */
    ljmp $0x08, $long_mode_start

.no_multiboot:
    /* Write red error marker 'M' to VGA buffer and halt */
    movw $0x4F4D, 0xb8000
    hlt
    jmp .no_multiboot

check_cpuid:
    /* Check if CPUID is supported by toggling EFLAGS ID bit (bit 21) */
    pushfl
    pop %eax
    mov %eax, %ecx
    xor $(1 << 21), %eax
    push %eax
    popfl
    pushfl
    pop %eax
    push %ecx
    popfl
    xor %ecx, %eax
    jz .no_cpuid
    ret
.no_cpuid:
    movw $0x4F43, 0xb8000 /* 'C' */
    hlt
    jmp .no_cpuid

check_long_mode:
    /* Verify extended processor info exists */
    mov $0x80000000, %eax
    cpuid
    cmp $0x80000001, %eax
    jb .no_long_mode
    /* Query extended feature flags */
    mov $0x80000001, %eax
    cpuid
    test $(1 << 29), %edx /* Bit 29: Long Mode (LM) */
    jz .no_long_mode
    ret
.no_long_mode:
    movw $0x4F4C, 0xb8000 /* 'L' */
    hlt
    jmp .no_long_mode

setup_page_tables:
    /* Point boot_pml4[0] to boot_pdpt */
    mov $boot_pdpt, %eax
    or $0x03, %eax /* Present (bit 0) | Writable (bit 1) */
    mov %eax, boot_pml4

    /* Point boot_pdpt[0] to boot_pd */
    mov $boot_pd, %eax
    or $0x03, %eax
    mov %eax, boot_pdpt

    /* Identity map the first 1 GiB using 512 x 2 MiB huge pages */
    mov $0, %ecx
.map_pde:
    mov $0x200000, %eax /* 2 MiB */
    mul %ecx
    or $0x83, %eax       /* Present | Writable | HugePage (bit 7) */
    mov %eax, boot_pd(,%ecx,8)
    inc %ecx
    cmp $512, %ecx
    jne .map_pde
    ret

/* ---------------- 64-Bit Subsegment ---------------- */
.code64
long_mode_start:
    /* Reload 64-bit data segment registers with selector 0x10 */
    mov $0x10, %ax
    mov %ax, %ds
    mov %ax, %es
    mov %ax, %ss
    mov %ax, %fs
    mov %ax, %gs

    /* Initialize kernel execution stack */
    mov $stack_top, %rsp
    xor %rbp, %rbp
    cld

    /* Pass System V AMD64 ABI Arguments to SNlang kernel_main:
       RDI = Arg 1: Multiboot2 magic (0x36d76289)
       RSI = Arg 2: Multiboot2 Information structure pointer */
    mov $0x36d76289, %rdi
    mov %rdi, %rax
    mov %rdi, %rdi
    mov %rdi, %rdi
    mov $0x36d76289, %rdi
    mov %rdi, %rdi
    mov %rdi, %rdi
    mov $0x36d76289, %rdi
    mov %rdi, %rdi
    mov %rdi, %rdi
    mov $0x36d76289, %rdi
    /* Zero-extend 32-bit EDI (saved MBI pointer) into 64-bit RSI */
    mov %edi, %esi

    /* Invoke SNlang Entry Point */
    call kernel_main

    /* Fallback Hang Loop (diverging fn contract) */
.kernel_hang:
    cli
    hlt
    jmp .kernel_hang

/* ---------------- GDT & Page Table Allocation ---------------- */
.section .rodata
.align 8
gdt64:
    .quad 0x0000000000000000 /* 0x00: Null descriptor */
    .quad 0x00af9a000000ffff /* 0x08: 64-bit Code (DPL=0, R/X, Long mode L=1) */
    .quad 0x00af92000000ffff /* 0x10: 64-bit Data (DPL=0, R/W) */
gdt64_ptr:
    .short . - gdt64 - 1
    .quad gdt64

.section .bss
.align 4096
boot_pml4:
    .skip 4096
boot_pdpt:
    .skip 4096
boot_pd:
    .skip 4096
stack_bottom:
    .skip 65536 /* 64 KiB bootstrap kernel stack */
stack_top:
```

#### 1.2.3 Linker Script (`kernel/arch/x86_64/linker.ld`)

```ld
ENTRY(_start)
OUTPUT_FORMAT("elf64-x86-64")

SECTIONS
{
    /* Load address: 1 MiB physical (standard for x86 bootloaders) */
    . = 1M;

    /* Multiboot2 header MUST be within first 32 KiB */
    .multiboot_header : ALIGN(8) {
        KEEP(*(.multiboot_header))
    }

    .boot : ALIGN(4K) {
        *(.boot)
    }

    .text : ALIGN(4K) {
        *(.text .text.*)
    }

    .rodata : ALIGN(4K) {
        *(.rodata .rodata.*)
    }

    .data : ALIGN(4K) {
        *(.data .data.*)
    }

    .bss : ALIGN(4K) {
        *(COMMON)
        *(.bss .bss.*)
    }

    /DISCARD/ : {
        *(.comment)
        *(.eh_frame)
        *(.note.gnu.build-id)
    }
}
```

---

### 1.3 VGA Text Buffer Driver (`0xB8000`) in SNlang

The VGA text buffer is mapped at physical address `0xB8000`. In 80x25 text mode, each cell consists of 2 contiguous bytes:
- Byte 0: ASCII code (`u8`).
- Byte 1: Color Attribute (`u8`): `(bg << 4) | fg`.

#### 1.3.1 Driver Implementation (`kernel/drivers/vga.sn`)

```snlang
// VGA Text Buffer Driver in Pure SNlang (0xB8000)
const u64 VGA_BASE = 0xB8000
const int VGA_WIDTH = 80
const int VGA_HEIGHT = 25

// VGA Color Palette
const u8 COLOR_BLACK         = 0x00
const u8 COLOR_BLUE          = 0x01
const u8 COLOR_GREEN         = 0x02
const u8 COLOR_CYAN          = 0x03
const u8 COLOR_RED           = 0x04
const u8 COLOR_MAGENTA       = 0x05
const u8 COLOR_BROWN         = 0x06
const u8 COLOR_LIGHT_GRAY    = 0x07
const u8 COLOR_DARK_GRAY     = 0x08
const u8 COLOR_LIGHT_BLUE    = 0x09
const u8 COLOR_LIGHT_GREEN   = 0x0A
const u8 COLOR_LIGHT_CYAN    = 0x0B
const u8 COLOR_LIGHT_RED     = 0x0C
const u8 COLOR_PINK          = 0x0D
const u8 COLOR_YELLOW        = 0x0E
const u8 COLOR_WHITE         = 0x0F

int vga_row = 0
int vga_col = 0
u8 vga_attr = 0x0F // White on Black

fn vga_cell_addr(row: int, col: int) -> u64 {
    int offset = (row * VGA_WIDTH + col) * 2
    return VGA_BASE + cast(offset, u64)
}

fn vga_write_cell(row: int, col: int, ch: u8, attr: u8) {
    u64 addr = vga_cell_addr(row, col)
    mmio<u8> char_reg = cast(addr, mmio<u8>)
    mmio<u8> attr_reg = cast(addr + 1, mmio<u8>)
    char_reg.write(ch)
    attr_reg.write(attr)
}

fn vga_read_char(row: int, col: int) -> u8 {
    u64 addr = vga_cell_addr(row, col)
    mmio<u8> char_reg = cast(addr, mmio<u8>)
    return char_reg.read()
}

fn vga_read_attr(row: int, col: int) -> u8 {
    u64 addr = vga_cell_addr(row, col)
    mmio<u8> attr_reg = cast(addr + 1, mmio<u8>)
    return attr_reg.read()
}

fn vga_scroll() {
    // Shift rows 1..VGA_HEIGHT-1 upward to 0..VGA_HEIGHT-2
    for (r in 1..VGA_HEIGHT) {
        for (c in 0..VGA_WIDTH) {
            u8 ch = vga_read_char(r, c)
            u8 attr = vga_read_attr(r, c)
            vga_write_cell(r - 1, c, ch, attr)
        }
    }
    // Clear bottom row
    for (c in 0..VGA_WIDTH) {
        vga_write_cell(VGA_HEIGHT - 1, c, 0x20, vga_attr)
    }
    vga_row = VGA_HEIGHT - 1
    vga_col = 0
}

fn vga_update_cursor(row: int, col: int) {
    int pos = row * VGA_WIDTH + col
    u8 low = cast(pos & 0xFF, u8)
    u8 high = cast((pos >> 8) & 0xFF, u8)
    
    // CRTC Register 14 (Cursor High)
    asm {
        in("dx") 0x3D4_u16, in("al") 0x0E_u8: "out %al, %dx"
        in("dx") 0x3D5_u16, in("al") high: "out %al, %dx"
    }
    // CRTC Register 15 (Cursor Low)
    asm {
        in("dx") 0x3D4_u16, in("al") 0x0F_u8: "out %al, %dx"
        in("dx") 0x3D5_u16, in("al") low: "out %al, %dx"
    }
}

fn vga_init() {
    vga_row = 0
    vga_col = 0
    vga_attr = 0x0F
    for (r in 0..VGA_HEIGHT) {
        for (c in 0..VGA_WIDTH) {
            vga_write_cell(r, c, 0x20, vga_attr)
        }
    }
    vga_update_cursor(0, 0)
}

fn vga_putc(ch: u8) {
    if (ch == 0x0A) { // Newline '\n'
        vga_col = 0
        vga_row += 1
    } else if (ch == 0x0D) { // Carriage return '\r'
        vga_col = 0
    } else if (ch == 0x08) { // Backspace '\b'
        if (vga_col > 0) {
            vga_col -= 1
            vga_write_cell(vga_row, vga_col, 0x20, vga_attr)
        }
    } else {
        vga_write_cell(vga_row, vga_col, ch, vga_attr)
        vga_col += 1
        if (vga_col >= VGA_WIDTH) {
            vga_col = 0
            vga_row += 1
        }
    }

    if (vga_row >= VGA_HEIGHT) {
        vga_scroll()
    }
    vga_update_cursor(vga_row, vga_col)
}

fn vga_print(s: str) {
    int len = s.length()
    for (i in 0..len) {
        vga_putc(s.byte_at(i))
    }
}
```

---

### 1.4 16550 UART Serial Driver (`0x3F8`) in SNlang

The 16550 UART (COM1) provides the primary headless debugging and test automation pipeline.

#### 1.4.1 Register Map (Base Port `0x3F8`)

| Offset | DLAB | Read / Write | Register Name | Description |
|---|---|---|---|---|
| `+0` | 0 | Write | `THR` | Transmitter Holding Register (transmitting data) |
| `+0` | 0 | Read | `RBR` | Receiver Buffer Register (received data) |
| `+0` | 1 | R/W | `DLL` | Divisor Latch Low (baud rate divisor low byte) |
| `+1` | 0 | R/W | `IER` | Interrupt Enable Register |
| `+1` | 1 | R/W | `DLH` | Divisor Latch High (baud rate divisor high byte) |
| `+2` | X | Write | `FCR` | FIFO Control Register |
| `+2` | X | Read | `IIR` | Interrupt Identification Register |
| `+3` | X | R/W | `LCR` | Line Control Register (Bit 7 = DLAB) |
| `+4` | X | R/W | `MCR` | Modem Control Register |
| `+5` | X | Read | `LSR` | Line Status Register (Bit 5 = THRE, Bit 0 = DR) |
| `+6` | X | Read | `MSR` | Modem Status Register |
| `+7` | X | R/W | `SCR` | Scratch Register |

#### 1.4.2 Driver Implementation (`kernel/drivers/uart.sn`)

SNlang provides both I/O Port operations (standard x86 `0x3F8`) and Memory-Mapped I/O abstractions.

```snlang
// 16550 UART Serial Driver in Pure SNlang (COM1: 0x3F8)
const u16 COM1_BASE = 0x3F8

fn port_outb(port: u16, val: u8) {
    asm {
        in("dx") port, in("al") val: "out %al, %dx"
    }
}

fn port_inb(port: u16) -> u8 {
    u8 res = 0
    asm {
        in("dx") port, out("al") res: "in %dx, %al"
    }
    return res
}

fn uart_init() {
    // 1. Disable all serial interrupts during initialization
    port_outb(COM1_BASE + 1, 0x00)

    // 2. Enable DLAB (Divisor Latch Access Bit, Bit 7 of LCR) to configure baud rate
    port_outb(COM1_BASE + 3, 0x80)

    // 3. Set baud rate divisor to 1 (115200 baud, 1.8432 MHz / (16 * 115200))
    port_outb(COM1_BASE + 0, 0x01) // DLL = 1
    port_outb(COM1_BASE + 1, 0x00) // DLH = 0

    // 4. Set framing: 8 bits, no parity, 1 stop bit (8N1); clears DLAB
    port_outb(COM1_BASE + 3, 0x03)

    // 5. Enable and configure 14-byte FIFO, clearing RX/TX queues
    port_outb(COM1_BASE + 2, 0xC7)

    // 6. Set Modem Control: RTS/DTR active, Auxiliary Output 2 enabled
    port_outb(COM1_BASE + 4, 0x0B)
}

fn uart_is_tx_empty() -> bool {
    // LSR Bit 5 (0x20): Transmitter Holding Register Empty (THRE)
    u8 lsr = port_inb(COM1_BASE + 5)
    return (lsr & 0x20) != 0
}

fn uart_has_rx() -> bool {
    // LSR Bit 0 (0x01): Data Ready (DR)
    u8 lsr = port_inb(COM1_BASE + 5)
    return (lsr & 0x01) != 0
}

fn uart_putc(ch: u8) {
    // Spin-wait until transmitter buffer is empty
    while (not uart_is_tx_empty()) {
        asm { pause }
    }
    port_outb(COM1_BASE + 0, ch)
}

fn uart_getc() -> u8 {
    // Spin-wait until receiver buffer contains incoming byte
    while (not uart_has_rx()) {
        asm { pause }
    }
    return port_inb(COM1_BASE + 0)
}

fn uart_print(s: str) {
    int len = s.length()
    for (i in 0..len) {
        u8 ch = s.byte_at(i)
        if (ch == 0x0A) { // Normalize '\n' to '\r\n' for terminal emulators
            uart_putc(0x0D)
        }
        uart_putc(ch)
    }
}
```

---

## 2. QEMU Bare-Metal Test Harness & Automated Termination

### 2.1 Headless Execution Command Line

```sh
qemu-system-x86_64 \
    -kernel kernel.elf \
    -nographic \
    -serial stdio \
    -display none \
    -no-reboot \
    -m 128M \
    -device isa-debug-exit,iobase=0x501,iosize=0x01
```

#### Parameter Rationale
- `-kernel kernel.elf`: Uses QEMU's internal Multiboot2 direct loader to parse the ELF executable and jump directly to `_start` without requiring a GRUB stage-2 ISO disk image.
- `-nographic`: Completely disables graphical output; multiplexes UART serial output directly to standard I/O.
- `-serial stdio`: Connects COM1 UART to host terminal stdout/stdin, enabling line-buffered log capture.
- `-display none`: Disables graphical framebuffer canvas allocation, minimizing VM initialization overhead to < 50ms.
- `-no-reboot`: Exits QEMU immediately upon CPU triple-fault or shutdown instead of infinite reboot looping.
- `-m 128M`: Allocates a deterministic 128 MiB physical RAM space.
- `-device isa-debug-exit,iobase=0x501,iosize=0x01`: Instantiates the ISA debug exit device at I/O port `0x501`.

---

### 2.2 Automated Test Termination Mechanics

CI test automation requires deterministic process termination. Bare-metal kernels cannot call `exit(0)`. We specify two complementary hardware termination mechanisms:

```
                            TEST TERMINATION DISPATCH
                                        |
                 +----------------------+----------------------+
                 |                                             |
                 v                                             v
     Method A: isa-debug-exit                      Method B: ACPI Poweroff
         (Port 0x501)                                   (Port 0x604)
                 |                                             |
     outb(0x501, status)                           outw(0x604, 0x2000)
                 |                                             |
                 v                                             v
     QEMU exits with status:                       QEMU exits with status:
       exit_code = (val << 1) | 1                        exit_code = 0
     Status 0x10 -> Exit Code 33                   Clean Host CI Pass
     Status 0x11 -> Exit Code 35
```

#### 2.2.1 Method A: ISA Debug Exit Device (`0x501`)
The QEMU `isa-debug-exit` device maps an I/O port (`0x501`) such that writing value $v$ causes QEMU to immediately terminate with host exit status:
$$\text{ExitCode} = (v \ll 1) \mid 1$$
- Success: Kernel writes `0x10` (16) $\implies$ Exit status $(16 \times 2) + 1 = \mathbf{33}$.
- Failure: Kernel writes `0x11` (17) $\implies$ Exit status $(17 \times 2) + 1 = \mathbf{35}$.

#### 2.2.2 Method B: Bochs / QEMU ACPI Poweroff (`0x604`)
Writing `0x2000` to the Bochs ACPI poweroff port `0x604` initiates clean ACPI shutdown, yielding host exit status $\mathbf{0}$.

#### 2.2.3 Unified Exit Helper in SNlang (`kernel/arch/x86_64/exit.sn`)

```snlang
fn qemu_exit(status: u8) -> ! {
    // 1. Signal through ISA debug-exit device (Port 0x501)
    asm {
        in("dx") 0x501_u16, in("al") status: "out %al, %dx"
    }

    // 2. If status == 0, trigger Bochs/QEMU ACPI clean shutdown (Port 0x604)
    if (status == 0) {
        asm {
            in("dx") 0x604_u16, in("ax") 0x2000_u16: "out %ax, %dx"
        }
    }

    // 3. Halt fallback
    loop {
        asm {
            cli
            hlt
        }
    }
}
```

---

### 2.3 Python CI Test Runner (`scripts/qemu_runner.py`)

A robust Python harness executes QEMU, enforces an execution timeout, validates log assertions, and parses exit codes.

```python
#!/usr/bin/env python3
"""
Automated QEMU Bare-Metal Verification Test Runner for SNlang
Enforces deterministic timeouts, verifies serial logs, and handles ISA exit codes.
"""

import sys
import subprocess
import argparse
import time

def run_kernel_test(kernel_path: str, timeout_sec: float = 5.0) -> int:
    cmd = [
        "qemu-system-x86_64",
        "-kernel", kernel_path,
        "-nographic",
        "-serial", "stdio",
        "-display", "none",
        "-no-reboot",
        "-m", "128M",
        "-device", "isa-debug-exit,iobase=0x501,iosize=0x01"
    ]

    print(f"[TEST RUNNER] Launching: {' '.join(cmd)}")
    start_time = time.time()

    try:
        proc = subprocess.Popen(
            cmd,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True
        )
    except FileNotFoundError:
        print("[TEST ERROR] qemu-system-x86_64 not found on host system.")
        return 2

    stdout_buf = []
    has_test_pass = False
    has_panic = False

    try:
        stdout_data, stderr_data = proc.communicate(timeout=timeout_sec)
        elapsed = time.time() - start_time
        stdout_buf.append(stdout_data)

        # Print serial output to console for inspection
        for line in stdout_data.splitlines():
            print(f"[SERIAL] {line}")
            if "[SN-TEST-PASS]" in line:
                has_test_pass = True
            if "[PANIC]" in line:
                has_panic = True

        raw_code = proc.returncode
        print(f"[TEST RUNNER] QEMU terminated in {elapsed:.2f}s with raw exit code {raw_code}")

        # Normalization:
        # Exit code 33 = ISA debug-exit (0x10) -> SUCCESS
        # Exit code 0  = ACPI poweroff -> SUCCESS
        # Exit code 35 = ISA debug-exit (0x11) -> ASSERTION FAILURE
        if raw_code in (0, 33) and has_test_pass and not has_panic:
            print("[TEST RUNNER] RESULT: PASS (Deterministic bare-metal verification confirmed)")
            return 0
        elif raw_code == 35 or has_panic:
            print("[TEST RUNNER] RESULT: FAIL (Bare-metal assertion or panic occurred)")
            return 1
        else:
            print(f"[TEST RUNNER] RESULT: FAIL (Unexpected exit code {raw_code})")
            return 1

    except subprocess.TimeoutExpired:
        proc.kill()
        print(f"[TEST RUNNER] RESULT: FAIL (Watchdog timeout after {timeout_sec}s - kernel hung)")
        return 1

if __name__ == "__main__":
    parser = argparse.ArgumentParser(description="SNlang Bare-Metal QEMU Runner")
    parser.add_argument("--kernel", required=True, help="Path to kernel.elf")
    parser.add_argument("--timeout", type=float, default=5.0, help="Watchdog timeout in seconds")
    args = parser.parse_args()

    sys.exit(run_kernel_test(args.kernel, args.timeout))
```

---

## 3. Slice-by-Slice Implementation Plan

The kernel transition is structured into six discrete, testable slices.

```
+---------------------------------------------------------------------------------------+
| SLICE 1: Multiboot2 Header, Linker Script (linker.ld) & Trampoline (boot.s)           |
+---------------------------------------------------------------------------------------+
                                           |
                                           v
+---------------------------------------------------------------------------------------+
| SLICE 2: Compiler Driver (--freestanding), @entry Attribute & GC Link-out Rule        |
+---------------------------------------------------------------------------------------+
                                           |
                                           v
+---------------------------------------------------------------------------------------+
| SLICE 3: Type System mmio<T> & Extended Inline Assembly asm { ... }                   |
+---------------------------------------------------------------------------------------+
                                           |
                                           v
+---------------------------------------------------------------------------------------+
| SLICE 4: VGA Text Buffer Driver (0xB8000) & 16550 UART Serial Driver (0x3F8)          |
+---------------------------------------------------------------------------------------+
                                           |
                                           v
+---------------------------------------------------------------------------------------+
| SLICE 5: Freestanding Runtime Profile, Panic Handler (@panic_handler) & Memory Provider|
+---------------------------------------------------------------------------------------+
                                           |
                                           v
+---------------------------------------------------------------------------------------+
| SLICE 6: Automated QEMU Test Harness, ISA Debug Exit & CI Verification Suite          |
+---------------------------------------------------------------------------------------+
```

---

### Slice 1: Multiboot2 Header, Linker Script & Trampoline

- **Objective:** Establish the low-level ELF memory layout and 32-to-64-bit transition code.
- **Dependencies:** GNU Assembler / Clang LLVM (`as`, `ld.lld` or `clang`).
- **File Touch List:**
  - `kernel/arch/x86_64/multiboot_header.s` (Multiboot2 specification header)
  - `kernel/arch/x86_64/boot.s` (32-bit protected mode to 64-bit long mode trampoline)
  - `kernel/arch/x86_64/linker.ld` (Physical load address 1 MiB, section alignment)
- **Acceptance Criteria:**
  - `clang -c kernel/arch/x86_64/boot.s` and `ld -T kernel/arch/x86_64/linker.ld` produces a valid ELF64 binary.
  - `qemu-system-x86_64 -kernel kernel.elf` successfully loads the kernel without triple-faulting or falling into `.no_multiboot`.

---

### Slice 2: Compiler Driver (`--freestanding`), `@entry` Attribute & GC Link-out

- **Objective:** Enable the SNlang compiler (`snc`) to compile freestanding targets without requiring POSIX libc, threads, or default `runtime.c`.
- **Dependencies:** Slice 1.
- **File Touch List:**
  - `compiler/src/token.rs`: Add `TokenKind::At` (`@`).
  - `compiler/src/parser.rs`: Parse `@entry` and `@panic_handler` decorators on function declarations.
  - `compiler/src/ast.rs`: Add `is_entry: bool` and `is_panic_handler: bool` to `FnItem`.
  - `compiler/src/check.rs`:
    - Disallow `new T()` when `--freestanding` flag is active (`E-NO-GC-IN-FREESTANDING`).
    - Permit programs lacking `fn main()` if a valid `@entry fn` is declared.
  - `compiler/src/driver.rs`:
    - Add `--freestanding` CLI flag.
    - Omit `-pthread` and default POSIX runtime linking when `--freestanding` is passed.
- **Acceptance Criteria:**
  - Compiling an SNlang file with `@entry fn kernel_main(magic: u64, mbi: u64) -> !` succeeds under `snc --freestanding`.
  - Calling `new Point()` in `--freestanding` mode triggers compile-time error `E-NO-GC-IN-FREESTANDING`.

---

### Slice 3: Type System `mmio<T>` & Extended Inline Assembly `asm { ... }`

- **Objective:** Implement type-safe memory-mapped register access and volatile inline assembly.
- **Dependencies:** Slice 2.
- **File Touch List:**
  - `compiler/src/types.rs`: Add `Type::Mmio(Box<Type>)`.
  - `compiler/src/token.rs`: Add `TokenKind::Mmio` and `TokenKind::Asm`.
  - `compiler/src/parser.rs`: Parse `mmio<u8>`, `mmio<u16>`, `mmio<u32>`, `mmio<u64>` and `asm { in("al") x, out("al") y: "inst" }`.
  - `compiler/src/check.rs`:
    - Enforce scalar width restrictions on `mmio<T>`.
    - Prohibit taking address/reference of MMIO variables (`E-MMIO-ESCAPE`).
  - `compiler/src/ir.rs` & `compiler/src/llvm.rs`:
    - Lower `.read()` to LLVM `load volatile`.
    - Lower `.write(v)` to LLVM `store volatile`.
    - Lower `asm { ... }` blocks to LLVM `call void asm sideeffect`.
- **Acceptance Criteria:**
  - Emitted LLVM IR contains `load volatile i8, ptr` and `store volatile i8, ptr`.
  - LLVM `-O2` optimizations do not elide, reorder, or cache MMIO register accesses.

---

### Slice 4: VGA Text Buffer & 16550 UART Drivers in Pure SNlang

- **Objective:** Provide robust, zero-dependency console and serial communication written entirely in SNlang.
- **Dependencies:** Slice 3.
- **File Touch List:**
  - `kernel/drivers/vga.sn`: VGA text driver at `0xB8000` with cursor control and scrolling.
  - `kernel/drivers/uart.sn`: 16550 UART driver at `0x3F8` with 115200 baud init, TX/RX, and string formatting.
  - `kernel/main.sn`: Kernel initialization printing banner messages to both drivers.
- **Acceptance Criteria:**
  - Booting kernel in QEMU displays messages in both the VGA text buffer and terminal stdout via `-serial stdio`.

---

### Slice 5: Freestanding Runtime Profile, Panic Handler & Memory Provider

- **Objective:** Provide runtime memory primitives, deterministic panic management, and pluggable kernel page allocation.
- **Dependencies:** Slice 4.
- **File Touch List:**
  - `compiler/runtime_freestanding.c`: Lightweight `sn_memcpy`, `sn_memset`, `sn_memmove`, and lane bump allocators.
  - `kernel/panic.sn`: `@panic_handler fn panic(file: str, line: int, msg: str) -> !` dumping state to UART/VGA before halting.
  - `kernel/memory.sn`: `sn_set_memory_provider(alloc_fn, free_fn)` contract connecting physical frame allocators to SNlang's elastic pinned lanes and `std.pool`.
- **Acceptance Criteria:**
  - Deliberately triggering an assertion failure invokes `@panic_handler`, outputs file and line diagnostics to serial, and cleanly halts the CPU.

---

### Slice 6: Automated QEMU Test Harness, ISA Debug Exit & CI Suite

- **Objective:** Integrate automated bare-metal verification into continuous integration.
- **Dependencies:** Slice 5.
- **File Touch List:**
  - `kernel/arch/x86_64/exit.sn`: Hardware exit signaling functions (`qemu_exit`).
  - `scripts/qemu_runner.py`: Python CI test harness with timeout watchdog and assertion parser.
  - `tests/baremetal/test_boot.sn`: Integration test confirming boot, memory access, and clean exit.
  - `tests/baremetal/test_uart.sn`: Integration test verifying UART echo and string formatting.
  - `tests/baremetal/test_mmio.sn`: Integration test verifying volatile semantics.
  - `Makefile`: Build targets `make baremetal`, `make qemu-test`.
- **Acceptance Criteria:**
  - Executing `python3 scripts/qemu_runner.py --kernel target/kernel.elf` completes in < 2 seconds, asserting all test cases and exiting with status 0.
