// Rust Memory Safety Test 3: Heap Use-After-Free and Double-Free
//
// In Rust, memory deallocation is governed by affine types (move semantics).
// When an allocated heap value (Box<T>) is deallocated via drop(), ownership
// is moved and consumed. Any subsequent read, write, or second drop attempt
// is rejected at compile time (error E0382).

fn test_uaf_read() {
    let ptr = Box::new(42);
    drop(ptr);
    // 1. Reading from deallocated pointer:
    println!("{}", *ptr); // ERROR[E0382]: borrow of moved value: `ptr`
}

fn test_uaf_write() {
    let mut ptr = Box::new(42);
    drop(ptr);
    // 2. Writing to deallocated pointer:
    *ptr = 99; // ERROR[E0382]: use of moved value: `ptr`
}

fn test_double_free() {
    let ptr = Box::new(42);
    drop(ptr);
    // 3. Deallocating already freed pointer:
    drop(ptr); // ERROR[E0382]: use of moved value: `ptr`
}

fn main() {
    test_uaf_read();
    test_uaf_write();
    test_double_free();
}
