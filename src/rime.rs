use std::{marker::PhantomData, mem::size_of_val, hash::Hash, sync::atomic::*, alloc::*, ptr::*};

/// A trait for defining a reference-counting strategy.
///
/// `Counter` is implemented by types that support manual increment and decrement
/// operations. It enables [`Rime`] to be agnostic about how reference counts are
/// stored or updated (e.g. atomically, interior-mutable, or primitive integers).
///
/// # Safety
/// Implementors must ensure:
/// - `increment()` increases the count.
/// - `decrement()` decreases it and returns `true` if the count reached zero.
/// - Overflow and underflow are either prevented or result in a panic.
///
/// Atomic counters must provide proper memory ordering for safe concurrent use.
///
/// # Intended use
/// This trait enables custom memory semantics for [`Rime`], such as:
/// - Single-threaded usage via primitive types (e.g. `u8`, `usize`)
/// - Interior mutability via `Cell<T>`
/// - Thread-safe usage via `AtomicU*` types
pub trait Counter: Sized {
    fn new() -> Self;
    fn increment(&mut self);
    fn decrement(&mut self) -> bool;
}

macro_rules! impl_ref_count_for_primitive {
    ($($t:ty),*) => {
        $(
            impl Counter for $t {
                #[inline(always)] fn new() -> Self { 1 }
                #[inline(always)] fn increment(&mut self) { *self += 1 }
                #[inline(always)] fn decrement(&mut self) -> bool {
                    *self -= 1;
                    *self == 0
                }
            }

            impl Counter for std::cell::Cell<$t> {
                #[inline(always)] fn new() -> Self { std::cell::Cell::new(1) }
                #[inline(always)] fn increment(&mut self) { self.set(self.get().checked_add(1).expect("RefCount overflow")); }
                #[inline(always)] fn decrement(&mut self) -> bool {
                    let value = self.get().checked_sub(1).expect("RefCount underflow");
                    self.set(value);
                    value == 0
                }
            }
        )*
    };
}

macro_rules! impl_ref_count_for_atomic {
    ($($atomic:ty),*) => {
        $(
            impl Counter for $atomic {
                #[inline(always)] fn new() -> Self { <$atomic>::new(1) }
                #[inline(always)] fn increment(&mut self) { self.fetch_add(1, Ordering::Relaxed); }
                #[inline(always)] fn decrement(&mut self) -> bool {
                    if self.fetch_sub(1, Ordering::Release) == 1 {
                        fence(Ordering::Acquire); true 
                    } else { false }
                }
            }
        )*
    };
}

impl_ref_count_for_primitive!(u8, u16, u32, u64, u128, usize);
impl_ref_count_for_atomic!(AtomicU8, AtomicU16, AtomicU32, AtomicU64, AtomicUsize);

/// A compact reference-counted pointer for unsized or immutable data.
///
/// `Rime<C, T>` combines a user-defined [`Counter`] `C` with inline allocation of a dynamically sized value `T`. 
/// This is a low-level, flexible alternative to `Rc` or `Arc`, optimized for immutable data and custom memory layouts.
///
/// The pointer layout is:
/// ```text
/// [ C | T ]
///   |   |____ user data (T)
///   |________ reference counter (C)
/// ```
///
/// # Features
/// - Configurable: users choose atomic or non-atomic reference counting
/// - Efficient: counter and data are stored in a single allocation
/// - Flexible: supports unsized types (`str`, `[T]`)
///
/// # Safety
/// - `new` copies the content of a reference into an internal allocation; the input must be valid for reads.
/// - `steal` moves ownership of `T`, which must not be accessed afterward.
/// - Dropping the last clone deallocates the entire block.
///
/// # Example
/// ```
/// use std::sync::atomic::AtomicU8;
/// use kroos::Rime;
///
/// let shared = Rime::<AtomicU8, str>::new("hello world!");
/// let cloned = shared.clone();
///
/// assert_eq!(&*cloned, "hello world!");
/// ```
///
/// # When to use
/// Use `Rime` when you need:
/// - Shared ownership of unsized data
/// - Custom memory management semantics
/// - Low overhead
///
/// # Comparison
/// | Feature               | `Rc` / `Arc` | `Rime`             |
/// |-----------------------|--------------|--------------------|
/// | Works with DSTs       | ❌           | ✅                 |
/// | Atomic counters       | ✅ (`Arc`)   | ✅ (via `Atomic*`) |
/// | Inline allocation     | ❌           | ✅                 |
/// | Custom counter logic  | ❌           | ✅                 |
#[derive(Debug)]
pub struct Rime<C: Counter, T: ?Sized> {
    _marker: PhantomData<(C, T)>,
    counter_ptr: *mut C,
    inner_ptr: *const T,
}

impl<C: Counter, T: Sized> Rime<C, T> {
    /// Constructs a `Rime` from a `Sized` value by moving it into an inline allocation.
    ///
    /// This method stores both the reference counter and the value `D` in a single contiguous block. 
    /// The result is a reference-counted smart pointer with zero external metadata.
    ///
    /// # Panics
    /// Panics if memory allocation fails.
    ///
    /// # Example
    /// ```
    /// use std::sync::atomic::AtomicU8;
    /// use kroos::Rime;
    ///
    /// let rime = Rime::<AtomicU8, String>::steal("hello".to_string());
    /// assert_eq!(&*rime, "hello");
    /// ```
    ///
    /// # Notes
    /// - `steal` takes ownership of the input value
    /// - For dynamically sized values, use [`Rime::new`] instead
    pub fn steal(value: T) -> Self {
        unsafe {
            let c_size = size_of::<C>();
            let layout = Layout::from_size_align_unchecked(
                size_of::<T>() + c_size, 
                align_of::<C>().max(align_of::<T>()));

            let raw = alloc(layout);
            if raw.is_null() {
                dealloc(raw, layout);
                handle_alloc_error(layout);
            }

            let counter_ptr = raw as *mut C;
            write(counter_ptr, C::new());

            let data_ptr = raw.add(c_size) as *mut T;
            write(data_ptr, value);

            Self::from_raw(counter_ptr, data_ptr as *const T)
        }
    }
}


impl<C: Counter, T: ?Sized> Rime<C, T> {
    /// Creates a `Rime` from raw pointers to the counter and data.
    ///
    /// # Safety
    /// This function assumes:
    /// - `counter_ptr` points to a valid `C`
    /// - `inner_ptr` points to a valid `T`
    ///
    /// These must originate from a valid layout created by `Rime`.
    ///
    /// This method is intended for advanced usage (e.g. FFI or custom allocators).
    #[inline(always)]
    pub fn from_raw(counter_ptr: *mut C, inner_ptr: *const T) -> Self {
        Self { _marker: PhantomData, counter_ptr, inner_ptr }
    }

    /// Creates a `Rime` from raw components and metadata for unsized types.
    ///
    /// This is a lower-level variant of [`from_raw`] for constructing dynamically sized values.
    /// Metadata is typically derived via `core::ptr::metadata`.
    ///
    /// # Safety
    /// The caller must ensure that the memory layout corresponds to: `[ counter: C | data: T ]` and that both pointers are valid.
    ///
    /// For example:
    /// ```
    /// use std::ptr::{metadata, from_raw_parts};
    /// use kroos::Rime;
    ///
    /// let slice: &[u8] = &[1, 2, 3];
    /// let meta = metadata(slice);
    /// let r = Rime::<u8, [u8]>::from_raw_parts(ptr_to_counter, ptr_to_data, meta);
    /// ```
    #[inline(always)]
    pub fn from_raw_parts(counter_ptr: *mut C, inner_ptr: *mut u8, metadata: <T as Pointee>::Metadata) -> Self {
        Self::from_raw(counter_ptr, from_raw_parts::<T>(inner_ptr, metadata))
    }

    /// Constructs a `Rime` by copying the contents of a reference into the allocation.
    ///
    /// The resulting pointer owns its own allocation and behaves like an `Arc` or `Rc`
    /// clone, but with memory layout tightly packed and under user control.
    ///
    /// # Safety
    /// The input reference must remain valid during construction. Internally,
    /// the referenced bytes are copied to heap memory.
    ///
    /// # Example
    /// ```
    /// use kroos::Rime;
    ///
    /// let r = Rime::<u8, str>::new("abc");
    /// assert_eq!(&*r, "abc");
    /// ```
    pub fn new(value: &T) -> Self {
        unsafe {
            let t_size = size_of_val(value);
            let c_size = size_of::<C>();

            let layout = Layout::from_size_align_unchecked(
                c_size + t_size,
                align_of::<C>().max(align_of_val(value))
            );
            
            let raw = alloc(layout);
            if raw.is_null() { 
                dealloc(raw, layout);
                handle_alloc_error(layout);
            }
            
            let counter_ptr = raw as *mut C;
            write(counter_ptr, C::new());

            let inner_ptr = raw.add(c_size);
            copy_nonoverlapping(value as *const T as *const u8, inner_ptr, t_size);

            Self::from_raw_parts(counter_ptr, inner_ptr, metadata(value))
        }
    }
    
    /// Returns a raw fat pointer to the heap-allocated value.
    ///
    /// This includes metadata (e.g. length for slices, vtable for trait objects)
    /// and remains valid as long as the `Rime` instance (and any clones) are alive.
    ///
    /// # Safety
    /// This pointer must not be dereferenced after all `Rime` clones are dropped.
    #[inline(always)]
    pub fn as_ptr(&self) -> *const T { 
        self.inner_ptr
    }

    /// Returns a mutable raw fat pointer to the heap-allocated value.
    ///
    /// Enables in-place mutation of the value stored by this `Rime`.
    ///
    /// # Safety
    /// - You must ensure there are no other aliases to the same memory (including other `Rime` clones).
    /// - The memory must not be mutated in a way that violates the type’s layout or Rust’s aliasing rules.
    /// - The `Rime` must remain alive for the duration of use, and must not be accessed concurrently from other threads.
    #[inline(always)]
    pub fn as_mut_ptr(&self) -> *mut T { 
        self.inner_ptr.cast_mut()
    }
}

impl<C: Counter, T: ?Sized> Drop for Rime<C, T> {
    #[inline(always)]
    fn drop(&mut self) {
        unsafe {
            if (*self.counter_ptr).decrement() {
                let inner = &*self.inner_ptr;
                dealloc(
                    self.counter_ptr.cast(), 
                    Layout::from_size_align_unchecked(
                        size_of::<C>() + size_of_val(inner),
                        align_of::<C>().max(align_of_val(inner))
                    )
                );
            }
        }
    }
}

impl<C: Counter, T: ?Sized> Clone for Rime<C, T> {
    #[inline]
    fn clone(&self) -> Self {
        unsafe { (*self.counter_ptr).increment() }
        Self {
            inner_ptr: self.inner_ptr,
            counter_ptr: self.counter_ptr,
            _marker: PhantomData
        }
    }
}

impl<C: Counter, T: ?Sized> From<&Rime<C, T>> for Rime<C, T> {
    #[inline(always)]
    fn from(value: &Rime<C, T>) -> Self {
        value.clone()
    }
}

impl<C: Counter, T: ?Sized> AsRef<T> for Rime<C, T> {
    #[inline]
    fn as_ref(&self) -> &T {
        unsafe { &*self.inner_ptr }
    }
}

impl<C: Counter, T: ?Sized> std::ops::Deref for Rime<C, T> {
    type Target = T;

    #[inline]
    fn deref(&self) -> &Self::Target {
        unsafe { &*self.inner_ptr }
    }
}

impl<C: Counter, T: ?Sized> Eq for Rime<C, T> { }
impl<C: Counter, T: ?Sized> PartialEq for Rime<C, T> {
    #[inline(always)]
    fn eq(&self, other: &Self) -> bool {
        addr_eq(self.inner_ptr, other.inner_ptr)
    }
}

impl<C: Counter, T: ?Sized + Ord> Ord for Rime<C, T> {
    #[inline(always)]
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        unsafe { (&*self.inner_ptr).cmp(&*other) }
    }
}

impl<C: Counter, T: ?Sized + PartialOrd> PartialOrd for Rime<C, T> {
    #[inline(always)]
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        unsafe { (&*self.inner_ptr).partial_cmp(&*other) }
    }
}

impl<C: Counter, T: ?Sized + Hash> Hash for Rime<C, T> {
    #[inline(always)]
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        unsafe { (&*self.inner_ptr).hash(state) }
    }
}

unsafe impl<C: Counter + Send, T: ?Sized + Send> Send for Rime<C, T> {}
unsafe impl<C: Counter + Sync, T: ?Sized + Sync> Sync for Rime<C, T> {}

#[cfg(test)]
mod tests {
    use std::sync::atomic::*;
    use super::*;

    #[test]
    fn test_basic_clone_and_deref() {
        let rime = Rime::<u8, str>::new("hello");
        assert_eq!(&*rime, "hello");

        let cloned = rime.clone();
        assert_eq!(&*cloned, "hello");
        assert_eq!(rime.as_ptr(), cloned.as_ptr()); // Same backing memory
    }

    #[test]
    fn test_equality_and_ordering() {
        let r1 = Rime::<u8, str>::new("abc");
        let r2 = r1.clone();
        let r3 = Rime::<u8, str>::new("abc");

        assert_eq!(r1, r2);
        assert_ne!(r1, r3); // Different allocations
        assert!(r1 <= r2);
        assert!(r3 >= r1);
    }

    #[test]
    fn test_drop_deallocates() {
        use std::cell::RefCell;
        use std::rc::Rc;

        struct DropCounter(Rc<RefCell<u8>>);
        impl Drop for DropCounter {
            fn drop(&mut self) {
                *self.0.borrow_mut() += 1;
            }
        }

        let dropped = Rc::new(RefCell::new(0));
        {
            let counter = DropCounter(dropped.clone());
            let r1 = Rime::<usize, _>::new(&counter);
            let _r2 = r1.clone(); // two references
        }

        assert_eq!(*dropped.borrow(), 1); // Dropped once after refcount hits zero
    }

    #[test]
    fn test_atomic_clone_thread_safe() {
        use std::thread;

        let rime = Rime::<AtomicUsize, str>::new("multi");
        let mut handles = vec![];

        for _ in 0..10 {
            let cloned = rime.clone();
            handles.push(thread::spawn(move || {
                assert_eq!(&*cloned, "multi");
            }));
        }

        for handle in handles {
            handle.join().unwrap();
        }

        assert_eq!(&*rime, "multi");
    }

    #[test]
    fn test_as_ref_and_conversion() {
        let rime = Rime::<u8, str>::new("as_ref test");
        let rime2: Rime<u8, str> = (&rime).into();

        assert_eq!(rime.as_ref(), rime2.as_ref());
    }
}
