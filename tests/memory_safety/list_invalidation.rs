// Rust Memory Safety Test 2: Collection Invalidation During Active Borrow
//
// In Rust, holding a shared (immutable) reference to an element of a collection
// locks the entire collection against concurrent mutation. Calling .push()
// requires a mutable borrow of `numbers`, which is rejected at compile time (error E0502)
// because reallocating the underlying vector would invalidate existing references.

fn main() {
    let mut numbers = vec![10, 20, 30];

    // Immutable borrow of the first element
    let first = &numbers[0];

    // Attempting to mutate the collection while an element reference is active:
    // This causes vector reallocation in memory.
    numbers.push(40); // ERROR[E0502]: cannot borrow `numbers` as mutable because it is also borrowed as immutable

    println!("First element: {}", *first);
}
