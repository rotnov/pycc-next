//! Free lists for the runtime's small fixed-size allocations, as CPython
//! keeps them for tuples, floats and frames.
//!
//! Compiled code allocates and frees its small objects at a high rate: a
//! parser that slices and extends `tuple[str, ...]` keys creates and drops
//! hundreds of tuple objects per document, each a `PyListObj` header and a
//! data block, and every string slice is a `PyStrObj`. Through `malloc` and
//! `free` each pair costs on the order of a hundred instructions; a block
//! taken from and returned to a [`Pool`] costs a few.
//!
//! A pool holds raw blocks of **one layout**, chosen by its user (the
//! header of a list, a tuple's data block of one length, a `str` object),
//! allocated with the global allocator and returned to it when the pool is
//! full. A block in a pool is uninitialized memory: the user writes a whole
//! value into a block it takes and drops (or reads out) the value before
//! giving the block back.
//!
//! **Threads.** The runtime is called from one thread at a time in practice
//! (an `--ext` module runs under the GIL), but nothing here relies on it:
//! a pool is guarded by a try-lock, and a caller that finds it held simply
//! uses the global allocator for that one block.
//!
//! The pooled blocks stay allocated until the process exits (bounded by
//! each pool's capacity); they are reachable from the pool, so a leak
//! checker reports them as still reachable, never as lost.

use std::alloc::Layout;
use std::cell::UnsafeCell;
use std::sync::atomic::{AtomicBool, Ordering};

/// A bounded stack of free blocks of one layout.
pub(crate) struct Pool<const N: usize> {
    locked: AtomicBool,
    inner: UnsafeCell<Stack<N>>,
}

struct Stack<const N: usize> {
    len: usize,
    blocks: [*mut u8; N],
}

// SAFETY: `inner` is only touched while `locked` is held (`with`).
unsafe impl<const N: usize> Sync for Pool<N> {}

impl<const N: usize> Pool<N> {
    pub(crate) const fn new() -> Self {
        Pool {
            locked: AtomicBool::new(false),
            inner: UnsafeCell::new(Stack {
                len: 0,
                blocks: [std::ptr::null_mut(); N],
            }),
        }
    }

    /// Runs `f` on the stack, or answers `None` when another thread holds it.
    fn with<R>(&self, f: impl FnOnce(&mut Stack<N>) -> R) -> Option<R> {
        if self
            .locked
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            return None;
        }
        // SAFETY: the lock is held, so no one else touches the stack.
        let result = f(unsafe { &mut *self.inner.get() });
        self.locked.store(false, Ordering::Release);
        Some(result)
    }

    /// A free block of `layout` (the pool's one layout): a pooled one, or a
    /// fresh allocation.
    pub(crate) fn take(&self, layout: Layout) -> *mut u8 {
        let pooled = self
            .with(|stack| {
                if stack.len == 0 {
                    return std::ptr::null_mut();
                }
                stack.len -= 1;
                stack.blocks[stack.len]
            })
            .unwrap_or(std::ptr::null_mut());
        if !pooled.is_null() {
            return pooled;
        }
        // SAFETY: every pooled layout has a non-zero size.
        let block = unsafe { std::alloc::alloc(layout) };
        if block.is_null() {
            std::alloc::handle_alloc_error(layout);
        }
        block
    }

    /// Returns `block`, a block of `layout` holding no live value, to the
    /// pool, or to the global allocator when the pool is full.
    ///
    /// # Safety
    /// `block` must have been allocated with `layout` by the global
    /// allocator (or taken from this pool), and nothing may use it after.
    pub(crate) unsafe fn give(&self, block: *mut u8, layout: Layout) {
        let kept = self
            .with(|stack| {
                if stack.len == N {
                    return false;
                }
                stack.blocks[stack.len] = block;
                stack.len += 1;
                true
            })
            .unwrap_or(false);
        if !kept {
            // SAFETY (caller): allocated with `layout`.
            unsafe { std::alloc::dealloc(block, layout) };
        }
    }

    /// The number of pooled blocks (tests).
    #[cfg(test)]
    pub(crate) fn pooled(&self) -> usize {
        self.with(|stack| stack.len).unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_given_block_is_taken_again_and_a_full_pool_frees() {
        static POOL: Pool<2> = Pool::new();
        let layout = Layout::new::<[u64; 3]>();
        let a = POOL.take(layout);
        let b = POOL.take(layout);
        let c = POOL.take(layout);
        assert!(!a.is_null() && a != b && b != c);
        unsafe {
            POOL.give(a, layout);
            POOL.give(b, layout);
            // Full: `c` goes back to the allocator.
            POOL.give(c, layout);
        }
        assert_eq!(POOL.pooled(), 2);
        assert_eq!(POOL.take(layout), b);
        assert_eq!(POOL.take(layout), a);
        assert_eq!(POOL.pooled(), 0);
        unsafe {
            std::alloc::dealloc(a, layout);
            std::alloc::dealloc(b, layout);
        }
    }

    #[test]
    fn a_held_pool_falls_back_to_the_allocator() {
        static POOL: Pool<4> = Pool::new();
        let layout = Layout::new::<u64>();
        let a = POOL.take(layout);
        unsafe { POOL.give(a, layout) };
        POOL.locked.store(true, Ordering::Relaxed);
        let b = POOL.take(layout);
        assert_ne!(a, b, "a held pool hands out a fresh block");
        unsafe { POOL.give(b, layout) };
        POOL.locked.store(false, Ordering::Relaxed);
        assert_eq!(POOL.pooled(), 1);
        assert_eq!(POOL.take(layout), a);
        unsafe { std::alloc::dealloc(a, layout) };
    }
}
