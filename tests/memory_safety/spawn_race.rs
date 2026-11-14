// Rust Memory Safety Test 4: Concurrency Data Races in Thread Spawn
//
// In Rust, fearless concurrency is enforced at compile time via the Send and Sync traits.
// Any attempt to pass unsynchronized mutable references (&mut T) or non-thread-safe
// shared mutable containers (such as Rc<RefCell<T>>) across threads is rejected by
// the compiler (error E0597 / E0277).

use std::cell::RefCell;
use std::rc::Rc;
use std::thread;

struct Counter {
    value: i32,
}

// 1. Passing unsynchronized mutable reference across thread boundary
fn race_mutable_reference() {
    let mut val = 10;
    let ptr = &mut val;
    thread::spawn(move || {
        *ptr = 99; // ERROR[E0597]: `val` does not live long enough ('static required)
    });
}

// 2. Passing unsynchronized mutable object across thread boundary
fn race_unsynchronized_type() {
    let c = Rc::new(RefCell::new(Counter { value: 0 }));
    let c_clone = Rc::clone(&c);
    thread::spawn(move || {
        c_clone.borrow_mut().value = 42; // ERROR[E0277]: `Rc<RefCell<Counter>>` cannot be sent between threads safely
    });
}

fn main() {
    race_mutable_reference();
    race_unsynchronized_type();
}
