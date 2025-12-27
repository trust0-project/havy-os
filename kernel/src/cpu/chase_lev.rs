//! Chase-Lev Work-Stealing Deque
//!
//! A lock-free deque optimized for work-stealing schedulers.
//! The owner has exclusive LIFO access (push/pop), while thieves
//! use CAS to steal from the opposite end (FIFO).
//!
//! Based on: "Dynamic Circular Work-Stealing Deque" by Chase & Lev (2005)
//!
//! ## Memory Model
//!
//! - `bottom`: Modified only by owner (relaxed writes, acquire reads)
//! - `top`: Shared, uses CAS for stealing coordination
//! - `buffer`: Array access uses proper fence ordering

use alloc::boxed::Box;
use alloc::sync::Arc;
use core::cell::UnsafeCell;
use core::mem::MaybeUninit;
use core::sync::atomic::{AtomicIsize, AtomicPtr, Ordering, fence};

/// Initial buffer capacity (must be power of 2)
const INITIAL_CAPACITY: usize = 32;

/// A lock-free work-stealing deque
///
/// - Owner: calls `push()` and `pop()` (single-threaded, LIFO)
/// - Thieves: call `steal()` concurrently (CAS-protected, FIFO)
pub struct WorkStealingDeque<T> {
    /// Bottom index - owned by the single producer/consumer
    bottom: AtomicIsize,
    /// Top index - shared, modified via CAS
    top: AtomicIsize,
    /// Circular buffer (can grow dynamically)
    buffer: AtomicPtr<Buffer<T>>,
}

/// Circular buffer for the deque
struct Buffer<T> {
    /// Capacity (always power of 2)
    capacity: usize,
    /// Storage
    data: Box<[UnsafeCell<MaybeUninit<T>>]>,
}

impl<T> Buffer<T> {
    fn new(capacity: usize) -> *mut Self {
        assert!(capacity.is_power_of_two());
        
        let data: Box<[UnsafeCell<MaybeUninit<T>>]> = (0..capacity)
            .map(|_| UnsafeCell::new(MaybeUninit::uninit()))
            .collect();
        
        Box::into_raw(Box::new(Self { capacity, data }))
    }
    
    fn capacity(&self) -> usize {
        self.capacity
    }
    
    /// Get element at index (circular)
    unsafe fn get(&self, index: isize) -> T
    where
        T: Clone,
    {
        let idx = (index as usize) & (self.capacity - 1);
        (*self.data[idx].get()).assume_init_ref().clone()
    }
    
    /// Put element at index (circular)
    unsafe fn put(&self, index: isize, value: T) {
        let idx = (index as usize) & (self.capacity - 1);
        self.data[idx].get().write(MaybeUninit::new(value));
    }
    
    /// Grow buffer, copying elements from old to new
    unsafe fn grow(&self, bottom: isize, top: isize) -> *mut Self
    where
        T: Clone,
    {
        let new_capacity = self.capacity * 2;
        let new_buf = Buffer::new(new_capacity);
        
        for i in top..bottom {
            (*new_buf).put(i, self.get(i));
        }
        
        new_buf
    }
}

/// Result of a steal attempt
pub enum StealResult<T> {
    /// Successfully stole an item
    Success(T),
    /// Deque is empty
    Empty,
    /// CAS failed, retry may succeed
    Retry,
}

impl<T: Clone> WorkStealingDeque<T> {
    /// Create a new empty deque (const, buffer allocated on first push)
    pub const fn new() -> Self {
        Self {
            bottom: AtomicIsize::new(0),
            top: AtomicIsize::new(0),
            buffer: AtomicPtr::new(core::ptr::null_mut()),
        }
    }
    
    /// Ensure buffer is allocated (lazy initialization)
    #[inline]
    fn ensure_buffer(&self) -> *mut Buffer<T> {
        let buf = self.buffer.load(Ordering::Acquire);
        if buf.is_null() {
            let new_buf = Buffer::new(INITIAL_CAPACITY);
            // Try to be the one to initialize
            match self.buffer.compare_exchange(
                core::ptr::null_mut(),
                new_buf,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => new_buf,
                Err(existing) => {
                    // Another thread initialized, drop ours
                    unsafe { drop(Box::from_raw(new_buf)); }
                    existing
                }
            }
        } else {
            buf
        }
    }
    
    /// Push an item onto the bottom of the deque (owner only)
    pub fn push(&self, value: T) {
        let bottom = self.bottom.load(Ordering::Relaxed);
        let top = self.top.load(Ordering::Acquire);
        let buffer = self.ensure_buffer();
        
        unsafe {
            let size = bottom - top;
            let capacity = (*buffer).capacity() as isize;
            
            // Grow if full
            let buffer = if size >= capacity - 1 {
                let new_buf = (*buffer).grow(bottom, top);
                self.buffer.store(new_buf, Ordering::Release);
                new_buf
            } else {
                buffer
            };
            
            (*buffer).put(bottom, value);
        }
        
        fence(Ordering::Release);
        self.bottom.store(bottom + 1, Ordering::Relaxed);
    }
    
    /// Pop an item from the bottom of the deque (owner only)
    pub fn pop(&self) -> Option<T> {
        let bottom = self.bottom.load(Ordering::Relaxed) - 1;
        self.bottom.store(bottom, Ordering::Relaxed);
        
        fence(Ordering::SeqCst);
        
        let top = self.top.load(Ordering::Relaxed);
        
        if top <= bottom {
            // Non-empty
            let buffer = self.buffer.load(Ordering::Relaxed);
            if buffer.is_null() {
                // Buffer not yet allocated - deque is empty
                self.bottom.store(top, Ordering::Relaxed);
                return None;
            }
            let value = unsafe { (*buffer).get(bottom) };
            
            if top == bottom {
                // Last element - race with thieves
                if self.top.compare_exchange(
                    top,
                    top + 1,
                    Ordering::SeqCst,
                    Ordering::Relaxed,
                ).is_err() {
                    // Lost race to thief
                    self.bottom.store(top + 1, Ordering::Relaxed);
                    return None;
                }
                self.bottom.store(top + 1, Ordering::Relaxed);
            }
            
            Some(value)
        } else {
            // Empty
            self.bottom.store(top, Ordering::Relaxed);
            None
        }
    }
    
    /// Steal an item from the top of the deque (thief)
    pub fn steal(&self) -> StealResult<T> {
        let top = self.top.load(Ordering::Acquire);
        fence(Ordering::SeqCst);
        let bottom = self.bottom.load(Ordering::Acquire);
        
        if top >= bottom {
            return StealResult::Empty;
        }
        
        let buffer = self.buffer.load(Ordering::Acquire);
        if buffer.is_null() {
            return StealResult::Empty;
        }
        let value = unsafe { (*buffer).get(top) };
        
        if self.top.compare_exchange(
            top,
            top + 1,
            Ordering::SeqCst,
            Ordering::Relaxed,
        ).is_ok() {
            StealResult::Success(value)
        } else {
            StealResult::Retry
        }
    }
    
    /// Check if deque is empty (approximate)
    pub fn is_empty(&self) -> bool {
        let top = self.top.load(Ordering::Relaxed);
        let bottom = self.bottom.load(Ordering::Relaxed);
        bottom <= top
    }
    
    /// Get approximate length
    pub fn len(&self) -> usize {
        let top = self.top.load(Ordering::Relaxed);
        let bottom = self.bottom.load(Ordering::Relaxed);
        if bottom > top {
            (bottom - top) as usize
        } else {
            0
        }
    }
}

impl<T> Default for WorkStealingDeque<T>
where
    T: Clone,
{
    fn default() -> Self {
        Self::new()
    }
}

// Safety: WorkStealingDeque is designed for multi-threaded access
unsafe impl<T: Send> Send for WorkStealingDeque<T> {}
unsafe impl<T: Send> Sync for WorkStealingDeque<T> {}

// ═══════════════════════════════════════════════════════════════════════════════
// TESTS
// ═══════════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_push_pop_basic() {
        let deque: WorkStealingDeque<i32> = WorkStealingDeque::new();
        
        deque.push(1);
        deque.push(2);
        deque.push(3);
        
        // LIFO order for owner
        assert_eq!(deque.pop(), Some(3));
        assert_eq!(deque.pop(), Some(2));
        assert_eq!(deque.pop(), Some(1));
        assert_eq!(deque.pop(), None);
    }
    
    #[test]
    fn test_steal_basic() {
        let deque: WorkStealingDeque<i32> = WorkStealingDeque::new();
        
        deque.push(1);
        deque.push(2);
        deque.push(3);
        
        // FIFO order for thieves
        match deque.steal() {
            StealResult::Success(v) => assert_eq!(v, 1),
            _ => panic!("Expected success"),
        }
        match deque.steal() {
            StealResult::Success(v) => assert_eq!(v, 2),
            _ => panic!("Expected success"),
        }
    }
    
    #[test]
    fn test_empty_deque() {
        let deque: WorkStealingDeque<i32> = WorkStealingDeque::new();
        
        assert!(deque.is_empty());
        assert_eq!(deque.pop(), None);
        assert!(matches!(deque.steal(), StealResult::Empty));
    }
    
    #[test]
    fn test_len() {
        let deque: WorkStealingDeque<i32> = WorkStealingDeque::new();
        
        assert_eq!(deque.len(), 0);
        deque.push(1);
        assert_eq!(deque.len(), 1);
        deque.push(2);
        assert_eq!(deque.len(), 2);
        deque.pop();
        assert_eq!(deque.len(), 1);
    }
    
    #[test]
    fn test_grow() {
        let deque: WorkStealingDeque<i32> = WorkStealingDeque::new();
        
        // Push more than initial capacity
        for i in 0..100 {
            deque.push(i);
        }
        
        assert_eq!(deque.len(), 100);
        
        // Pop all - should be LIFO
        for i in (0..100).rev() {
            assert_eq!(deque.pop(), Some(i));
        }
    }
}
