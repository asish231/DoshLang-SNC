// Rust Memory Safety Comparison 5: Reference Cycle Memory Leak
//
// Demonstrates that standard Rust reference counting (Rc / Arc) without weak references
// cannot reclaim cyclic reference structures. When cycles are formed, the reference count
// never drops to zero, and the destructor (Drop) is never invoked, permanently leaking memory.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::atomic::{AtomicUsize, Ordering};

static DROP_COUNT: AtomicUsize = AtomicUsize::new(0);

struct Node {
    id: usize,
    next: RefCell<Option<Rc<Node>>>,
}

impl Drop for Node {
    fn drop(&mut self) {
        DROP_COUNT.fetch_add(1, Ordering::SeqCst);
        println!("Dropping Node {}", self.id);
    }
}

fn create_cyclic_graph() {
    let a = Rc::new(Node {
        id: 1,
        next: RefCell::new(None),
    });
    let b = Rc::new(Node {
        id: 2,
        next: RefCell::new(None),
    });

    // Create mutual reference cycle: a -> b -> a
    *a.next.borrow_mut() = Some(Rc::clone(&b));
    *b.next.borrow_mut() = Some(Rc::clone(&a));

    println!("Inside scope: a.strong_count = {}, b.strong_count = {}",
        Rc::strong_count(&a), Rc::strong_count(&b));
    // Leaving scope: a and b stack handles are dropped, but strong_counts remain 1.
}

fn main() {
    println!("=== Rust Reference Cycle Leak Demonstration ===");
    create_cyclic_graph();
    println!("Exited cycle creation scope.");

    let dropped = DROP_COUNT.load(Ordering::SeqCst);
    println!("Nodes dropped: {} (expected: 0 because cycle leaks)", dropped);

    if dropped == 0 {
        println!("RESULT: LEAK CONFIRMED. Reference cycle permanently leaked in Rust Rc.");
    } else {
        println!("RESULT: Nodes were dropped.");
    }
}
