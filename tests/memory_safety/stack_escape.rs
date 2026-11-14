// Rust Memory Safety Test 1: Stack Reference Escape
//
// In Rust, the borrow checker verifies lifetimes statically.
// Any attempt to return a reference to a local variable or store a reference to a local
// variable into a struct/data structure that outlives that local variable's scope is
// caught and rejected at compile time (error E0515 / E0597).

struct Holder<'a> {
    ptr: Option<&'a i32>,
}

// 1. Escaping local reference via return value
fn escape_return() -> &'static i32 {
    let local = 42;
    &local // ERROR[E0515]: cannot return reference to local variable `local`
}

// 2. Escaping local reference into a struct that outlives local variable's block scope
fn escape_struct() {
    let mut holder = Holder { ptr: None };
    {
        let local = 99;
        holder.ptr = Some(&local); // ERROR[E0597]: `local` does not live long enough
    }
    println!("Value: {}", holder.ptr.unwrap());
}

fn main() {
    let p = escape_return();
    println!("Returned ptr: {}", p);
    escape_struct();
}
