use std::{alloc::*, hash::Hash, marker::PhantomData, ptr::*};

/// A low-level heap-allocated wrapper for dynamically-sized types (`?Sized`) without ownership semantics.
///
/// `Flake` allows allocation of types like `str` or `[T]` directly on the heap, without invoking
/// constructors or destructors. It is intended for immutable, trivially-copyable data,
/// and does not manage logical ownership or lifetimes beyond raw allocation.
///
/// # Safety
/// - `Flake` only copies memory — it does not respect `Drop`, `Clone`, or other semantic constraints.
/// - Intended for POD-like types: `str`, `[u8]`, etc. Using it with `Drop` types is undefined behavior.
/// - The source reference must point to a valid, fully-initialized object.
///
/// # Use cases
/// - Alternative to `Box<str>` or `Box<[u8]>` in performance-critical code.
/// - Zero-cost abstraction over raw allocation for immutable data.
/// - Efficient heap storage for `?Sized` types.
///
/// # When *not* to use
/// - If the type implements `Drop`, or contains references or heap resources.
/// - As a general-purpose container — this is a specialized primitive.
///
/// See [`Rime`] for reference-counted DST support.
pub struct Flake<T: ?Sized> {
    _marker: PhantomData<T>,
    inner_ptr: *const T,
}

impl<T: Sized> Flake<T> {
    /// Moves a `Sized` value into a `Flake` by allocating and transferring ownership.
    ///
    /// This method performs a direct move into the heap, allocating space for the value and constructing a `Flake` to wrap it. 
    /// It differs from [`Flake::new`] in that it transfers ownership instead of copying from a reference.
    ///
    /// # Safety
    /// - Only works with `Sized` types. For DSTs, use [`Flake::new`] instead.
    /// - The value must not be used after the move.
    ///
    /// # Panics
    /// Panics if heap allocation fails.
    ///
    /// # Example
    /// ```
    /// use kroos::Flake;
    /// 
    /// let flake = Flake::steal(String::from("owned"));
    /// assert_eq!(&*flake, "owned");
    /// ```
    pub fn steal(value: T) -> Self {
        unsafe {
            let layout = Layout::new::<T>();
            let raw = alloc(layout);
            if raw.is_null() { 
                dealloc(raw, layout);
                handle_alloc_error(layout);
            }

            write(raw as *mut T, value);

            Self::from_raw(raw as *const T)
        } 
    }
}

impl<T: ?Sized> Flake<T> {
    /// Constructs a `Flake` from a raw fat pointer to a heap-allocated value.
    ///
    /// # Safety
    /// - The pointer must originate from a valid heap allocation compatible with `Layout::for_value`.
    /// - Caller is responsible for ensuring exclusive ownership and valid metadata.
    /// - `Flake` will take ownership and deallocate the memory on `Drop`.
    #[inline(always)]
    pub unsafe fn from_raw(ptr: *const T) -> Self {
        Self { _marker: PhantomData, inner_ptr: ptr }
    }

    /// Constructs a `Flake` from a data pointer and metadata, forming a valid fat pointer.
    ///
    /// # Safety
    /// - The `ptr` must point to a valid allocation for `D`.
    /// - The `metadata` must match the type's expected layout (e.g., length for slices).
    /// - Same ownership guarantees as [`from_raw`] apply.
    #[inline(always)]
    pub unsafe fn from_raw_parts(ptr: *const u8, metadata: <T as Pointee>::Metadata) -> Self {
        Self { _marker: PhantomData, inner_ptr: from_raw_parts::<T>(ptr, metadata) }
    }

    /// Copies a `?Sized` value from a reference into the heap and returns a `Flake`.
    ///
    /// This function allocates memory equal to the size of the value, and copies the raw bytes into the heap.
    /// The original value is not consumed or moved.
    ///
    /// # Safety
    /// - Only use with types that can be safely duplicated by memory copy.
    /// - Do **not** use with types that implement `Drop` or manage internal state.
    ///
    /// # Panics
    /// Panics if heap allocation fails.
    ///
    /// # Example
    /// ```
    /// use kroos::Flake;
    /// 
    /// let slice: &[u8] = &[1, 2, 3];
    /// let flake = Flake::new(slice);
    /// assert_eq!(&*flake, &[1, 2, 3]);
    /// ```
    pub fn new(value: &T) -> Self {
        unsafe {
            let layout = Layout::for_value(value);
            let raw = alloc(layout);
            if raw.is_null() { 
                dealloc(raw, layout);
                handle_alloc_error(layout);
            }

            copy_nonoverlapping(value as *const T as *const u8, raw, size_of_val(value));

            Self::from_raw_parts(raw, metadata(value))
        }
    }

    /// Forcibly drops the heap value stored in the `Flake`.
    ///
    /// # Safety
    /// - Only call if the inner value was never dropped elsewhere.
    /// - Use only with types that implement `Drop` and were intentionally stored using unsafe means (e.g., `ManuallyDrop`).
    /// - Do not use on raw data like `str` or `[u8]` — this will cause UB.
    #[inline(always)]
    pub unsafe fn drop_inner(&mut self) {
        drop_in_place(self.inner_ptr.cast_mut());
    }

    /// Returns a raw fat pointer to the value stored in the heap.
    ///
    /// Includes metadata (length, vtable, etc.), and is valid while the `Flake` lives.
    ///
    /// # Safety
    /// Do not dereference the pointer after the `Flake` is dropped.
    #[inline(always)]
    pub fn as_ptr(&self) -> *const T {
        self.inner_ptr
    }

    /// Returns a mutable raw fat pointer to the value stored in the heap.
    ///
    /// Allows in-place mutation of the heap value. Use with caution.
    ///
    /// # Safety
    /// - Mutating the memory must not violate the original type's layout or invariants.
    /// - The `Flake` must be alive and not concurrently accessed.
    /// - You must ensure there are no aliasing references.
    #[inline(always)]
    pub fn as_mut_ptr(&self) -> *mut T {
        self.inner_ptr.cast_mut()
    }
}

impl<T: ?Sized> Drop for Flake<T> {
    fn drop(&mut self) {
        unsafe {
            dealloc(self.inner_ptr as *mut u8, Layout::for_value(&*self.inner_ptr));
        }
    }
}

impl<T: ?Sized> AsRef<T> for Flake<T> {
    #[inline]
    fn as_ref(&self) -> &T {
        unsafe { &*self.inner_ptr }
    }
}

impl<T: ?Sized> std::ops::Deref for Flake<T> {
    type Target = T;

    #[inline]
    fn deref(&self) -> &Self::Target {
        unsafe { &*self.inner_ptr }
    }
}


impl<T: ?Sized> Eq for Flake<T> { }
impl<T: ?Sized> PartialEq for Flake<T> {
    #[inline(always)]
    fn eq(&self, other: &Self) -> bool {
        addr_eq(self.inner_ptr, other.inner_ptr)
    }
}

impl<T: ?Sized + Ord> Ord for Flake<T> {
    #[inline(always)]
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        unsafe { (&*self.inner_ptr).cmp(&*other) }
    }
}

impl<T: ?Sized + PartialOrd> PartialOrd for Flake<T> {
    #[inline(always)]
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        unsafe { (&*self.inner_ptr).partial_cmp(&*other) }
    }
}

impl<T: ?Sized + Hash> Hash for Flake<T> {
    #[inline(always)]
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        unsafe { (&*self.inner_ptr).hash(state) }
    }
}

unsafe impl<T: ?Sized> Send for Flake<T> {}
unsafe impl<T: ?Sized> Sync for Flake<T> {}

#[cfg(test)]
mod tests {
    use super::Flake;

    #[test]
    fn flake_from_str() {
        let input: &str = "hello";
        let flake = Flake::new(input);
        assert_eq!(&*flake, "hello");
        assert_eq!(flake.as_ref(), "hello");
    }

    #[test]
    fn flake_from_slice() {
        let slice: &[u8] = &[1, 2, 3, 4];
        let flake = Flake::new(slice);
        assert_eq!(&*flake, &[1, 2, 3, 4]);
    }

    #[test]
    fn flake_eq_cmp_ord() {
        let a = Flake::new("abc");
        let b = Flake::new("abc");
        let c = Flake::new("xyz");

        assert!(a != b); // different pointers
        assert_eq!(*a, *b); // same contents
        assert!(a < c);
    }

    #[test]
    fn flake_mutate_bytes() {
        let slice: &[u8] = &[1, 2, 3];
        let flake = Flake::new(slice);

        unsafe {
            let ptr = flake.as_mut_ptr();
            (*ptr)[1] = 9;
        }

        assert_eq!(&*flake, &[1, 9, 3]);
    }

    #[test]
    fn flake_steal_string() {
        let string = String::from("yo");
        let flake = Flake::steal(string);
        assert_eq!(&*flake, "yo");
    }

    #[test]
    fn flake_from_raw_manual() {
        use std::{alloc::*, ptr::copy_nonoverlapping, ptr::from_raw_parts_mut};

        let slice: &[u8] = &[10, 20, 30];
        let layout = Layout::for_value(slice);
        unsafe {
            let raw = alloc(layout);
            assert!(!raw.is_null());
            copy_nonoverlapping(slice.as_ptr(), raw, slice.len());

            let ptr = from_raw_parts_mut::<[u8]>(raw as *mut (), slice.len());
            let flake = Flake::from_raw(ptr);
            assert_eq!(&*flake, &[10, 20, 30]);
        }
    }
}
