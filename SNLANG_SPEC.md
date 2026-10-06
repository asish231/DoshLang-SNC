# SNlang — The SafarNow Language

**Version:** 0.2  
**Created by:** SafarNow  
**Paradigm:** Compiled, Strongly-Typed, Object-Oriented, Concurrent  
**Philosophy:** Easy syntax, native LLVM, opt-in null — not “as fast as C” by slogan, and not a Go/Rust clone.

SNlang compiles to native code through LLVM (`snc` emits IR, `clang` builds the binary). No semicolons. Curly braces `{}` for blocks. English-readable logic (`and` / `or` / `not`). The compiler lives in `compiler/` (Rust).

---

## 1. Comments

```text
// This is a single-line comment

/*
   This is a multi-line comment.
   Use it for documentation.
*/
```

---

## 2. Data Types

SNlang enforces strict typing. Every variable must declare its type.

### Primitive Types

| Type       | Description                          | Example                  |
|------------|--------------------------------------|--------------------------|
| `int`      | Whole numbers (64-bit signed)        | `int age = 25`           |
| `dec(X)`   | Decimal with X digits precision      | `dec(2) price = 99.50`   |
| `str`      | Text / string of characters          | `str name = "Alice"`     |
| `bool`     | True or false                        | `bool active = true`     |
| `byte`     | Single byte (0–255)                  | `byte flag = 0xFF`       |

### Collection Types

| Type          | Description                             | Example                              |
|---------------|-----------------------------------------|--------------------------------------|
| `list<T>`     | Ordered collection of type T            | `list<int> scores = [90, 85, 70]`    |
| `map<K, V>`   | Key-value pairs                         | `map<str, int> ages = {"Bob": 30}`   |

### Special Types

| Type       | Description                          | Example                  |
|------------|--------------------------------------|--------------------------|
| `none`     | Represents no value / null           | `return none`            |

> **`any`** — MVP in v0.2: `any_box(value)`, `.as_int()`, `.as_str()`, `.type_name()`. Boxed int/str/bool tags only.

> **`record`** — copy-by-value stack struct: `record Point { int x; int y }`.

---

## 3. Constants

Constants are immutable. Once set, they can never change.

```text
const int MAX_USERS = 1000
const str APP_NAME = "SafarNow"
const dec(2) TAX_RATE = 18.00
```

---

## 4. Variables & Assignment

```text
int count = 0
str greeting = "Hello"
bool isReady = false

// Reassignment (same type only — strict!)
count = 10        // OK
count = "hello"   // ERROR: type mismatch
```

---

## 5. Operators

### Arithmetic

| Operator | Description    | Example         |
|----------|----------------|-----------------|
| `+`      | Addition       | `x + y`         |
| `-`      | Subtraction    | `x - y`         |
| `*`      | Multiplication | `x * y`         |
| `/`      | Division       | `x / y`         |
| `%`      | Modulo         | `x % 2`         |
| `**`     | Power          | `x ** 3`        |

### Comparison

| Operator | Description          | Example       |
|----------|----------------------|---------------|
| `==`     | Equal to             | `x == 5`      |
| `!=`     | Not equal to         | `x != 5`      |
| `>`      | Greater than         | `x > 5`       |
| `<`      | Less than            | `x < 5`       |
| `>=`     | Greater or equal     | `x >= 5`      |
| `<=`     | Less or equal        | `x <= 5`      |

### Logical (English words, not symbols!)

| Operator | Description          | Example                    |
|----------|----------------------|----------------------------|
| `and`    | Both must be true    | `x > 0 and x < 100`       |
| `or`     | Either can be true   | `x == 0 or x == 1`        |
| `not`    | Negation             | `not isReady`              |

### Assignment Shortcuts

| Operator | Description           | Example      |
|----------|-----------------------|--------------|
| `+=`     | Add and assign        | `x += 5`     |
| `-=`     | Subtract and assign   | `x -= 5`     |
| `*=`     | Multiply and assign   | `x *= 2`     |
| `/=`     | Divide and assign     | `x /= 2`     |

---

## 6. Blocks — Curly Braces `{}`

SNlang uses curly braces `{}` for all block scoping. This keeps the language familiar to anyone coming from or going to Java, C++, or Go. No semicolons are needed — each statement is one line.

```text
fn main() {
    print("Hello")
}
```

---

## 7. Control Flow

### If / Else If / Else

Parentheses are **required** around conditions. Curly braces define blocks.

```text
if (age >= 18 and age < 60) {
    print("Adult")
} else if (age >= 60) {
    print("Senior")
} else {
    print("Child")
}
```

### Match (Switch-Case Alternative)

Clean pattern matching. No `break` needed — only the matched arm executes.

```text
match (status) {
    "active" {
        print("User is active")
    }
    "banned" {
        print("User is banned")
    }
    default {
        print("Unknown status")
    }
}
```

---

## 8. Loops

### For Loop (Counted)

Uses the standard `for` keyword — easy to transfer skills to Java/C++.

```text
for (int i = 0, i < 10, i += 1) {
    print(i)
}
```

### For-In Loop (Iterate Collections)

Same `for` keyword with `in` to iterate over collections.

```text
list<str> fruits = ["Apple", "Mango", "Banana"]

for (fruit in fruits) {
    print(fruit)
}
```

### While Loop

```text
int attempts = 0

while (attempts < 3) {
    print("Trying...")
    attempts += 1
}
```

### Loop Control

| Keyword   | Description                         |
|-----------|-------------------------------------|
| `stop`    | Exits the loop immediately (break)  |
| `skip`    | Skips to next iteration (continue)  |

```text
for (num in [1, 2, 3, 4, 5]) {
    if (num == 3) {
        skip
    }
    if (num == 5) {
        stop
    }
    print(num)
}
// Output: 1, 2, 4
```

---

## 9. Functions

Use `fn` to declare functions. Return types are declared after `->`.

```text
fn greet(str name) -> str {
    return "Hello, " + name
}

fn add(int a, int b) -> int {
    return a + b
}

// Functions with no return value
fn logMessage(str msg) {
    print("[LOG] " + msg)
}
```

### Default Parameters

```text
fn connect(str host, int port = 8080) {
    print("Connecting to " + host)
}

connect("localhost")         // Uses port 8080
connect("localhost", 3000)   // Uses port 3000
```

### Multiple Return Values (like Go!)

```text
fn divide(int a, int b) -> (int, bool) {
    if (b == 0) {
        return (0, false)
    }
    return (a / b, true)
}

int result, bool ok = divide(10, 3)

if (not ok) {
    print("Division failed!")
}
```

---

## 10. Pointers & Addresses (Simplified)

SNlang gives you direct memory access like C, but makes it **much simpler and safer**. No confusing `*`, `&`, `->` juggling.

### Declaring a Pointer

Use `ref<T>` (reference) to declare a pointer to a type. It reads like English: "a reference to an int."

```text
int x = 42
ref<int> p = address(x)    // p holds the memory address of x
```

### Reading the Value at an Address

Use `value(p)` to dereference — get the data the pointer points to.

```text
int y = value(p)    // y = 42
```

### Writing to an Address

```text
set(p, 100)         // The memory at p now holds 100
print(value(p))     // 100
print(x)            // 100 — x changed because p points to x
```

### Pointer Arithmetic

Move through memory manually. Essential for building your own data structures.

```text
ref<byte> start = address(buffer)
ref<byte> next = start + 1      // Move 1 byte forward
ref<byte> tenth = start + 10    // Move 10 bytes forward

byte val = value(next)          // Read the byte at that address
```

### Allocating Raw Memory

Use `alloc(bytes)` to request raw memory from the system. Use `free(p)` to release it.

```text
ref<byte> block = alloc(1024)   // Allocate 1024 bytes

// Write to it
set(block, 0xFF)
set(block + 1, 0x00)

// Read from it
byte first = value(block)       // 0xFF

// Release when done
free(block)
```

### Why This Is Easier Than C

| Task               | C Syntax                | SNlang Syntax            |
|--------------------|-------------------------|--------------------------|
| Declare pointer    | `int *p;`               | `ref<int> p`             |
| Get address        | `p = &x;`               | `p = address(x)`         |
| Read value         | `y = *p;`               | `y = value(p)`           |
| Write value        | `*p = 100;`             | `set(p, 100)`            |
| Pointer arithmetic | `*(p + 3) = 10;`        | `set(p + 3, 10)`         |
| Allocate memory    | `p = malloc(1024);`     | `p = alloc(1024)`        |
| Free memory        | `free(p);`              | `free(p)`                |

With this system, a programmer can build **any** data structure (linked lists, stacks, queues, trees, hash maps) from scratch — without fighting cryptic syntax.

---

## 11. Building Your Own Data Structures (Example)

SNlang does **not** provide built-in complex data structures. The programmer builds them using blueprints, pointers, and raw memory. Here is an example of a simple linked list node:

```text
blueprint Node {
    int data
    ref<Node>? next = none

    fn create(int data) {
        self.data = data
    }
}

fn main() {
    object a = Node(10)
    object b = Node(20)
    object c = Node(30)

    a.next = address(b)
    b.next = address(c)

    // Walk the list
    ref<Node>? current = address(a)
    while (current != none) {
        print(value(current).data)
        current = value(current).next
    }
    // Output: 10, 20, 30
}
```

---

## 12. Error Handling

Explicit error handling inspired by Go. No hidden exceptions. You always deal with errors.

```text
fn readConfig(str path) -> (str, error) {
    str data = file_read(path)
    if (data == "") {
        return ("", error("File missing or empty: " + path))
    }
    return (data, none)
}

str data, error err = readConfig("config.txt")

if (err != none) {
    print("Error: " + err.message())
} else {
    print(data)
}
```

> Prefer **`use std.file`** — `read`/`write` return `(str, error)` / `error`; `exists(path) -> bool`.

### Panic (Unrecoverable Errors)

```text
fn mustConnect(str db) -> connection {
    connection conn, error err = openDB(db)
    if (err != none) {
        panic("Cannot start without database!")
    }
    return conn
}
```

---

## 13. Blueprints (OOP — Classes)

`blueprint` replaces the word `class`. Curly braces `{}` for the body.

```text
blueprint User {
    str name
    str email
    int age

    fn create(str name, str email, int age) {
        self.name = name
        self.email = email
        self.age = age
    }

    fn greet() -> str {
        return "Hi, I'm " + self.name
    }

    fn isAdult() -> bool {
        return (self.age >= 18)
    }
}
```

### Object Creation

Use `new Type name(field: value, …)`. Field values are assigned, then optional `fn create()` runs if defined.

```text
new User alice(name: "Alice", email: "alice@mail.com", age: 25)
print(alice.greet())

if (alice.isAdult()) {
    print("Access granted")
}
```

### OOP — what is implemented vs not

SNlang has a **teaching OOP subset**, not full Java/C++ OOP:

| Concept | Status |
|---------|--------|
| Blueprints (classes), `self`, fields, methods | ✅ |
| Single inheritance (`from`) + `super.method()` | ✅ MVP |
| Contracts + `follows` (compile-time interface check) | ✅ |
| Field access: `open` / `closed` / `guarded` | ✅ enforced at compile time |
| Optional `fn create()` after field init | ✅ |
| Subtype polymorphism (`Child` where `Parent` expected) | ✅ MVP (`type_id` dispatch) |
| Contract-typed variables (`Drawable x`) | ❌ |
| Virtual / dynamic dispatch | ✅ MVP (not vtable-based) |
| Generic blueprints (`blueprint Box<T>`) | ✅ MVP (monomorphize at `new`) |
| Static (class) members, abstract classes | ❌ |
| Method-level `private` / properties | ❌ |
| Heterogeneous collections (`list<Animal>` mixed subtypes) | ❌ |

---

## 14. Inheritance

Use `from` to inherit from another blueprint.

```text
blueprint Animal {
    str species
    int legs

    fn describe() -> str {
        return self.species + " with " + cast(self.legs, str) + " legs"
    }
}

blueprint Dog from Animal {
    str breed

    fn bark() {
        print("Woof!")
    }
}
```

```text
new Dog rex(species: "Canine", legs: 4, breed: "Labrador")
print(rex.describe())
rex.bark()
```

---

## 15. Contracts (Interfaces)

`contract` defines a set of functions that a blueprint **must** implement.

```text
contract Drawable {
    fn draw()
    fn resize(int width, int height)
}

blueprint Circle follows Drawable {
    int radius

    fn draw() {
        print("Drawing circle with radius " + cast(self.radius, str))
    }

    fn resize(int width, int height) {
        self.radius = width / 2
    }
}
```

---

## 16. Access Control

| Keyword   | Description                                  |
|-----------|----------------------------------------------|
| `open`    | Accessible from anywhere (default)           |
| `closed`  | Only accessible within the blueprint itself   |
| `guarded` | Accessible by the blueprint and its children  |

```text
blueprint BankAccount {
    open str ownerName
    closed dec(2) balance = 0.00

    fn deposit(dec(2) amount) {
        self.balance += amount
    }

    fn getBalance() -> dec(2) {
        return self.balance
    }
}
```

---

## 17. Modules & Imports

Every `.sn` file is a module. Import using `use`.

```text
// file: math_utils.sn
fn square(int x) -> int {
    return x ** 2
}
```

```text
// file: main.sn
use math_utils

int result = math_utils.square(5)
print(result)   // 25
```

### Multiple Module Imports

You can import multiple modules in a single file:

```text
use std.math
use utils.string
use io.file
use net.http

fn main() {
    print("All modules loaded!")
}
```

### Dotted Module Paths

Modules can be organized in hierarchical paths:

```text
use std.collections.list
use std.collections.map
use company.auth.jwt
use company.database.postgres
```

**Current Implementation Status:**
- ✅ `use module.path` syntax parsing
- ✅ Single and multiple module imports
- ✅ Dotted module paths (`use std.math`)
- ✅ File loading and symbol resolution (`driver.rs` walks imports)
- ✅ Search paths: importer directory, `packages/`, `stdlib/`, path deps in `sn.toml`
- ✅ `snc pkg init|add|list|build|publish` for local packages + lockfile (`sn.lock.toml`)
- ✅ Remote package registry MVP (`--registry` fetches `index.json` + tarball; see `docs/PACKAGES.md`)

### Standard Library Imports

```text
use std.io          // print, file_read, file_write (legacy globals)
use std.file        // read, write, append, exists, delete, copy, move, mkdir, rmdir, list_dir
use std.path        // join, base, dir, ext
use std.os          // getenv, exit, args
use std.math        // abs, min, max, pow, …
use std.net         // HTTP client (plain HTTP sockets; HTTPS via curl)
use std.http        // get, post, serve_once (TLS via verified curl by default)
use std.time        // now_ms, sleep_ms, format
use std.json        // parse, encode
use std.string      // isEmpty, repeat
use std.test        // assert_true, assert_eq_int, assert_eq_str, …
```

---

## 18. Concurrency / Threads

### Spawning Threads

```text
fn downloadFile(str url) {
    print("Downloading: " + url)
}

// Spawn new thread, continues immediately
spawn downloadFile("https://example.com/data.zip")
print("This runs immediately, no waiting!")
```

### Channels (Safe Communication)

```text
chan<str> messages

spawn {
    messages.send("Hello from thread!")
}

spawn {
    str msg = messages.receive()
    print(msg)
}
```

### Locks (Synchronization)

```text
lock counterLock
int counter = 0

fn increment() {
    lock(counterLock) {
        counter += 1
    }
}

spawn increment()
spawn increment()
print(counter)  // Output: 2
```

### Async/Await

```text
async fn slowTask() -> int {
    return 42
}

int result = await slowTask()
print(result)
```

---

## 19. Type Casting

Explicit only. No silent coercion — ever.

```text
int x = 42
dec(2) y = cast(x, dec(2))   // y = 42.00
str z = cast(x, str)         // z = "42"

str numStr = "100"
int num = cast(numStr, int)  // num = 100
```

---

## 20. Null Safety

A variable can only be `none` if you explicitly allow it with `?`.

```text
str name = "Alice"       // Can NEVER be none
str? nickname = none     // Allowed to be none

if (nickname != none) {
    print(nickname)
}

// Or use the 'otherwise' operator
print(nickname otherwise "No nickname")
```

---

## 21. String Operations

```text
str greeting = "Hello, World!"

int len = greeting.length           // 13
str sub = greeting.slice(0, 5)      // "Hello"
bool has = greeting.contains("World")  // true
str fixed = greeting.replace("World", "SNlang")
list<str> parts = greeting.split(", ")
str upper = greeting.upper()        // "HELLO, WORLD!"
str lower = greeting.lower()        // "hello, world!"

// String interpolation
str name = "Alice"
int age = 25
str msg = "My name is {name} and I am {age} years old"
```

---

## 22. Input / Output

```text
use std.io

print("Hello, SNlang!")

str userInput = input("Enter your name: ")
print("Welcome, " + userInput)
```

### File I/O

**Legacy globals** (no import): `file_read(path) -> str`, `file_write(path, data) -> bool`.

**`std.file`** (recommended):

```text
use std.file

fn main() {
    str data, error err = read("config.txt")
    if (err != none) {
        print(err.message())
        return
    }
    error e = write("out.txt", data)
    print(exists("out.txt"))
    list<str> names, error le = list_dir(".")
}
```

| Function | Returns |
|---|---|
| `read(path)` | `(str, error)` |
| `write(path, data)` | `error` |
| `append(path, data)` | `error` |
| `exists(path)` | `bool` |
| `delete(path)` | `error` |
| `copy(src, dst)` | `error` |
| `move(src, dst)` | `error` |
| `mkdir(path)` | `error` |
| `rmdir(path)` | `error` |
| `list_dir(path)` | `(list<str>, error)` |

### Path and OS

```text
use std.path
use std.os

str p = join("dir", "file.txt")
print(base(p))
str? v = getenv("HOME")
list<str> argv = args()
```

### Membership (`in`)

```text
list<int> xs = [1, 2, 3]
if (2 in xs) { print("yes") }
if ("lo" in "hello") { print("substring") }
```

Distinct from `for (x in xs)` — `in` is also a binary operator.

### Formatter

```sh
snc fmt examples/*.sn
```

Trims trailing whitespace, normalizes brace-based indentation (4 spaces), ensures a final newline.

### File I/O (legacy detail)

Whole-file text only — global builtins, no import:

```text
fn main() {
    bool ok = file_write("output.txt", "Hello from SNlang!")
    str contents = file_read("output.txt")
    print(contents)
}
```

- `file_read(path) -> str` — returns `""` if the file is missing or empty (legacy; prefer `std.file.read`)
- `file_write(path, data) -> bool` — overwrites the whole file

**MVP (see README):** HTTPS via system `curl` (cert verify by default; `SN_HTTP_INSECURE=1` for `-k`), remote registry fetch + `sn.lock.toml` + `pkg publish`, async/await (blocking), goroutine pool, OOP polymorphism, generic blueprints.

**Still not production-grade:** in-process OpenSSL/LibreSSL, hosted central registry, true async I/O, work-stealing scheduler, full borrow lifetimes, complete LSP/debugger, self-hosted compiler.

---

## 23. Program Entry Point

Every SNlang program starts from a `main` function.

```text
fn main() {
    print("Welcome to SNlang!")
    int x = 10
    int y = 20
    print("Sum: " + cast(x + y, str))
}
```

---

## 24. Complete Example Program

```text
use std.io

const str APP_NAME = "SNlang Demo"
const dec(2) TAX_RATE = 18.00

contract Printable {
    fn display() -> str
}

blueprint Product follows Printable {
    str name
    dec(2) price
    int quantity

    fn create(str name, dec(2) price, int quantity) {
        self.name = name
        self.price = price
        self.quantity = quantity
    }

    fn totalPrice() -> dec(2) {
        dec(2) subtotal = self.price * cast(self.quantity, dec(2))
        dec(2) tax = subtotal * TAX_RATE / 100.00
        return subtotal + tax
    }

    fn display() -> str {
        return self.name + " — $" + cast(self.totalPrice(), str)
    }
}

blueprint DigitalProduct from Product {
    str downloadURL

    fn display() -> str {
        return self.name + " [Digital] — $" + cast(self.totalPrice(), str)
    }
}

fn main() {
    print("=== " + APP_NAME + " ===")

    new Product laptop(name: "Laptop", price: 999.99, quantity: 1)
    new DigitalProduct ebook(
        name: "SNlang Guide",
        price: 29.99,
        quantity: 1,
        downloadURL: "https://safarnow.com/snlang-guide"
    )

    // MVP subtype polymorphism: Child may be used where Parent is expected (type_id dispatch).
    // Homogeneous list<Product> holding mixed subclasses is still limited — call methods directly:
    print(laptop.display())
    print(ebook.display())

    dec(2) total = laptop.totalPrice() + ebook.totalPrice()

    print("Total: $" + cast(total, str))

    str? coupon = none
    str discount = coupon otherwise "No coupon applied"
    print(discount)

    if (total > 500.00) {
        print("Free shipping!")
    } else {
        print("Shipping: $9.99")
    }
}
```

---

## Quick Reference Card

| Feature          | SNlang Syntax                    | Java / C++ / Go           |
|------------------|----------------------------------|----------------------------|
| Variable         | `int x = 5`                      | `int x = 5;`              |
| Constant         | `const int X = 5`                | `final int X = 5;`        |
| Function         | `fn add(int a, int b) -> int {`  | `int add(int a, int b) {` |
| Class            | `blueprint User {`               | `class User {`             |
| Interface        | `contract Drawable {`            | `interface Drawable {`     |
| Inheritance      | `blueprint Dog from Animal {`    | `class Dog extends Animal` |
| Implements       | `blueprint X follows Y {`        | `class X implements Y {`   |
| If               | `if (x > 5) {`                   | `if (x > 5) {`            |
| For Loop         | `for (int i=0, i<10, i+=1) {`   | `for(int i=0;i<10;i++) {` |
| For Each         | `for (x in list) {`             | `for (auto x : list) {`   |
| Switch           | `match (x) {`                   | `switch(x) {`              |
| Pointer          | `ref<int> p`                     | `int *p`                   |
| Get address      | `address(x)`                     | `&x`                       |
| Dereference      | `value(p)`                       | `*p`                       |
| Write to ptr     | `set(p, 100)`                    | `*p = 100`                 |
| Allocate         | `alloc(1024)`                    | `malloc(1024)`             |
| Free             | `free(p)`                        | `free(p)`                  |
| Thread           | `thread fn work() {`            | `new Thread(...)`          |
| Null check       | `str? x = none`                  | `String x = null;`        |
| Null fallback    | `x otherwise "default"`          | `x ?? "default"`           |
| Cast             | `cast(x, str)`                   | `(String) x`               |
| Import           | `use std.io`                     | `import java.io.*`         |
| Print            | `print("hi")`                    | `System.out.println("hi")` |
| Logical AND      | `and`                            | `&&`                       |
| Logical OR       | `or`                             | `\|\|`                     |
| Logical NOT      | `not`                            | `!`                        |
| Break            | `stop`                           | `break`                    |
| Continue         | `skip`                           | `continue`                 |
| Public           | `open`                           | `public`                   |
| Private          | `closed`                         | `private`                  |
| Protected        | `guarded`                        | `protected`                |

---

## File Extension

All SNlang source files use the **`.sn`** extension.

```
main.sn
utils.sn
models.sn
```

---

*SNlang — Built by SafarNow. Code should be simple, strict, and fast.*

---

## 25. Defer, try, null finish, closures (implemented)

```text
defer { print("on function exit") }

fn divide(int a, int b) -> (int, error) { ... }
int q = try divide(10, 2)   // early-return error; not Java exceptions

str? name = none
str shown = name otherwise "n/a"
if (name != none) { print(name) }   // name is str in this branch
str must = name!!                   // panic if none
print(name?.length())               // optional chaining

fn apply(fn(int) -> int f, int x) -> int { return f(x) }
```

`defer` is **function-scoped** (like Go): it runs on return / function exit, not at the end of an `if` block.

`match` on `T?` / `error` must be exhaustive (`none` + another arm, or `default`).

Nested `fn` and `fn(int x) -> int { ... }` lambdas are values (limited captures, copied like `spawn`). There is no goroutine scheduler and no borrow checker.

## 26. Packages

`sn.toml` + `snc pkg init|add|build|list`. `use foo.bar` resolves through `packages/` and `stdlib/`. See README.

