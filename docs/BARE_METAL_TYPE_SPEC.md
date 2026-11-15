# SNlang Bare-Metal Kernel Expansion: Formal Type & Language Specification
## Architecture, Typing Semantics, Escape Analysis, and Machine Lowering
**Document Version:** 1.0.0-PROPOSAL  
**Author:** `@type_designer` (Type Theorist & Language Designer)  
**Status:** Canonical Formal Specification  
**Target Subsystems:** `compiler/src/types.rs`, `compiler/src/ast.rs`, `compiler/src/parser.rs`, `compiler/src/check.rs`, `compiler/src/ir.rs`, `compiler/src/llvm.rs`

---

## 1. Executive Summary & Architectural Overview

This specification formally establishes the type-theoretic foundation, static safety rules, escape analysis discipline, and LLVM lowering pipeline for SNlang's bare-metal kernel expansion.

To support high-performance, verifiable bare-metal kernel development (operating systems, hypervisors, device drivers, and firmware), the SNlang compiler introduces three fundamental language capabilities:
1. **`mmio<T>` / `volatile<T>` Hardware Register Semantics**: First-class, zero-cost affine abstractions for memory-mapped I/O with strictly enforced scalar width typing, mandatory explicit `.read()` and `.write()` operations, compiler escape analysis preventing pointers/references (`E-MMIO-ESCAPE`), GC capture rejection, and sequential consistency guarantees via LLVM `load volatile` / `store volatile` emission.
2. **GC Link-Out Rule in `--freestanding` Mode**: Statically eliminating the garbage collector runtime in kernel targets, intercepting and rejecting any `new T()` expression at compile-time with diagnostic `E-NO-GC-IN-FREESTANDING`, directing developers to deterministic alternatives (`region { }`, pinned lanes, or `std.pool`), and guaranteeing zero GC symbol emissions into the final ELF binary.
3. **Extended Inline Assembly `asm { ... }`**: A typed, declarative inline assembly block supporting input/output register constraints (`in("rax") val`, `out("al") res`), clobbers (`clobber("memory")`), operand bitwidth verification, and mandatory volatile LLVM IR lowering (`asm sideeffect`) to prevent optimizer dead-code elimination.

---

## 2. `mmio<T>` / `volatile<T>` Type Semantics

### 2.1 Domain Problem & Motivation
In bare-metal kernel engineering, memory-mapped I/O (MMIO) registers map directly to physical hardware buses (PCIe, APB, AXI, LPC). Accessing device registers using raw pointers or ordinary variable reads/writes exhibits three catastrophic hazards under modern optimizing compilers:
1. **Optimizer Dead-Store & Dead-Code Elimination**: Standard memory operations assume the C abstract machine, where two writes in sequence can be coalesced into the final write (Dead Store Elimination, DSE), or repeated reads can be commoned into a single CPU register read (Common Subexpression Elimination / Global Value Numbering, GVN). In hardware, writing twice to a command register triggers two discrete bus pulses, and reading an interrupt register clears pending interrupts. Coalescing or eliding accesses corrupts peripheral state machines.
2. **Access Width & Alignment Integrity**: Peripheral bus interconnects decode byte-enable signals based on access width. Accessing a 32-bit MMIO register via two 16-bit loads or byte-wise `memcpy` triggers a hardware Bus Fault or PCIe Target Abort.
3. **Pointers, Borrows & GC Pointer Escapes**: If code takes a pointer (`ptr`) or borrow (`ref<mmio<T>>`) of an MMIO register, that reference can escape into data structures, closures, or GC tracking tables. If the garbage collector sweeps an MMIO address as a heap object, it triggers random bus reads or bus errors.

SNlang addresses this with the **`mmio<T>`** affine scalar hardware type.

---

### 2.2 Formal Syntax & Typing Rules

#### Grammar
```ebnf
MmioType     ::= "mmio" "<" ScalarWidth ">"
               | "volatile" "<" ScalarWidth ">"
ScalarWidth  ::= "u8" | "u16" | "u32" | "u64"
               | "i8" | "i16" | "i32" | "i64"
```

#### Well-Formedness Typing Judgments
Let $\mathcal{S}_{\text{scalar}} = \{\mathtt{u8}, \mathtt{u16}, \mathtt{u32}, \mathtt{u64}, \mathtt{i8}, \mathtt{i16}, \mathtt{i32}, \mathtt{i64}\}$.

$$\frac{\tau \in \mathcal{S}_{\text{scalar}}}{\Delta \vdash \mathtt{mmio}\langle \tau \rangle \ \mathbf{type}}$$

$$\frac{\tau \notin \mathcal{S}_{\text{scalar}}}{\Delta \vdash \mathtt{mmio}\langle \tau \rangle \ \mathbf{ill\text{-}formed} \implies \mathtt{E\text{-}MMIO\text{-}INVALID\text{-}TYPE}}$$

When $\tau \notin \mathcal{S}_{\text{scalar}}$ (e.g., `mmio<str>`, `mmio<bool>`, `mmio<Point>`, `mmio<[u8; 4]>`), compilation halts immediately:
```text
error: invalid type argument for 'mmio<T>' [E-MMIO-INVALID-TYPE]
  --> kernel/uart.sn:14:15
   |
14 |     let reg = mmio<MyRecord>::at(0x1000_0000)
   |               ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^
   |
   = note: mmio<T> is restricted to primitive scalar widths: u8, u16, u32, u64 (or signed i8, i16, i32, i64).
   = help: found aggregate type 'MyRecord'. Use primitive scalar registers for hardware MMIO.
```

#### Instantiation & Memory Layout
At runtime, `mmio<T>` occupies exactly a single machine word: an unsigned 64-bit address (`uptr` / `u64`). It carries zero heap overhead, zero runtime metadata, and zero indirection.

Instantiation occurs via the static constructor `mmio<T>::at(address: u64)`:
```snlang
// Bound to standard 16550 UART COM1 base (0x3F8)
let uart_rbr = mmio<u8>::at(0x03F8)

// Bound to Local APIC End-Of-Interrupt register (0xFEE000B0)
let apic_eoi = mmio<u32>::at(0xFEE000B0)
```

---

### 2.3 Operations & Strict Non-Deref Interface

To prevent unintended reads or writes, `mmio<T>` does not permit implicit dereferencing, assignment via `=`, or arithmetic operations:

| Operation | Signature | Semantics & Verification |
|---|---|---|
| `reg.read()` | `fn read() -> T` | Performs single volatile load of bitwidth $\mathrm{sizeof}(T)$ from bound address. Returns scalar value $T$. |
| `reg.write(val)` | `fn write(val: T) -> void` | Performs single volatile store of bitwidth $\mathrm{sizeof}(T)$ with value `val`. `val` must match type $T$ exactly. |
| `reg.address()` | `fn address() -> u64` | Returns underlying 64-bit memory address (read-only introspection). |

#### Syntactic Rejections
1. **Implicit Dereference**: `*reg` $\implies$ Compile error: `cannot dereference mmio<T>: use reg.read() for explicit volatile hardware bus access`.
2. **Direct Scalar Assignment**: `reg = 0x42` $\implies$ Compile error: `cannot assign directly to mmio<T>: use reg.write(val)`.
3. **Compound Arithmetic**: `reg += 1` or `reg |= 0x02` $\implies$ Compile error: `compound assignment not supported on mmio<T>: read and write must be explicit: reg.write(reg.read() | 0x02)`.

---

### 2.4 Escape Analysis & Concurrency Invariants (`E-MMIO-ESCAPE`)

Hardware device registers represent non-fungible physical bus endpoints. Allowing an `mmio<T>` value to escape into arbitrary memory or un-synchronized concurrency primitives violates memory safety and hardware integrity.

The type checker enforces four strict escape prevention rules:

#### Rule 1: Reference and Borrow Prohibition
Taking the reference or address of an `mmio<T>` is strictly forbidden:
$$\frac{\Gamma \vdash e : \mathtt{mmio}\langle \tau \rangle}{\Gamma \vdash \&e : \bot \quad [\mathtt{E\text{-}MMIO\text{-}ESCAPE}]}$$
$$\frac{\Gamma \vdash e : \mathtt{mmio}\langle \tau \rangle}{\Gamma \vdash \mathtt{ref}\langle \mathtt{mmio}\langle \tau \rangle \rangle : \bot \quad [\mathtt{E\text{-}MMIO\text{-}ESCAPE}]}$$
$$\frac{\Gamma \vdash e : \mathtt{mmio}\langle \tau \rangle}{\Gamma \vdash \mathtt{mut}\langle \mathtt{mmio}\langle \tau \rangle \rangle : \bot \quad [\mathtt{E\text{-}MMIO\text{-}ESCAPE}]}$$

Diagnostic:
```text
error: cannot take reference of MMIO register 'reg' [E-MMIO-ESCAPE]
  --> kernel/drivers/vga.sn:28:16
   |
28 |     let r_ref = &vga_ctrl
   |                 ^^^^^^^^^
   |
   = note: taking references (ref<T>, mut<T>, ptr) to MMIO registers is forbidden.
   = note: device registers must remain affine values and cannot be aliased by pointers.
```

#### Rule 2: GC Heap Capture Prohibition
An `mmio<T>` field cannot be declared inside a heap-allocated GC `blueprint`:
$$\frac{\mathtt{blueprint}\ B\ \{\ \dots,\ f : \mathtt{mmio}\langle \tau \rangle,\ \dots\ \}}{\vdash B \ \mathbf{contains\ MMIO} \implies \forall \mathtt{new}\ B() : \bot \quad [\mathtt{E\text{-}MMIO\text{-}ESCAPE}]}$$

Diagnostic:
```text
error: cannot allocate blueprint containing MMIO register on GC heap [E-MMIO-ESCAPE]
  --> kernel/drivers/pci.sn:45:15
   |
45 |     let dev = new PciDevice(bus, slot)
   |               ^^^^^^^^^^^^^^^^^^^^^^^^
   |
   = note: blueprint 'PciDevice' contains field 'cmd_reg: mmio<u16>'.
   = note: MMIO registers cannot be managed by the GC collector or allocated on the dynamic heap.
   = help: define 'PciDevice' as a stack-allocated 'record' or manage via 'region { }' / 'pinned lane'.
```

#### Rule 3: Closure Capture Prohibition
If a lambda expression captures an `mmio<T>` and the lambda escapes the local stack frame:
```snlang
fn bad_capture(reg: mmio<u32>) -> fn() -> u32 {
    return || reg.read() // Compile error: E-MMIO-ESCAPE
}
```
Diagnostic:
```text
error: closure captures MMIO register 'reg' across escaping boundary [E-MMIO-ESCAPE]
  --> kernel/driver.sn:19:12
   |
19 |     return || reg.read()
   |            ^^^^^^^^^^^^^
   |
   = note: MMIO registers cannot escape the declaring stack frame via heap closures.
```

#### Rule 4: Thread Concurrency & Channel Transfer Prohibition
`mmio<T>` does not implement `Send` or `Sync`.
- Passing `mmio<T>` into `spawn` or `go`: **Compile Error `E-MMIO-ESCAPE`**.
- Sending `mmio<T>` through a channel `chan<mmio<T>>`: **Compile Error `E-MMIO-ESCAPE`**.
```text
error: cannot send or capture un-synchronized mmio<T> across threads [E-MMIO-ESCAPE]
  --> kernel/sched.sn:82:11
   |
82 |     spawn { timer_reg.write(0x01) }
   |           ^^^^^^^^^^^^^^^^^^^^^^^^^
   |
   = note: device registers cannot be captured into thread tasks without explicit synchronization.
   = help: wrap device access in a driver lock or confine MMIO operations to the dedicated driver thread.
```

---

### 2.5 LLVM Lowering & Machine Optimization Guarantees

#### Instruction Mapping
In `compiler/src/ir.rs` and `compiler/src/llvm.rs`:
- An `mmio<T>` variable is lowered to LLVM IR type `ptr`.
- `reg.read()` is emitted as:
  ```llvm
  %val = load volatile <llvm_t>, ptr %reg_addr, align <align_bytes>
  ```
- `reg.write(val)` is emitted as:
  ```llvm
  store volatile <llvm_t> %val, ptr %reg_addr, align <align_bytes>
  ```

Where `<llvm_t>` and `<align_bytes>` are mapped strictly according to scalar width:
| SNlang Type | LLVM Type | Alignment Requirement | Emitted Load Instruction | Emitted Store Instruction |
|---|---|---|---|---|
| `mmio<u8>` | `i8` | `align 1` | `load volatile i8, ptr %p, align 1` | `store volatile i8 %v, ptr %p, align 1` |
| `mmio<u16>` | `i16` | `align 2` | `load volatile i16, ptr %p, align 2` | `store volatile i16 %v, ptr %p, align 2` |
| `mmio<u32>` | `i32` | `align 4` | `load volatile i32, ptr %p, align 4` | `store volatile i32 %v, ptr %p, align 4` |
| `mmio<u64>` | `i64` | `align 8` | `load volatile i64, ptr %p, align 8` | `store volatile i64 %v, ptr %p, align 8` |

#### Optimization Barrier Invariants (LLVM Optimizer Constraints)
Under the LLVM IR Language Reference Specification (§"Volatile Memory Accesses"):
1. **Zero Dead-Store Elimination (DSE)**: Consecutive stores to the same MMIO register are guaranteed to be emitted as discrete machine store instructions in exact program order.
2. **Zero Redundancy Elimination (GVN / CSE)**: Consecutive loads from the same MMIO address cannot be replaced with a cached scalar value in a CPU register.
3. **Zero Loop-Invariant Code Motion (LICM)**: A polling loop:
   ```snlang
   while ((status_reg.read() & 0x01) == 0) { }
   ```
   guarantees that `status_reg.read()` will be re-executed on every loop iteration, preventing compiler-induced lockups.
4. **Program Order Preservation**: LLVM optimizer passes are forbidden from reordering volatile loads/stores relative to other volatile accesses within the same thread.

---

## 3. GC Link-Out Rule in `--freestanding` Mode

### 3.1 Motivation & Kernel Execution Profile
In hosted environments (macOS, Linux, Windows), SNlang applications link with `runtime.c`, which initializes a thread-safe garbage collector (allocating through `mmap` or `malloc`), registers root pointers, handles GC safepoints, and intercepts signals.

In bare-metal kernel development:
1. Physical memory paging is uninitialized or manually configured by page-table mappings.
2. Standard C runtime (`libc`, `pthreads`, `mmap`, `fprintf`) does not exist.
3. Garbage collector stop-the-world pauses and background sweeps are unacceptable during interrupt dispatch or scheduler task preemption.
4. Binary images must link cleanly with `-ffreestanding -nostdlib`.

Therefore, SNlang introduces the `--freestanding` compilation profile.

---

### 3.2 Compile-Time Rejection of GC Heap Constructors (`E-NO-GC-IN-FREESTANDING`)

#### Compiler Flag
`snc --freestanding <source.sn> -o <target.elf>` sets compiler state flag `Cx.is_freestanding = true`.

#### Semantic Invariant
When `is_freestanding == true`, the `new` keyword and all associated dynamic GC heap allocations are strictly banned:

$$\frac{\kappa = \mathtt{freestanding} \quad \Gamma \vdash \mathtt{new}\ T(\vec{e})}{\Gamma \vdash \bot \quad [\mathtt{E\text{-}NO\text{-}GC\text{-}IN\text{-}FREESTANDING}]}$$

#### Diagnostic Specification
When a developer invokes `new T(...)` in a freestanding build, compilation immediately aborts with:
```text
error: dynamic GC heap allocation 'new T()' is forbidden in --freestanding mode [E-NO-GC-IN-FREESTANDING]
  --> kernel/sched/task.sn:54:16
   |
54 |     let task = new Task(pid, "init")
   |                ^^^^^^^^^^^^^^^^^^^^^
   |
   = note: bare-metal kernel targets operate without a runtime garbage collector.
   = help: use stack allocation, 'region { }' elastic arenas, pinned lanes, or generational 'std.pool'.
```

---

### 3.3 Safe Freestanding Memory Paradigms

The compiler diagnostic actively guides the developer toward SNlang's three existing deterministic memory primitives:

```
+-----------------------------------------------------------------------------------------+
|                                    FREESTANDING MEMORY                                  |
+------------------------------+---------------------------+------------------------------+
| 1. Elastic Arenas            | 2. Pinned Memory Lanes    | 3. Generational Pools        |
|    `region { ... }`          |    `std.arena` / Carving  |    `std.pool` / `pool<T>`    |
| - Fast stack-like bump alloc | - Fixed contiguous frames | - Generational handles       |
| - Instantaneous rollback     | - Pinned DMA / PageTables | - Intrusive O(1) freelist    |
| - Zero fragmentation         | - Never paged or moved    | - Temporal safety (no UAF)   |
+------------------------------+---------------------------+------------------------------+
```

1. **Stack Allocation (`record`)**:
   Stack-allocated records (`record TaskControlBlock { ... }`) require zero dynamic allocation and are reclaimed upon function exit.
2. **Elastic Arenas (`region { ... }`)**:
   Scoped arena blocks automatically record the current lane watermark at block entry and rollback the watermark upon exit:
   ```snlang
   region {
       let buf = lane_alloc(0, 4096)
       // parse ACPI tables...
   } // Watermark instantly rolled back; zero leaks, zero GC
   ```
3. **Pinned Lanes (`std.arena` / Lane Carving)**:
   Physical frames partitioned for device driver descriptors, packet rings, and page tables.
4. **Generational Slot Vaults (`std.pool` / `pool<T>`)**:
   For long-lived dynamic kernel objects (e.g., process table, file descriptors), `std.pool` allocates from a pre-allocated static buffer using 64-bit generational handles `(slot, gen)`. Access through stale handles is caught at runtime without any GC overhead.

---

### 3.4 Static Link-Out Guarantee & Symbol Elimination

When compiling under `--freestanding`:
1. **Symbol Omission**: The LLVM backend (`llvm.rs`) conditionally suppresses all declarations and calls to the GC subsystem:
   - `@sn_obj_new`
   - `@sn_gc_track`
   - `@sn_gc_safepoint`
   - `@sn_gc_init`
   - `@sn_gc_collect`
   - `@sn_gc_register_root`
   - `@sn_gc_enter` / `@sn_gc_leave`
2. **Runtime Slicing**: The kernel links against `runtime_freestanding.c` instead of `runtime.c`. `runtime_freestanding.c` contains only core string formatting, panic handlers, memory copy/set intrinsics (`memcpy`, `memset`, `memmove`), and lane allocation watermarks.
3. **Zero Undefined Symbols Guarantee**: Running:
   ```bash
   nm -u kernel.elf | grep "sn_gc"
   ```
   is guaranteed to produce an empty output (0 undefined GC symbols), ensuring seamless linkage with freestanding bare-metal linkers (`ld.lld -nostdlib -static`).

---

## 4. Extended Inline Assembly `asm { ... }`

### 4.1 Motivation & Legacy Inadequacies
Kernel development requires direct access to machine instructions that cannot be expressed in standard high-level language constructs:
- Reading and writing control registers (`cr0`, `cr2`, `cr3`, `cr4`).
- Managing CPU interrupts (`cli`, `sti`, `hlt`).
- Invoking legacy port I/O (`inb`, `outb`, `inl`, `outl`).
- Executing specialized CPU instructions (`cpuid`, `rdtsc`, `invlpg`, `wrmsr`, `rdmsr`).

The prototype assembly statement (`asm "..." : clobbers`) was insufficient: it lacked typed input/output operand bindings, did not check operand widths, and was vulnerable to dead-code elimination by aggressive compiler optimizations.

---

### 4.2 Formal Concrete Grammar & AST

```ebnf
AsmStmt         ::= "asm" "{" AsmTemplate ( "," AsmClause )* "}"
AsmExpr         ::= "asm" "{" AsmTemplate ( "," AsmClause )* "}"

AsmTemplate     ::= StringLiteral
AsmClause       ::= AsmInput
                  | AsmOutput
                  | AsmInOut
                  | AsmClobber
                  | AsmOption

AsmInput        ::= "in" "(" ConstraintString ")" Expression
AsmOutput       ::= "out" "(" ConstraintString ")" LValue
AsmInOut        ::= "inout" "(" ConstraintString ")" Identifier
AsmClobber      ::= "clobber" "(" StringLiteral ")"
AsmOption       ::= "options" "(" Identifier ( "," Identifier )* ")"

ConstraintString::= StringLiteral
```

#### Rust AST Data Structures (`compiler/src/ast.rs`)
```rust
#[derive(Clone, Debug)]
pub struct AsmBlock {
    pub template: String,
    pub inputs: Vec<AsmOperand>,
    pub outputs: Vec<AsmOperand>,
    pub inouts: Vec<AsmOperand>,
    pub clobbers: Vec<String>,
    pub is_volatile: bool,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct AsmOperand {
    pub constraint: String,
    pub expr: Expr,
    pub span: Span,
}
```

---

### 4.3 Register Constraints & Constraint Modifiers

Constraint strings specify register classes and operand placement:

| Constraint | Register Class (x86_64) | Description | Allowed Types |
|---|---|---|---|
| `"r"` | Any General Purpose Register | Selected automatically by LLVM register allocator | `u8`, `u16`, `u32`, `u64`, `int`, `ptr` |
| `"rax"` / `"a"` | `rax`, `eax`, `ax`, `al` | Accumulator register | Sized to variable width |
| `"rbx"` / `"b"` | `rbx`, `ebx`, `bx`, `bl` | Base register | Sized to variable width |
| `"rcx"` / `"c"` | `rcx`, `ecx`, `cx`, `cl` | Counter register (shifts, loop counts) | Sized to variable width |
| `"rdx"` / `"d"` | `rdx`, `edx`, `dx`, `dl` | Data register (I/O ports, high multiply) | Sized to variable width |
| `"rsi"` / `"S"` | `rsi`, `esi`, `si` | Source Index register | Sized to variable width |
| `"rdi"` / `"D"` | `rdi`, `edi`, `di` | Destination Index register | Sized to variable width |
| `"m"` | Memory Operand | Direct memory reference address `[base + offset]` | Any pointer or lvalue |
| `"i"` | Immediate Integer | Constant integer known at compile-time | Integer constants |

#### Output Modifiers
- `"="`: Write-only output operand. The initial value in the register is discarded; the assembly instruction writes a new value.
- `"+"`: Read-write (inout) operand. The register is loaded with the input value and subsequently updated with the output value.
- `"&"`: Early-clobber operand. The register is modified before the instruction finishes reading all inputs; the compiler must not allocate an input operand in the same register.

#### Clobber Declarations
- `clobber("memory")`: **Mandatory for memory barriers**. Informs the compiler that arbitrary memory may be read or written by the assembly block. Prevents LLVM from reordering non-volatile loads or stores across the inline assembly call.
- `clobber("cc")`: Informs the compiler that condition code flags (`rflags`) are overwritten.

---

### 4.4 Static Type Checking & Operand Bitwidth Matrix (`E-ASM-OPERAND-WIDTH`)

To prevent register corruption, the type checker (`check.rs`) cross-validates each operand's type width against its register constraint:

$$\frac{\mathrm{Width}(\tau) \neq \mathrm{ConstraintWidth}(C)}{\Gamma \vdash \mathtt{in}(C)\ e : \bot \quad [\mathtt{E\text{-}ASM\text{-}OPERAND\text{-}WIDTH}]}$$

| Constraint Specifier | Target Width | Permitted SNlang Types | Type Diagnostic if Mismatched |
|---|---|---|---|
| `"al"`, `"bl"`, `"cl"`, `"dl"` | 8 bits | `u8`, `i8`, `byte`, `bool` | `E-ASM-OPERAND-WIDTH: 8-bit constraint requires 8-bit operand, found '{T}'` |
| `"ax"`, `"bx"`, `"cx"`, `"dx"` | 16 bits | `u16`, `i16` | `E-ASM-OPERAND-WIDTH: 16-bit constraint requires 16-bit operand, found '{T}'` |
| `"eax"`, `"ebx"`, `"ecx"`, `"edx"` | 32 bits | `u32`, `i32` | `E-ASM-OPERAND-WIDTH: 32-bit constraint requires 32-bit operand, found '{T}'` |
| `"rax"`, `"rbx"`, `"rcx"`, `"rdx"` | 64 bits | `u64`, `i64`, `int`, `ptr` | `E-ASM-OPERAND-WIDTH: 64-bit constraint requires 64-bit operand, found '{T}'` |

Diagnostic example:
```text
error: asm constraint bitwidth mismatch [E-ASM-OPERAND-WIDTH]
  --> kernel/arch/x86/io.sn:18:9
   |
18 |         in("dx") port_64
   |         ^^^^^^^^^^^^^^^^
   |
   = note: constraint "dx" designates 16-bit register 'dx'.
   = note: operand 'port_64' has type 'u64' (64 bits).
   = help: cast operand to 'u16' or use a 64-bit constraint ("rdx").
```

---

### 4.5 LLVM Lowering & Volatile Asm Emission

#### Volatile Asm Guarantee
In bare-metal kernel code, inline assembly operations almost always trigger processor state changes (`cli`, `sti`, `hlt`, `outb`, page invalidation) even if they produce no return values.
If an asm block were emitted without the `sideeffect` qualifier, LLVM's optimizer would treat it as a pure function and delete it if its outputs are unused.

Therefore, **all inline assembly in SNlang is emitted with the LLVM `sideeffect` attribute**:

```llvm
; Statement format (void return)
call void asm sideeffect "<template_escaped>", "<constraint_string>"(<operands>)

; Expression format (scalar return)
%res = call <ret_llvm_t> asm sideeffect "<template_escaped>", "<constraint_string>"(<operands>)

; Expression format (multiple outputs / struct return)
%res = call { i32, i32 } asm sideeffect "<template_escaped>", "<constraint_string>"(<operands>)
```

#### Constraint String Synthesis
The compiler translates SNlang constraints into standard LLVM inline assembly constraint format:
1. Outputs are prepended with `"="` (or `"+"` for inouts) and listed first.
2. Inputs follow.
3. Clobbers are wrapped in `"~{...}"` and appended.

For example:
```snlang
asm {
    "inb %1, %0"
    out("=al") val,
    in("dx") port,
    clobber("memory")
}
```
Lowers to:
```llvm
%0 = call i8 asm sideeffect "inb $1, $0", "={al},{dx},~{memory}"(i16 %port)
```

---

## 5. End-to-End Concrete Implementations & Verification

### 5.1 VGA Text Console Driver (0xB8000) using `mmio<u8>`
```snlang
// File: kernel/drivers/vga.sn
// Compile: snc --freestanding kernel/drivers/vga.sn

record VgaConsole {
    base_addr: u64,
    mut col: u32,
    mut row: u32,
    color_byte: u8
}

fn vga_init() -> VgaConsole {
    return VgaConsole {
        base_addr: 0x000B8000,
        col: 0,
        row: 0,
        color_byte: 0x0F // White on Black
    }
}

fn vga_write_char(vga: mut<VgaConsole>, c: u8) {
    if (c == 10) { // Newline '\n'
        vga.col = 0
        vga.row = vga.row + 1
        return
    }

    let offset = (vga.row * 80 + vga.col) * 2
    let char_cell = mmio<u8>::at(vga.base_addr + cast(offset, u64))
    let attr_cell = mmio<u8>::at(vga.base_addr + cast(offset + 1, u64))

    // Explicit volatile stores to hardware buffer
    char_cell.write(c)
    attr_cell.write(vga.color_byte)

    vga.col = vga.col + 1
    if (vga.col >= 80) {
        vga.col = 0
        vga.row = vga.row + 1
    }
}
```

---

### 5.2 16550 UART Serial & QEMU Debugcon Driver using `asm { ... }`
```snlang
// File: kernel/drivers/serial.sn

fn outb(port: u16, val: u8) {
    asm {
        "outb %0, %1"
        in("al") val,
        in("dx") port,
        clobber("memory")
    }
}

fn inb(port: u16) -> u8 {
    let mut val: u8 = 0
    asm {
        "inb %1, %0"
        out("=al") val,
        in("dx") port,
        clobber("memory")
    }
    return val
}

// QEMU Debugcon exit port (0x501)
fn qemu_exit_success() -> ! {
    outb(0x0501, 0x31) // Exit code 0
    loop {
        asm {
            "cli; hlt"
            clobber("memory")
        }
    }
}
```

---

### 5.3 Bare-Metal Entry Point in `--freestanding` Mode
```snlang
// File: kernel/main.sn
// Invocation: snc --freestanding kernel/main.sn -o kernel.elf

fn kernel_main() -> ! {
    // 1. Initialize serial port
    outb(0x3F8 + 1, 0x00) // Disable interrupts
    outb(0x3F8 + 3, 0x80) // Enable DLAB
    outb(0x3F8 + 0, 0x03) // Set baud divisor 38400
    outb(0x3F8 + 1, 0x00)
    outb(0x3F8 + 3, 0x03) // 8 bits, no parity, one stop bit

    // 2. Deterministic non-GC memory allocation via region
    region {
        let page_buffer = lane_alloc(0, 4096)
        // Memory dynamically reserved without GC and rolled back on exit
    }

    // 3. Print success and cleanly poweroff QEMU
    qemu_exit_success()
}
```

---

### 5.4 Diagnostic Verification Matrix

| Test Scenario | Source Construct | Compiler Flag | Expected Diagnostic Code | Output Message Anchor |
|---|---|---|---|---|
| MMIO Non-Scalar Type | `mmio<bool>` / `mmio<str>` | any | `E-MMIO-INVALID-TYPE` | `mmio<T> is restricted to primitive scalar widths: u8, u16, u32, u64` |
| MMIO Reference Escape | `let p = &reg` | any | `E-MMIO-ESCAPE` | `cannot take reference of MMIO register 'reg'` |
| MMIO GC Heap Capture | `blueprint Dev { reg: mmio<u32> }; new Dev()` | any | `E-MMIO-ESCAPE` | `cannot allocate blueprint containing MMIO register on GC heap` |
| MMIO Cross-Thread Send | `spawn { reg.write(1) }` | any | `E-MMIO-ESCAPE` | `cannot send or capture un-synchronized mmio<T> across threads` |
| GC Constructor in Kernel | `new Task(1)` | `--freestanding` | `E-NO-GC-IN-FREESTANDING` | `dynamic GC heap allocation 'new T()' is forbidden in --freestanding mode` |
| Asm Register Width Mismatch| `in("al") port_u32` | any | `E-ASM-OPERAND-WIDTH` | `8-bit constraint requires 8-bit operand, found 'u32'` |
| Asm Memory Clobber | `asm { "cli" , clobber("memory") }` | any | Compiles successfully | LLVM IR emits `~{memory}` and `sideeffect` |

---

## 6. Conclusion & Swarm Handoff

This specification delivers the rigorous formal definitions required for SNlang's bare-metal kernel expansion:
- **Type Safety**: Affine `mmio<T>` guarantees zero reference escapes, zero GC leaks, and deterministic volatile bus I/O.
- **Freestanding Purity**: `--freestanding` with `E-NO-GC-IN-FREESTANDING` ensures zero GC overhead, completely omitting GC symbols and routing kernel allocations to `region { }`, pinned lanes, and `std.pool`.
- **Machine Expressivity**: Typed `asm { ... }` with register constraints and volatile LLVM emission guarantees safe, un-elided CPU-level control.

Ready for implementation by `@compiler_systems`, `@runtime_systems`, and verification by `@verifier_systems`.
