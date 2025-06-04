
# Kroos
A zero-cost abstraction over heap allocation and reference counting for `?Sized` types, built for performance-critical systems with manual memory control.

<p>
  <a href="https://discord.gg/MYZbyRYaxF">
    <img src="https://img.shields.io/badge/Discord-%235865F2.svg?style=flat-square&logo=discord&logoColor=white" alt="Discord">
  </a>
  <a href="https://crates.io/crates/kroos">
    <img src="https://img.shields.io/crates/v/kroos.svg">
  </a>
  <a href="https://deepwiki.com/Pavez7274/kroos">
    <img src="https://deepwiki.com/badge.svg" alt="Ask DeepWiki">
  </a>
</p>

## Why this exists
While developing [`kodrst`](https://github.com/KodekoStudios/kodrst), we encountered non-trivial overhead from using `Arc<str>` in performance-sensitive paths. Even in immutable scenarios, `Arc` introduces unnecessary synchronization and layout complexity. `kroos` provides low-level primitives (`Flake`, `Rime`) designed to bypass these costs entirely, trading away safety and general-purpose semantics in favor of raw performance.

## When to use `kroos`
- You want to store unsized data like `str` or `[u8]` on the heap with no overhead.
- You want shared ownership over unsized types, but don't want to pay for `Arc` or `Rc`.
- You need maximum control over memory layout, reference counting, or allocation.
- You can **guarantee** safe memory usage manually (e.g., no `Drop`, aliasing, or race conditions).

## When *not* to use
- Your types require `Drop`, or have complex ownership semantics.
- You want ergonomic or type-safe abstractions.
- You're not prepared to work at the raw pointer level.


## `Flake<T: ?Sized>`
A `Box`-like wrapper for unsized types without ownership semantics. It provides heap allocation for types like `str`, `[u8]`, or any `?Sized` data using raw pointers.

### `Flake::new`
Allocates a copy of a referenced `?Sized` object to the heap.

```rust
let flake = Flake::new("hello");
assert_eq!(&*flake, "hello");
```

### `Flake::steal`
Moves a `Sized` value directly into the heap, without duplication.

```rust
let flake = Flake::steal(String::from("hi"));
assert_eq!(&*flake, "hi");
```

### Memory layout
A `Flake<T>` contains a single fat pointer to the heap-allocated value. It does **not** store length or capacity explicitly — metadata is embedded in the fat pointer.

### Safe mutability
While `Flake` is primarily intended for immutable data, controlled mutation is allowed:

```rust
let mut flake = Flake::new(&[1, 2, 3][..]);
unsafe { (*flake.as_mut_ptr())[0] = 42; }
assert_eq!(&*flake, &[42, 2, 3]);
```

> [!CAUTION]
> Use only if you guarantee exclusive access.


## `Rime<C, T: ?Sized>`

A compact, inline, reference-counted pointer to unsized types, using a custom `Counter`.

### Features
* Fully inline: `Rime` stores `[ Counter | Data ]` in one allocation.
* Works with dynamically sized types (`str`, `[u8]`, etc.)
* Counter-agnostic: atomic or non-atomic counters via `Rime<AtomicU8, str>` or `Rime<Cell<u8>, str>`.
* No vtable, no indirection.

### `Rime::new`
Copies a reference to heap and initializes a new refcount.

```rust
let rime = Rime::<AtomicU8, str>::new("hello");
let clone = rime.clone();
assert_eq!(&*clone, "hello");
```

### `Rime::steal`
Takes ownership of a `Sized` value and moves it into a single block.

```rust
let rime = Rime::<u8, String>::steal("hi".to_string());
assert_eq!(&*rime, "hi".to_string());
```

### Safe mutability
If you use a `Cell<u8>` counter (or similar interior-mutable strategy), you can achieve safe mutability under the following constraints:

1. `Rime` must not be cloned (i.e. you must be the only owner).
2. Use `as_mut_ptr` to mutate contents in-place.
3. You are responsible for enforcing Rust’s aliasing rules manually.


## Handling growth and capacity
You can store metadata in a header format like:
```text
[ Counter | Metadata | Data ]
            ↑          ↑
         len/cap       T
```

For example, a fixed-capacity string-like object might store `len` as a `usize`, followed by a zero-terminated buffer. On mutation:

1. Update `len` manually.
2. Overwrite data in-place within bounds.

This enables a fixed-capacity "string" stored inline, avoiding reallocation.


## Avoiding stack-to-heap moves
`Flake::from_raw` and `Rime::from_raw` allow you to construct heap-backed references without the intermediate cost of stack allocation followed by a move. Instead of creating a `Box<T>` or `Vec<T>` and extracting its pointer, you can allocate memory and write directly into it.

For example, to create a `*mut str` without touching the stack:

```rust
pub unsafe fn alloc_str_and_write(bytes: &[u8]) -> *mut str {
    let len = bytes.len();
    let ptr = alloc(Layout::array::<u8>(len).unwrap());
    
    // You can handle layouts errors checking ptr.in_null()

    ptr.copy_from_nonoverlapping(bytes.as_ptr(), len);
    ptr::slice_from_raw_parts_mut(ptr, len) as *mut str
}
```

You can then wrap the result into a `Flake`:

```rust
let raw = unsafe { alloc_str_and_write(b"hola") };
let flake = unsafe { Flake::from_raw(raw) };
```

This pattern avoids temporary allocations or extra indirection like:

```rust
let boxed = Box::new(value);     // Allocates and moves
let flake = Flake::new(&*boxed); // Copies again
```

Instead, you allocate once and write directly where the value will live.

> [!NOTE]
> Box is optimized for sometimes do a `placement-in protocol`-like.


## Comparison Table
| Feature              | `Box` / `Arc` | `Flake` / `Rime`   |
| -------------------- | ------------- | ------------------ |
| Works with `?Sized`  | ✅ (`Box`)     | ✅                  |
| Custom counter       | ❌             | ✅                  |
| Atomic optional      | ✅ (`Arc`)     | ✅ (via `AtomicU*`) |
| Inline allocation    | ❌             | ✅                  |
| Copy or move control | ❌             | ✅                  |
| Drop safety          | ✅             | ❌ (must be manual) |


## Safety Warnings
- `Flake` and `Rime` bypass `Drop`, `Clone`, and Rust's ownership model.
- Do **not** use with types that manage heap resources or contain non-`Copy` fields.
- You are responsible for:
  - Ensuring uniqueness or correct refcounting.
  - Avoiding data races (use `Atomic*` if needed).
  - Deallocating correctly (done automatically, but only if created via safe constructors).


## Contributing & Attribution
This project is licensed under the `GNU AGPL v3`.

All contributions must retain clear attribution to the original author(s).  
If you modify or extend this project, please include appropriate credit in your repository and documentation.

In particular:
- Keep the original copyright notices.
- If publishing a fork, mention the original project name and link in your README.
- If you use this project as a dependency, consider acknowledging it in your LICENSE or README.

I believe in free software and shared credit.

## Contributors
[![Kaffssist](https://github.com/Pavez7274.png?size=80)](https://github.com/Pavez7274)


*This crate exists to explore *precision memory control* and support edge-case optimizations in projects like [`kodrst`](https://github.com/KodekoStudios/kodrst). It is **not** intended as a general-purpose utility. If you're using `Flake` or `Rime` outside internal systems, you're either very brave — or very desperate...*
