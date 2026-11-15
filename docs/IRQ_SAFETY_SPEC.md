# The Novel `irq fn` Static Interrupt Safety Discipline
## Formal Concurrency, Typing Invariants, and Interprocedural Verification for Bare-Metal SNlang

**Author:** `irq_theorist` (Concurrency & Interrupt Safety Theorist, SNlang Bare-Metal Kernel Expansion)  
**Status:** Canonical Formal Specification · Passed to Compiler & Systems Verifiers  
**Target Architecture:** Bare-Metal x86_64 (`-ffreestanding`, `-nostdlib`, Multiboot2) & AArch64  
**Compiler Integration:** `compiler/src/check.rs` (Interprocedural Effect Analysis), `compiler/src/ir.rs` (LLVM `x86_intr` Lowering)

---

## 1. Executive Summary & Foundational Motivation

In bare-metal operating system kernel development, hardware interrupt handlers (Interrupt Service Routines or ISRs) execute in an execution environment fundamentally distinct from normal schedulable threads. Classical languages (such as C, C++, and Assembly) treat interrupt handlers as ordinary void functions (`void isr_handler(void)` or functions decorated with `__attribute__((interrupt))`). Under this model, the compiler provides no static verification of what operations are physically and logically safe to execute during an interrupt.

This lack of static checking is the direct cause of three of the most notorious classes of operating system bugs:
1. **Uniprocessor Allocation Self-Deadlock:** An interrupt preempts a thread that is inside the kernel heap or frame allocator (holding an internal allocator spinlock or manipulating free-list pointers). The interrupt handler invokes dynamic memory allocation (`malloc`, `new`, or expanding a lane). Because the interrupted thread is frozen on the *same physical core*, the allocator lock can never be released. The core deadlocks permanently.
2. **Sleep-in-Atomic / Sleep-in-IRQ Scheduler Crashes:** An interrupt handler invokes a blocking primitive (such as sleeping on a mutex, receiving from a channel, or invoking `await`). Because an interrupt does not run in a schedulable Task Control Block (TCB) context, attempting to suspend the interrupt either corrupts the Interrupt Stack Table (IST), crashes the scheduler, or hangs the machine with hardware interrupts masked.
3. **Data Inconsistency Under Re-entrancy:** An interrupt handler mutates unsynchronized kernel structures or shared collections without atomic linearizability or priority masking, leading to torn reads, corrupted state, and silent memory corruption.

SNlang resolves these problems at compile-time by introducing the **`irq fn` Static Interrupt Safety Discipline**. Through a capability-and-effect typing lattice, compile-time static invariant checkers (`E-IRQ-ALLOC`, `E-IRQ-BLOCK`), and interprocedural purity propagation (`E-IRQ-INTERPROC`), SNlang guarantees that:
- Every `irq fn` is statically verified to be **strictly non-allocating** and **wait-free non-blocking**.
- All function calls within an `irq fn` are transitively verified across the entire interprocedural call graph.
- Permitted synchronization with the rest of the kernel is restricted to provably safe lock-free atomics (CAS, atomic load/store) and Single-Producer Single-Consumer (SPSC) ring buffers.
- Hardware interrupts compile down to zero-overhead machine instructions utilizing the native `x86_intr` calling convention with automated register saving and `iretq` execution.

---

## 2. Mathematical Modeling of the Interrupt Execution Context

### 2.1 Abstract Machine Architecture and Preemption Semantics

Let the bare-metal machine state be modeled as a tuple:
$$\mathcal{S} = \langle \mathcal{C}, \mathcal{M}, \mathcal{I}_{\text{ctrl}}, \mathcal{Q}_{\text{pending}} \rangle$$
where:
- $\mathcal{C} = \{c_0, c_1, \dots, c_{N-1}\}$ is the finite set of physical CPU cores.
- $\mathcal{M}: \text{Addr} \to \text{Byte}$ is the globally shared memory address space.
- $\mathcal{I}_{\text{ctrl}}$ is the programmable interrupt controller state (e.g., Local APIC / 8259 PIC / ARM GIC), tracking interrupt masks, vector priorities, and In-Service Registers (ISR).
- $\mathcal{Q}_{\text{pending}} \subseteq \text{Vector} \times \text{Core}$ is the multiset of pending hardware interrupt signals.

Each core $c \in \mathcal{C}$ maintains an execution context stack:
$$\Xi(c) = [\xi_0, \xi_1, \dots, \xi_k]$$
representing the stack of preempted contexts currently resident on core $c$.

A single execution context $\xi$ is defined as:
$$\xi = \langle \text{kind}, \text{ipl}, \text{tid}, \sigma_{\text{regs}}, \sigma_{\text{stack}} \rangle$$
where:
- $\text{kind} \in \{ \text{Thread}, \text{SoftIRQ}, \text{HardIRQ} \}$.
- $\text{ipl} \in \mathbb{N}_0$ is the hardware Interrupt Priority Level (IPL). Normal threads run at $\text{ipl} = 0$; deferred bottom halves run at $\text{ipl} = 1$; hardware interrupts execute at $\text{ipl} \ge 2$.
- $\text{tid} \in \text{TaskID} \cup \{ \bot \}$ is the active OS Task Identifier. For all $\text{HardIRQ}$ contexts, $\text{tid} = \bot$ (there is no schedulable thread control block).
- $\sigma_{\text{regs}}$ is the saved CPU hardware register state (instruction pointer `rip`, stack pointer `rsp`, processor flags `rflags`, general purpose registers `rax..r15`).
- $\sigma_{\text{stack}} \subseteq \text{MemoryRange}$ is the memory range assigned to the active stack. For normal threads, this is the thread kernel/user stack; for $\text{HardIRQ}$, this is the hardware-pinned Interrupt Stack Table (IST) frame $[S_{\text{IST\_base}}, S_{\text{IST\_top}}]$.

### 2.2 Asynchronous Preemption Transition

Hardware interruption is an asynchronous state transition triggered by $\mathcal{I}_{\text{ctrl}}$:

$$\frac{(\iota, c) \in \mathcal{Q}_{\text{pending}} \quad \text{prio}(\iota) > \text{top}(\Xi(c)).\text{ipl} \quad \text{IF}(\sigma_{\text{regs}}) = 1}{\langle \mathcal{C}[c \mapsto \Xi(c)], \mathcal{M}, \mathcal{I}_{\text{ctrl}}, \mathcal{Q}_{\text{pending}} \rangle \xrightarrow{\text{irq}(\iota)} \langle \mathcal{C}[c \mapsto \Xi(c) \circ \xi_{\text{new}}], \mathcal{M}, \mathcal{I}_{\text{ctrl}}', \mathcal{Q}_{\text{pending}} \setminus \{(\iota, c)\} \rangle}$$

where:
$$\xi_{\text{new}} = \langle \text{HardIRQ}(\iota), \text{prio}(\iota), \bot, \text{snapshot}(\text{CPU\_REGS}), [S_{\text{IST\_base}}, S_{\text{IST\_top}}] \rangle$$

Notice the crucial physical characteristics of this transition:
1. **Arbitrary Interruption Point:** The transition fires between any two discrete CPU clock cycles, regardless of what critical invariant or lock the underlying context $\text{top}(\Xi(c))$ was in the middle of executing.
2. **Context Absence ($\text{tid} = \bot$):** The new context has no schedulable identity. It cannot be registered in runqueues, cannot be assigned a time slice, and cannot be yielded.
3. **Dedicated Physical Stack:** The CPU hardware switch (via x86_64 TSS IST or ARM SP_EL1) forces `rsp` to point to a fixed, non-growable memory slice (typically 4096 or 8192 bytes).

The return transition is performed via the hardware instruction `iretq`:

$$\frac{\text{CPU executes } \texttt{iretq} \quad \text{top}(\Xi(c)) = \xi_{\text{top}} \quad \Xi(c) = \Xi'(c) \circ \xi_{\text{top}}}{\langle \mathcal{C}[c \mapsto \Xi(c)], \mathcal{M}, \mathcal{I}_{\text{ctrl}}, \mathcal{Q} \rangle \xrightarrow{\text{iretq}} \langle \mathcal{C}[c \mapsto \Xi'(c)], \mathcal{M}, \mathcal{I}_{\text{ctrl}}'', \mathcal{Q} \rangle}$$

which restores the preempted register state $\xi_{\text{top}}.\sigma_{\text{regs}}$ and drops the IPL back to $\text{top}(\Xi'(c)).\text{ipl}$.

---

## 3. Mathematical Proofs of Failure Modes & Formal Prohibitions

### 3.1 Theorem 1: Allocation Deadlock on Uniprocessor Preemption (The Allocator Inversion Hazard)

Let $\mathcal{A}$ be any dynamic memory allocator (global heap, pool allocator, or growing elastic lane).
Let $L_{\mathcal{A}}$ be the mutual exclusion primitive protecting allocator internal metadata (free list nodes, page directory tables, bump frontiers).

> **Theorem 1 (Uniprocessor Allocation Deadlock).**  
> If an interrupt handler $\xi_{\text{irq}} = \text{HardIRQ}(\iota)$ invoked on core $c$ performs a dynamic memory allocation that acquires $L_{\mathcal{A}}$, and the interrupted context $\xi_{\text{prev}}$ on core $c$ held $L_{\mathcal{A}}$ at the instant of preemption, the system enters an instantaneous, permanent deadlock.

*Proof.*  
1. By the preemption operational semantics, $\Xi(c) = [\dots, \xi_{\text{prev}}, \xi_{\text{irq}}]$. Core $c$ is a single physical execution pipeline executing instructions strictly from $\text{top}(\Xi(c)) = \xi_{\text{irq}}$.
2. By assumption, $\xi_{\text{prev}}$ holds lock $L_{\mathcal{A}}$:
   $$\text{Owner}(L_{\mathcal{A}}) = \xi_{\text{prev}}$$
3. $\xi_{\text{irq}}$ attempts to allocate memory:
   $$\text{Action}(\xi_{\text{irq}}) = \text{Acquire}(L_{\mathcal{A}})$$
4. The lock acquisition condition requires $L_{\mathcal{A}}$ to be free or released by its owner:
   $$\text{Progress}(\xi_{\text{irq}}) \implies \exists t' > t, \text{Owner}(L_{\mathcal{A}}) = \bot$$
5. However, $\xi_{\text{prev}}$ can only execute and release $L_{\mathcal{A}}$ if control returns to it via `iretq`:
   $$\text{Progress}(\xi_{\text{prev}}) \implies \xi_{\text{irq}} \text{ terminates and executes } \texttt{iretq}$$
6. Therefore:
   $$\text{Progress}(\xi_{\text{irq}}) \implies \text{Progress}(\xi_{\text{prev}}) \implies \text{Progress}(\xi_{\text{irq}})$$
   The wait-for graph contains the cycle $c \to c$.
7. Because core $c$ is executing $\xi_{\text{irq}}$ which spins or hangs awaiting $L_{\mathcal{A}}$, $\xi_{\text{prev}}$ can never be scheduled. Neither context can ever make progress. The core is permanently deadlocked. $\blacksquare$

**Corollary 1.1:** *No runtime locking scheme (even re-entrant ticket locks or MCS locks) can resolve this deadlock on a single core without disabling interrupts across all allocation paths in all threads.* Since disabling interrupts globally across all memory operations introduces unacceptable interrupt latency jitter ($> 50\,\mu\text{s}$), dynamic allocation must be **statically prohibited** inside interrupt handlers.

---

### 3.2 Theorem 2: Schedulable Context Absence & Sleep-in-Atomic

Let the kernel scheduler state machine be $\mathcal{K}_{\text{sched}}: (\text{TaskID} \to \text{TCB}) \times \text{RunQueue}$.

> **Theorem 2 (Scheduler Corruption under Block-in-IRQ).**  
> Any invocation of a blocking synchronization operation ($\text{block}$, $\text{yield}$, $\text{wait}$, $\text{await}$) within context $\xi$ where $\xi.\text{tid} = \bot$ leads to an undefined state transition or catastrophic CPU freeze.

*Proof.*  
1. The formal precondition of a blocking operation $\text{block}(cond)$ is:
   $$\text{pre}(\text{block}) \equiv \xi.\text{tid} \neq \bot \land \exists \tau \in \text{TaskID}, \tau = \xi.\text{tid}$$
2. The operational transition of $\text{block}$ updates the task state in the scheduler:
   $$\mathcal{K}_{\text{sched}}' = \mathcal{K}_{\text{sched}}[\xi.\text{tid} \mapsto \text{Blocked}(cond)]$$
   and removes $\xi.\text{tid}$ from the run queue:
   $$\text{RunQueue}' = \text{RunQueue} \setminus \{\xi.\text{tid}\}$$
3. For an interrupt context $\xi_{\text{irq}}$, $\xi_{\text{irq}}.\text{tid} = \bot$. The mapping $\mathcal{K}_{\text{sched}}[\bot]$ is undefined.
4. If the scheduler attempts to deschedule the underlying preempted thread $\xi_{\text{prev}}.\text{tid}$:
   - The CPU stack pointer `rsp` currently points into the hardware IST stack $[S_{\text{IST\_base}}, S_{\text{IST\_top}}]$.
   - Saving registers into $\xi_{\text{prev}}.\text{tcb}$ saves the *interrupt frame* rather than the thread frame, corrupting the thread control block.
   - The hardware interrupt controller $\mathcal{I}_{\text{ctrl}}$ still has vector $\iota$ marked in its In-Service Register (ISR). Because no `EOI` (End-Of-Interrupt) or `iretq` was executed, the hardware blocks all future interrupts of priority $\le \text{prio}(\iota)$ across the entire system.
5. If the CPU instead executes a halt instruction `hlt` waiting for an event while inside the interrupt, and interrupts are masked, no external signal can ever wake the processor. The system freezes permanently. $\blacksquare$

---

### 3.3 Theorem 3: Lock-Acquisition Priority Inversion Deadlock

Suppose two execution contexts $\xi_{\text{low}}$ (thread, $\text{ipl} = 0$) and $\xi_{\text{high}}$ (IRQ, $\text{ipl} = 2$) share a non-recursive spinlock $S$.

> **Theorem 3 (Priority Inversion Deadlock with Spinlocks).**  
> If an interrupt handler acquires spinlock $S$, and a lower-priority context on the same core can acquire $S$ without disabling interrupts, the system is susceptible to unrecoverable deadlock.

*Proof.*  
Directly from Theorem 1: If $\xi_{\text{low}}$ acquires $S$, and before releasing $S$ is preempted by $\xi_{\text{high}}$ on the same core, $\xi_{\text{high}}$ will spin on $S$ indefinitely. Since $\xi_{\text{high}}$ never yields the CPU, $\xi_{\text{low}}$ never executes, and $S$ is never unlocked. $\blacksquare$

---

## 4. The Capability & Context Typing Lattice

To statically eliminate these failure modes, SNlang formalizes execution contexts as a bounded join-semilattice:

$$\mathbb{C} = \langle \mathcal{K}, \sqsubseteq, \sqcup, \sqcap, \bot, \top \rangle$$

where:
$$\mathcal{K} = \{ \text{Pure}, \text{IRQ}, \text{SoftIRQ}, \text{Thread}, \text{Any} \}$$

The partial order relation $\sqsubseteq$ represents **privilege and execution freedom** (or inversely, the severity of execution restrictions):

$$\bot \equiv \text{Pure} \sqsubset \text{IRQ} \sqsubset \text{SoftIRQ} \sqsubset \text{Thread} \sqsubset \text{Any} \equiv \top$$

```
               Top (Any)
                   │
              Thread (ipl=0)
                   │
             SoftIRQ (ipl=1)
                   │
               IRQ (ipl>=2)
                   │
              Pure (bot)
```

### Interpretation of Lattice Elements:
- $\text{Pure}$: Total mathematical purity. No side-effects, no I/O, no allocation, no synchronization. Callable from any context.
- $\text{IRQ}$: Hardware interrupt service routine context ($\text{ipl} \ge 2$). May read/write MMIO registers, execute atomics, append to SPSC rings. **Strictly prohibited from allocating or blocking.**
- $\text{SoftIRQ}$: Deferred interrupt / bottom-half context ($\text{ipl} = 1$). May execute heavier lock-free algorithms, signal threads.
- $\text{Thread}$: Normal kernel or user schedulable thread ($\text{ipl} = 0$). Has full capability set: dynamic allocation, sleeping mutexes, channels, `await`.

### Context Subtyping & Call Capability Rule

A function $f$ has a declared or inferred context capability $\kappa(f) \in \mathcal{K}$.

$$\frac{\Gamma \vdash f : (\tau_1, \dots, \tau_n) \xrightarrow{\kappa(f)} \tau_{\text{ret}} \quad \kappa(f) \sqsubseteq \kappa_{\text{caller}}}{\Gamma; \kappa_{\text{caller}} \vdash f(e_1, \dots, e_n) : \tau_{\text{ret}}}$$

**The Fundamental Invariant:**  
Inside an `irq fn`, the caller context is fixed to $\kappa_{\text{caller}} = \text{IRQ}$.  
Therefore, a function call $g(\dots)$ inside an `irq fn` is well-typed **if and only if**:
$$\kappa(g) \sqsubseteq \text{IRQ}$$
Any attempt to invoke a function $g$ where $\kappa(g) = \text{Thread}$ or $\kappa(g) = \text{SoftIRQ}$ is statically rejected by the type checker.

---

## 5. The Three Static Invariants Checked at Compile-Time

SNlang enforces three orthogonal, decidable invariants on all AST nodes enclosed within an `irq fn` or any function in its transitive call graph.

---

### Invariant 1: `E-IRQ-ALLOC` — Static Prohibition of Dynamic Allocation

$$\forall e \in \text{AST}(f), \quad \kappa(f) = \text{IRQ} \implies \text{AllocEff}(e) = \emptyset$$

#### 1. Prohibited Constructs under `E-IRQ-ALLOC`:
1. **Dynamic Heap Construction:**
   - Object instantiation: `new BlueprintName(...)`
   - Class / Record heap allocation
   - Boxed primitives or heap tuples
2. **Dynamic Collection Growth:**
   - Dynamic list or array resizing: `list.push(item)` where capacity is unbounded
   - Map insertions requiring rehashing or node allocation
   - Dynamic string concatenation: `"vector: " + to_str(vec)`
3. **Elastic Lane Growth:**
   - Calling `lane_alloc` on elastic lanes $\lambda_0, \lambda_1$ that can trigger kernel frame allocator calls (`sn_alloc_frame` or `sn_set_memory_provider`).
   - Dynamic arena reallocation.
4. **Escaping Closures:**
   - Constructing closures that capture their environment via heap allocation.

#### 2. Permitted Memory in `irq fn`:
1. **Stack Variables (L1):**
   - Primitive scalar values (`u8`, `u16`, `u32`, `u64`, `int`, `bool`, `ptr`, `dec`).
   - Fixed-size value structs allocated directly on the CPU stack (`sub rsp, N`).
   - Fixed-size stack arrays: `let buf: [u8; 64] = [0; 64]`.
   - **Stack Depth Bound:** The compiler computes the static stack consumption $S(f) = \text{frame\_size}(f) + \max_{g \in \text{callees}} S(g)$. The compiler verifies $S(f) \le \text{MAX\_IRQ\_STACK\_BYTES}$ (default $2048$ bytes) to guarantee no IST stack overflow.
2. **Bounded Pinned-Lane Scratchpads:**
   - Pre-allocated, fixed-capacity pinned scratchpad lane $\pi_{\text{irq}}$ reserved once at kernel initialization.
   - Bounded `region { ... }` block where the compiler proves via Presburger arithmetic or Difference Bound Matrix (DBM) that total bump allocation $\sum \text{bytes} \le \text{CAPACITY}(\pi_{\text{irq}})$.
   - Watermark rolls back on exit (`sn_lane_restore`) without invoking external OS frame providers.

#### Diagnostic Rule Specification:
```
error[E-IRQ-ALLOC]: dynamic memory allocation is strictly prohibited in 'irq fn' context
  --> src/drivers/timer.sn:42:15
   |
42 |     let msg = new String("timer tick");
   |               ^^^^^^^^^^^^^^^^^^^^^^^^
   |
   = note: function 'on_timer' is declared as 'irq fn' and executes at hardware interrupt priority.
   = note: dynamic allocation requires heap/lane metadata synchronization which causes uniprocessor self-deadlock if the interrupted thread was allocating.
   = help: replace dynamic allocation with a stack-allocated buffer '[u8; N]', an atomic scalar, or write to a pre-allocated lock-free SPSC ring buffer.
```

---

### Invariant 2: `E-IRQ-BLOCK` — Static Prohibition of Blocking Synchronization

$$\forall e \in \text{AST}(f), \quad \kappa(f) = \text{IRQ} \implies \text{BlockEff}(e) = \emptyset$$

#### 1. Prohibited Constructs under `E-IRQ-BLOCK`:
1. **Sleeping Mutexes and Monitors:**
   - Calling `.lock()` or `.acquire()` on `std.sync.Mutex`, `std.sync.RwLock`, or POSIX mutex wrappers.
   - Acquiring locks that put the calling thread to sleep on contention.
2. **Channel Operations:**
   - Calling `chan.send(val)` or `chan.recv()` on blocking channels (`std.channel`).
   - Rendezvous channel handshakes.
3. **Coroutines & Asynchronous Flow:**
   - `await expr`: Interrupt handlers cannot yield execution back to an async runtime scheduler.
   - `yield()` / `thread.sleep()` / `thread.yield()`.
4. **Condition Variables & Semaphores:**
   - `condvar.wait(lock)`
   - `semaphore.acquire()`
5. **Unbounded Spin-Wait on Normal Variables:**
   - Loops of the form `while (!ready) { }` over non-atomic variables (which also violate memory model visibility).

#### 2. Permitted Constructs:
1. **Bounded Computational Loops:**
   - Counted `for i in 0..N` loops where $N$ is statically bounded.
2. **Non-blocking Polls:**
   - `spsc_ring.try_push(val) -> bool`: Immediate return; on buffer full, returns `false` without blocking.
3. **Wait-Free Atomics:**
   - Bounded compare-and-swap loops with fixed iteration ceiling or immediate single-attempt CAS.

#### Diagnostic Rule Specification:
```
error[E-IRQ-BLOCK]: blocking synchronization primitive used in 'irq fn' context
  --> src/drivers/net.sn:88:9
   |
88 |         packet_queue.lock();
   |         ^^^^^^^^^^^^^^^^^^^
   |
   = note: function 'on_ethernet_rx' executes in hardware interrupt context.
   = note: taking a sleeping mutex in an interrupt handler causes catastrophic deadlock if the interrupted thread holds this lock.
   = note: hardware interrupt contexts have no schedulable Task Control Block (tid = ⊥) and cannot be descheduled.
   = help: use an atomic flag, a wait-free SpscRing<T, N>, or defer heavy synchronization to a bottom-half softirq worker.
```

---

### Invariant 3: `E-IRQ-INTERPROC` — Interprocedural Purity Propagation

Interrupt safety is an **end-to-end interprocedural property**. An `irq fn` cannot be made safe simply by checking its lexical body; any function it calls, and any function that function calls, must obey the identical discipline.

#### 1. Call Graph Formulation & Purity Fixpoint

Let the program call graph be a directed graph:
$$\mathcal{G} = \langle \mathcal{V}, \mathcal{E}_{\text{call}} \rangle$$
where $\mathcal{V}$ is the set of all declared functions and methods in the compiled module and imported packages, and directed edge $(u, v) \in \mathcal{E}_{\text{call}}$ exists if function $u$ contains a call site targeting function $v$.

Let the set of interrupt entrypoints be:
$$\mathcal{V}_{\text{irq\_roots}} = \{ f \in \mathcal{V} \mid \text{is\_irq}(f) = \text{true} \}$$

The set of functions reachable from any interrupt entrypoint is the transitive closure:
$$\mathcal{R}(\text{IRQ}) = \{ v \in \mathcal{V} \mid \exists u \in \mathcal{V}_{\text{irq\_roots}}, \; u \to^* v \}$$

#### 2. The Knaster-Tarski Effect Propagation Algorithm

For each function $v \in \mathcal{V}$, let its intrinsic effect set be:
$$\mathcal{E}_{\text{local}}(v) \subseteq \{ \text{Alloc}, \text{Block}, \text{MMIO}, \text{Atomic}, \text{Pure} \}$$

The interprocedural effect set $\mathcal{E}^*(v)$ is computed via the least fixpoint of the monotonic transfer function:
$$\mathcal{F}(\mathcal{E}^*)(u) = \mathcal{E}_{\text{local}}(u) \cup \bigcup_{(u, v) \in \mathcal{E}_{\text{call}}} \mathcal{E}^*(v)$$

Because the effect powerset lattice is finite, Knaster-Tarski's theorem guarantees that the fixpoint iteration:
$$\mathcal{E}^*_0(u) = \mathcal{E}_{\text{local}}(u)$$
$$\mathcal{E}^*_{k+1}(u) = \mathcal{E}^*_k(u) \cup \bigcup_{(u, v)} \mathcal{E}^*_k(v)$$
terminates in at most $|\mathcal{V}|$ iterations (implemented efficiently via Tarjan's Strongly Connected Components decomposition in topological order).

#### 3. Interprocedural Invariant Check:
For every function $v \in \mathcal{R}(\text{IRQ})$:
$$\text{Alloc} \in \mathcal{E}^*(v) \implies \text{\textbf{Emit E-IRQ-INTERPROC(Alloc)} with trace } u \rightsquigarrow v$$
$$\text{Block} \in \mathcal{E}^*(v) \implies \text{\textbf{Emit E-IRQ-INTERPROC(Block)} with trace } u \rightsquigarrow v$$

#### 4. Handling Function Pointers and Closures:
- Direct function calls are resolved statically via the call graph.
- Function pointers invoked within `irq fn` must carry the explicit `irq` capability in their signature:
  ```sn
  // Legal: function pointer explicitly typed with 'irq' capability
  let handler: irq fn(u32) -> void = dispatch_table[idx];
  handler(status);
  ```
- Attempting to pass or invoke an unannotated `fn(u32) -> void` inside `irq fn` generates compile-time error:
  `E-IRQ-CAPABILITY-MISMATCH: expected 'irq fn', found unverified function pointer 'fn'`.

#### 5. Handling Recursion on Interrupt Stacks:
- Unbounded recursion on an IST stack risks immediate `#DF` (Double Fault).
- Any recursive cycle in $\mathcal{R}(\text{IRQ})$ (an SCC of size $>1$ or a self-cycle) is rejected with `E-IRQ-RECURSION` unless bounded by a static Presburger induction metric proven by the contract solver.

#### Diagnostic Rule Specification:
```
error[E-IRQ-INTERPROC]: transitive interrupt safety violation
  --> src/kernel/keyboard.sn:14:5
   |
14 |     log_event("key pressed", scancode);
   |     ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^
   |
   = note: function 'keyboard_isr' is an 'irq fn'
   = note: call trace to forbidden operation:
       keyboard_isr (src/kernel/keyboard.sn:14)
       └── log_event (src/kernel/logger.sn:52)
           └── format_string (src/stdlib/format.sn:118)
               └── String.new (src/stdlib/string.sn:45) --> performs dynamic allocation (E-IRQ-ALLOC)
   = help: 'irq fn' cannot call 'log_event' because it transitively performs dynamic heap allocation.
   = help: write raw bytes to a fixed-size ring buffer or serial MMIO port directly.
```

---

## 6. Concurrency Integration & Permitted Safe Operations

An `irq fn` is not uselessly isolated; it serves as the critical hardware-to-software bridge in the kernel. SNlang provides three mathematically verified, wait-free synchronization mechanisms designed specifically for `irq fn`.

```
┌─────────────────────────────────────────────────────────────┐
│                 HARDWARE INTERRUPT (Top-Half)               │
│                                                             │
│   irq fn keyboard_isr(ctx: IrqContext) {                    │
│       let scancode = mmio_inb(0x60);                        │
│       spsc_ring.push(scancode);         // WAIT-FREE SPSC   │
│       softirq_flags.fetch_or(1 << 1);   // ATOMIC BITFLAG   │
│       apic_eoi();                       // ACK TO HARDWARE  │
│   }                                                         │
└──────────────────────────────┬──────────────────────────────┘
                               │
                spsc_ring      │ softirq_flags
                (Wait-Free)    │ (Atomic CAS)
                               ▼
┌─────────────────────────────────────────────────────────────┐
│                 DEFERRED BOTTOM-HALF (SoftIRQ)              │
│                                                             │
│   softirq fn process_keyboard_events() {                    │
│       while let Some(scancode) = spsc_ring.pop() {          │
│           translate_and_dispatch_key(scancode);             │
│       }                                                     │
│   }                                                         │
└─────────────────────────────────────────────────────────────┘
```

### 6.1 Lock-Free and Wait-Free Atomics

Hardware interrupts may freely manipulate atomic types. Atomics execute entirely in CPU hardware pipelines without holding OS locks or yielding the processor.

#### Supported Atomic Primitives in `irq fn`:
- Types: `AtomicBool`, `AtomicU8`, `AtomicU16`, `AtomicU32`, `AtomicU64`, `AtomicInt`, `AtomicPtr`.
- Operations:
  - `.load(order: MemoryOrder) -> T`
  - `.store(val: T, order: MemoryOrder)`
  - `.fetch_add(val: T, order: MemoryOrder) -> T`
  - `.fetch_sub(val: T, order: MemoryOrder) -> T`
  - `.fetch_or(val: T, order: MemoryOrder) -> T`
  - `.fetch_and(val: T, order: MemoryOrder) -> T`
  - `.fetch_xor(val: T, order: MemoryOrder) -> T`
  - `.compare_exchange_weak(expected: T, desired: T, succ: MemoryOrder, fail: MemoryOrder) -> (T, bool)`
  - `.compare_exchange_strong(expected: T, desired: T, succ: MemoryOrder, fail: MemoryOrder) -> (T, bool)`

#### Memory Order Semantics:
- `MemoryOrder.Relaxed`: Monotonic ordering, no synchronization barriers.
- `MemoryOrder.Acquire`: Synchronizes with Release store, guarantees subsequent reads see writes prior to Release.
- `MemoryOrder.Release`: Synchronizes with Acquire load, flushes prior writes before atomic store.
- `MemoryOrder.AcqRel`: Combined Acquire and Release barrier.
- `MemoryOrder.SeqCst`: Globally consistent total order across all CPU cores.

#### Hardware Lowering:
On x86_64, `load` with Acquire and `store` with Release lower to standard `mov` instructions due to the hardware Total Store Order (TSO) memory model. Read-Modify-Write operations lower to single `lock` prefixed hardware instructions:
- `atomic.fetch_add` $\to$ `lock xadd [rdi], rax`
- `atomic.compare_exchange` $\to$ `lock cmpxchg [rdi], rsi`
- `atomic.fetch_or` $\to$ `lock or [rdi], rax`

All of these instructions execute in $\mathcal{O}(1)$ bounded clock cycles, making them completely wait-free in `irq fn`.

---

### 6.2 Wait-Free Single-Producer Single-Consumer (SPSC) Ring Buffer

The canonical idiom for transferring data from hardware interrupt context to thread context is the **Wait-Free Bounded SPSC Ring Buffer**.

#### Formal Invariant Properties:
1. **Zero Dynamic Allocation:** Buffer storage `[T; N]` is pre-allocated at compile time or during kernel boot.
2. **Single Producer Ownership:** The `head` index is updated **exclusively** by the `irq fn` producer.
3. **Single Consumer Ownership:** The `tail` index is updated **exclusively** by the bottom-half / thread consumer.
4. **Power-of-Two Ring Arithmetic:** Capacity $N = 2^k$, allowing index wrapping via bitwise AND: `idx & (N - 1)`.
5. **Wait-Freedom:** The `push` method contains no loops, no locks, and executes in exactly bounded step count $\mathcal{O}(1)$.

#### Complete SNlang Specification for SPSC Ring:

```sn
blueprint SpscRing<T, const N: u64> {
    buffer: [T; N],
    mask: u64,
    head: AtomicU64,
    tail: AtomicU64,

    static fn new() -> SpscRing<T, N> {
        // Statically assert N is power of two
        assert((N & (N - 1)) == 0, "SpscRing capacity must be a power of two");
        return SpscRing {
            buffer: [T::default(); N],
            mask: N - 1,
            head: AtomicU64.new(0),
            tail: AtomicU64.new(0),
        };
    }

    /// Wait-free push callable from 'irq fn'.
    /// Returns true on success; false on buffer overrun (full).
    fn push(self, item: T) -> bool {
        let current_head = self.head.load(MemoryOrder.Relaxed);
        let current_tail = self.tail.load(MemoryOrder.Acquire);

        // Check if ring is full
        if (current_head - current_tail) >= N {
            return false; // Overrun condition: drops item, wait-free exit
        }

        // Store item into pre-allocated slot
        self.buffer[current_head & self.mask] = item;

        // Commit item to consumer via Release semantics
        self.head.store(current_head + 1, MemoryOrder.Release);
        return true;
    }

    /// Consumer pop callable from bottom-half / thread context.
    /// Returns Some(T) on success; None on empty.
    fn pop(self) -> T? {
        let current_tail = self.tail.load(MemoryOrder.Relaxed);
        let current_head = self.head.load(MemoryOrder.Acquire);

        if current_tail == current_head {
            return none; // Ring empty
        }

        let item = self.buffer[current_tail & self.mask];
        self.tail.store(current_tail + 1, MemoryOrder.Release);
        return item;
    }
}
```

#### Correctness Proof Sketch:
- **No Torn Reads:** The producer writes `self.buffer[...]` *before* storing `current_head + 1` with `MemoryOrder.Release`. The consumer loads `head` with `MemoryOrder.Acquire` *before* reading `self.buffer[...]`. By the Acquire-Release synchronization axiom, the consumer is mathematically guaranteed to read the fully committed item.
- **No Overwriting:** The condition `(current_head - current_tail) >= N` prevents the producer from advancing `head` beyond `tail + N`. Because `tail` is monotonically non-decreasing, the producer never writes to an unconsumed slot.

---

### 6.3 Deferred Bottom-Half (SoftIRQ) Signaling

In high-performance kernel design, the top-half `irq fn` does the absolute minimum physical work required:
1. Reads hardware registers (via `mmio<T>`).
2. Clears device interrupt status.
3. Enqueues event into `SpscRing`.
4. Sets an atomic pending bitflag in a global bitmask (`softirq_pending`).
5. Acknowledges interrupt to APIC (`apic.eoi()`).

When `irq fn` finishes and the CPU returns towards thread context (`ipl = 1`), the kernel's low-level interrupt exit hook inspects `softirq_pending`:

```sn
global softirq_pending: AtomicU64 = AtomicU64.new(0);

irq fn timer_isr(ctx: IrqContext) {
    ticks.fetch_add(1, MemoryOrder.Relaxed);
    // Mark TIMER softirq bit (Bit 0)
    softirq_pending.fetch_or(1 << 0, MemoryOrder.Release);
    lapic_eoi();
}
```

The bottom half executes at `ipl = 1` with hardware interrupts re-enabled (`sti`). Here, dynamic memory allocations (within worker arenas or pools) and task synchronization can proceed safely.

---

## 7. LLVM IR Lowering & Machine Target ABI

To interface cleanly with CPU hardware interrupt vectors, an `irq fn` is lowered to LLVM IR with special attributes and calling conventions.

### 7.1 LLVM Calling Convention: `x86_intr`

On x86_64, an `irq fn` is emitted with the `x86_intr` calling convention:

```llvm
define x86_intrcc void @sn_irq_keyboard_handler(ptr byval(%struct.IrqFrame) %frame) #0 {
entry:
  ; Handler body
  call void @llvm.x86.sse.clts() ; optional FPU state handling
  ret void
}

attributes #0 = { "no-red-zone"="true" "target-cpu"="x86-64" }
```

### 7.2 Hardware Register Preservation & Red-Zone Prohibition
1. **Red-Zone Invariant (`"no-red-zone"="true"`):**
   Standard x86_64 System V ABI allows user functions to use a 128-byte "red zone" below `rsp` without adjusting the stack pointer. For an `irq fn`, the hardware immediately pushes `ss`, `rsp`, `rflags`, `cs`, `rip` onto the stack upon interrupt arrival. If the interrupted code was using the red zone, the hardware push would overwrite live data. Therefore, the entire bare-metal kernel and all `irq fn` functions are compiled with `"no-red-zone"="true"`.
2. **All-Register Preservation:**
   Under `x86_intr`, the compiler automatically emits push/pop instructions for all caller-saved and callee-saved registers touched by the function body (`rax`, `rcx`, `rdx`, `rsi`, `rdi`, `r8..r11`).
3. **Interrupt Return Opcode:**
   Instead of the standard `ret` opcode (`0xC3`), LLVM emits the hardware interrupt return opcode `iretq` (`0x48 0xCF`), which atomically pops `rip`, `cs`, `rflags`, `rsp`, and `ss`, restoring the interrupted thread's execution flags and privilege level.

---

## 8. Concrete Kernel Examples

### Example 1: 16550 UART Serial Port Interrupt Handler

```sn
use kernel.arch.x86.io
use kernel.sync.atomic
use kernel.sync.ring

const UART_COM1_PORT: u16 = 0x3F8;
const UART_LINE_STATUS: u16 = 0x3FD;

global rx_ring: SpscRing<u8, 1024> = SpscRing<u8, 1024>::new();
global rx_overrun_counter: AtomicU64 = AtomicU64.new(0);

/// Hardware interrupt handler for COM1 serial port (IRQ 4, IDT Vector 36).
/// Statically checked: No allocations, no blocking locks, interprocedural purity.
irq fn serial_com1_isr(ctx: IrqContext) {
    // Read Line Status Register
    let lsr = io_inb(UART_LINE_STATUS);

    // If data ready bit (Bit 0) is set, drain hardware FIFO
    if (lsr & 0x01) != 0 {
        let byte_in = io_inb(UART_COM1_PORT);
        
        // Wait-free append to ring buffer
        if !rx_ring.push(byte_in) {
            // Buffer full: increment overrun counter without blocking
            rx_overrun_counter.fetch_add(1, MemoryOrder.Relaxed);
        }
    }

    // Send EOI to PIC/APIC
    lapic_eoi();
}
```

### Example 2: Negative Compilation Tests (Enforcing Invariants)

```sn
// --- REJECTION TEST 1: E-IRQ-ALLOC ---
irq fn bad_alloc_isr(ctx: IrqContext) {
    let s = new String("interrupt triggered"); 
    // COMPILE ERROR [E-IRQ-ALLOC]: dynamic allocation is prohibited in 'irq fn'
}

// --- REJECTION TEST 2: E-IRQ-BLOCK ---
global kernel_mutex: Mutex = Mutex::new();

irq fn bad_lock_isr(ctx: IrqContext) {
    kernel_mutex.lock(); 
    // COMPILE ERROR [E-IRQ-BLOCK]: blocking synchronization is prohibited in 'irq fn'
}

// --- REJECTION TEST 3: E-IRQ-INTERPROC ---
fn format_and_print(val: int) {
    let msg = "val: " + to_str(val); // dynamic string allocation
    serial_print(msg);
}

irq fn bad_transit_isr(ctx: IrqContext) {
    format_and_print(42);
    // COMPILE ERROR [E-IRQ-INTERPROC]: transitive interrupt safety violation:
    // bad_transit_isr -> format_and_print -> String concatenation (E-IRQ-ALLOC)
}
```

---

## 9. Summary Table of Static Safety Rules

| Diagnostic Code | Invariant Target | Checked Properties | Allowed Safe Alternatives |
|---|---|---|---|
| `E-IRQ-ALLOC` | Dynamic Memory | Rejects `new`, heap boxing, growing lists, expanding elastic lanes | Stack values, fixed `[T; N]` buffers, bounded pinned lanes ($\pi_{\text{irq}}$) |
| `E-IRQ-BLOCK` | Blocking Sync | Rejects `mutex.lock()`, `chan.send()`, `await`, `sleep()`, `yield()` | Wait-free atomics (`AtomicU64`), SPSC ring buffers, bitflags |
| `E-IRQ-INTERPROC` | Call Graph Purity | Computes transitive closure over call graph; rejects any call path reaching alloc or block | Pure functions, leaf math, verified `@irq_safe` functions |
| `E-IRQ-CAPABILITY` | Function Pointers | Rejects invocation of unannotated `fn(...)` | Explicitly typed `irq fn(...)` function pointers |
| `E-IRQ-RECURSION` | Stack Overflow | Rejects unbounded recursion cycles in interrupt call graph | Bounded loops with static upper bounds |

---
**Verification Verdict:** Complete, mathematically formal, and ready for compiler implementation in `compiler/src/check.rs` and `compiler/src/ir.rs`.
