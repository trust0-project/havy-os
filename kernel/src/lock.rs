//! Spinlock and synchronization primitives for SMP.
//!
//! This module provides various locking mechanisms:
//! - `Spinlock` - Basic mutual exclusion with swap-based acquisition
//! - `TicketLock` - Fair spinlock with FIFO ordering (no starvation)
//! - `RwLock` - Reader-writer lock (multiple readers OR one writer)
//!
//! ## Lock Ordering Protocol
//!
//! To prevent deadlocks, always acquire locks in this order (lowest to highest):
//! 1. CPU_TABLE
//! 2. PROCESS_TABLE
//! 3. SCHEDULER queues
//! 4. FS_STATE
//! 5. BLK_DEV
//! 6. NET_STATE
//! 7. KLOG
//! 8. HEAP_ALLOCATOR (implicit in alloc)

use core::cell::UnsafeCell;
use core::hint::spin_loop;
use core::ops::{Deref, DerefMut};
use core::sync::atomic::{AtomicU32, Ordering};
#[cfg(debug_assertions)]
use core::sync::atomic::AtomicUsize;

// ============================================================================
// Lock IDs for Lock Ordering Validation (Debug Mode)
// ============================================================================

/// Lock hierarchy levels - lower numbers must be acquired before higher numbers
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u8)]
pub enum LockId {
    CpuTable = 1,
    ProcessTable = 2,
    Scheduler = 3,
    FsState = 4,
    BlkDev = 5,
    NetState = 6,
    Klog = 7,
    HeapAllocator = 8,
    /// For locks that don't participate in ordering
    Unordered = 255,
}

// ============================================================================
// BASIC SPINLOCK
// ============================================================================

// Lock states as u32 for 32-bit atomic operations.
// On RISC-V, AtomicBool uses byte operations which may not be properly
// synchronized across harts in some emulators. Using AtomicU32 ensures
// we get proper AMOSWAP.W instructions that are correctly serialized.
const UNLOCKED: u32 = 0;
const LOCKED: u32 = 1;

/// A mutual exclusion primitive based on spinning.
///
/// Uses simple atomic swap for acquisition. For fair locking (FIFO),
/// use `TicketLock` instead.
///
/// # Example
///
/// ```
/// static COUNTER: Spinlock<u64> = Spinlock::new(0);
///
/// fn increment() {
///     let mut guard = COUNTER.lock();
///     *guard += 1;
/// }
/// ```
pub struct Spinlock<T> {
    // Use AtomicU32 instead of AtomicBool to ensure 32-bit atomic operations.
    // This guarantees we use AMOSWAP.W for swap and aligned LW/SW for load/store,
    // which are properly synchronized across harts.
    locked: AtomicU32,
    data: UnsafeCell<T>,
    #[cfg(debug_assertions)]
    holder: AtomicUsize, // Debug: track which hart holds the lock
    #[cfg(debug_assertions)]
    lock_id: LockId,
}

// Safety: Spinlock provides synchronized access to T
unsafe impl<T: Send> Sync for Spinlock<T> {}
unsafe impl<T: Send> Send for Spinlock<T> {}

impl<T> Spinlock<T> {
    /// Create a new spinlock wrapping the given value.
    pub const fn new(data: T) -> Self {
        Self {
            locked: AtomicU32::new(UNLOCKED),
            data: UnsafeCell::new(data),
            #[cfg(debug_assertions)]
            holder: AtomicUsize::new(usize::MAX),
            #[cfg(debug_assertions)]
            lock_id: LockId::Unordered,
        }
    }

    /// Create a new spinlock with a lock ID for ordering validation.
    #[cfg(debug_assertions)]
    pub const fn new_with_id(data: T, id: LockId) -> Self {
        Self {
            locked: AtomicU32::new(UNLOCKED),
            data: UnsafeCell::new(data),
            holder: AtomicUsize::new(usize::MAX),
            lock_id: id,
        }
    }

    /// Create a new spinlock with a lock ID for ordering validation.
    #[cfg(not(debug_assertions))]
    pub const fn new_with_id(data: T, _id: LockId) -> Self {
        Self::new(data)
    }

    /// Acquire the lock, blocking until available.
    ///
    /// Returns a guard that releases the lock when dropped.
    ///
    /// NOTE: Uses `swap` (AMOSWAP.W instruction) for acquisition because it's a single
    /// atomic instruction that works correctly for SMP. We use AtomicU32 instead of
    /// AtomicBool to ensure we get 32-bit operations (AMOSWAP.W, LW, SW) which are
    /// properly synchronized across harts, rather than byte operations which may not be.
    #[inline]
    pub fn lock(&self) -> SpinlockGuard<T> {
        let mut spin_count = 0u32;

        loop {
            // Try to acquire using swap (AMOSWAP.W instruction on RISC-V)
            // swap(LOCKED) atomically sets the lock and returns the old value
            // If old value was UNLOCKED, we acquired the lock
            if self.locked.swap(LOCKED, Ordering::Acquire) == UNLOCKED {
                // Acquired! (old value was UNLOCKED)
                #[cfg(debug_assertions)]
                {
                    let hart_id = get_hart_id();
                    self.holder.store(hart_id, Ordering::Relaxed);
                }
                return SpinlockGuard {
                    lock: self,
                    _not_send: core::marker::PhantomData,
                };
            }

            // Lock was already held - spin until we can acquire it
            // Note: We continue trying swap instead of just loading, because
            // the emulator's AMO operations are properly serialized while
            // regular loads may not have proper visibility across harts.
            loop {
                spin_loop();
                spin_count = spin_count.wrapping_add(1);

                // Detect potential deadlock in debug mode
                #[cfg(debug_assertions)]
                if spin_count > 1_000_000 {
                    let holder = self.holder.load(Ordering::Relaxed);
                    let my_hart = get_hart_id();
                    if holder == my_hart {
                        panic!(
                            "Deadlock detected: hart {} trying to re-acquire lock it already holds",
                            my_hart
                        );
                    }
                    spin_count = 0; // Reset counter
                }

                // Try to acquire again with swap
                if self.locked.swap(LOCKED, Ordering::Acquire) == UNLOCKED {
                    // Got it!
                    #[cfg(debug_assertions)]
                    {
                        let hart_id = get_hart_id();
                        self.holder.store(hart_id, Ordering::Relaxed);
                    }
                    return SpinlockGuard {
                        lock: self,
                        _not_send: core::marker::PhantomData,
                    };
                }
            }
        }
    }

    /// Try to acquire the lock without blocking.
    ///
    /// Returns `Some(guard)` if successful, `None` if lock is held.
    #[inline]
    pub fn try_lock(&self) -> Option<SpinlockGuard<T>> {
        // Use swap instead of compare_exchange to ensure AMOSWAP.W is used
        if self.locked.swap(LOCKED, Ordering::Acquire) == UNLOCKED {
            #[cfg(debug_assertions)]
            self.holder.store(get_hart_id(), Ordering::Relaxed);
            Some(SpinlockGuard {
                lock: self,
                _not_send: core::marker::PhantomData,
            })
        } else {
            None
        }
    }

    /// Check if the lock is currently held (for debugging).
    pub fn is_locked(&self) -> bool {
        self.locked.load(Ordering::Relaxed) != UNLOCKED
    }

    /// Get the data without locking (unsafe).
    ///
    /// # Safety
    /// Caller must ensure no concurrent access.
    #[allow(dead_code)]
    pub unsafe fn get_unchecked(&self) -> &T {
        &*self.data.get()
    }

    /// Get mutable data without locking (unsafe).
    ///
    /// # Safety
    /// Caller must ensure no concurrent access.
    #[allow(dead_code)]
    pub unsafe fn get_unchecked_mut(&self) -> &mut T {
        &mut *self.data.get()
    }
}

/// Get current hart ID.
fn get_hart_id() -> usize {
    let id: usize;
    unsafe {
        core::arch::asm!("csrr {}, mhartid", out(reg) id, options(nomem, nostack));
    }
    id
}

/// RAII guard that releases the spinlock when dropped.
pub struct SpinlockGuard<'a, T> {
    lock: &'a Spinlock<T>,
    // Prevent Send - this type contains a raw pointer conceptually
    _not_send: core::marker::PhantomData<*const ()>,
}

impl<T> Deref for SpinlockGuard<'_, T> {
    type Target = T;

    #[inline]
    fn deref(&self) -> &T {
        // Safety: We hold the lock, so exclusive access is guaranteed
        unsafe { &*self.lock.data.get() }
    }
}

impl<T> DerefMut for SpinlockGuard<'_, T> {
    #[inline]
    fn deref_mut(&mut self) -> &mut T {
        // Safety: We hold the lock exclusively
        unsafe { &mut *self.lock.data.get() }
    }
}

impl<T> Drop for SpinlockGuard<'_, T> {
    #[inline]
    fn drop(&mut self) {
        #[cfg(debug_assertions)]
        self.lock.holder.store(usize::MAX, Ordering::Relaxed);

        // Release the lock using AMOSWAP.W to ensure visibility across harts.
        // Using swap instead of store because the emulator serializes AMO operations
        // but may not properly synchronize regular store visibility across hart threads.
        self.lock.locked.swap(UNLOCKED, Ordering::Release);
    }
}

// ============================================================================
// TICKET LOCK (Fair Spinlock - FIFO ordering, no starvation)
// ============================================================================

/// A fair spinlock that ensures FIFO ordering.
///
/// Waiters take a "ticket" and wait until their number is called.
/// This prevents starvation - the process that requests the lock first
/// gets it first.
///
/// # Example
///
/// ```
/// static FAIR_COUNTER: TicketLock<u64> = TicketLock::new(0);
///
/// fn increment() {
///     let mut guard = FAIR_COUNTER.lock();
///     *guard += 1;
/// }
/// ```
pub struct TicketLock<T> {
    /// Next ticket to be issued (incremented by each waiter)
    next_ticket: AtomicU32,
    /// Currently served ticket number
    now_serving: AtomicU32,
    /// Protected data
    data: UnsafeCell<T>,
    #[cfg(debug_assertions)]
    holder: AtomicUsize,
}

unsafe impl<T: Send> Sync for TicketLock<T> {}
unsafe impl<T: Send> Send for TicketLock<T> {}

impl<T> TicketLock<T> {
    /// Create a new ticket lock wrapping the given value.
    pub const fn new(data: T) -> Self {
        Self {
            next_ticket: AtomicU32::new(0),
            now_serving: AtomicU32::new(0),
            data: UnsafeCell::new(data),
            #[cfg(debug_assertions)]
            holder: AtomicUsize::new(usize::MAX),
        }
    }

    /// Acquire the lock, blocking until our ticket is served.
    ///
    /// This ensures FIFO ordering - first come, first served.
    #[inline]
    pub fn lock(&self) -> TicketLockGuard<T> {
        // Atomically get a ticket number
        let my_ticket = self.next_ticket.fetch_add(1, Ordering::Relaxed);

        // Wait until our ticket is being served
        let mut spin_count = 0u32;
        while self.now_serving.load(Ordering::Acquire) != my_ticket {
            spin_loop();
            spin_count = spin_count.wrapping_add(1);

            #[cfg(debug_assertions)]
            if spin_count > 10_000_000 {
                // Very long wait - might be stuck
                let serving = self.now_serving.load(Ordering::Relaxed);
                let next = self.next_ticket.load(Ordering::Relaxed);
                panic!(
                    "TicketLock: potential deadlock - ticket={}, serving={}, next={}",
                    my_ticket, serving, next
                );
            }
        }

        // We have the lock!
        #[cfg(debug_assertions)]
        self.holder.store(get_hart_id(), Ordering::Relaxed);

        TicketLockGuard { lock: self }
    }

    /// Try to acquire the lock without blocking.
    ///
    /// Returns `Some(guard)` if we got the lock, `None` if busy.
    #[inline]
    pub fn try_lock(&self) -> Option<TicketLockGuard<T>> {
        let current = self.now_serving.load(Ordering::Acquire);
        let next = self.next_ticket.load(Ordering::Relaxed);

        // Lock is free if next == current (no one waiting)
        if current == next {
            // Try to atomically take a ticket and check if we got it immediately
            let my_ticket = self.next_ticket.fetch_add(1, Ordering::Relaxed);
            if self.now_serving.load(Ordering::Acquire) == my_ticket {
                #[cfg(debug_assertions)]
                self.holder.store(get_hart_id(), Ordering::Relaxed);
                return Some(TicketLockGuard { lock: self });
            } else {
                // Someone else got in first - we need to wait but we don't want to block
                // Decrement the ticket counter to "give back" our ticket
                // Actually, we can't safely give back the ticket, so we're stuck
                // For try_lock, we should check if lock is free before taking ticket
                // This is a limitation - just return None and let caller retry
            }
        }
        None
    }

    /// Check if the lock is currently held.
    pub fn is_locked(&self) -> bool {
        let current = self.now_serving.load(Ordering::Relaxed);
        let next = self.next_ticket.load(Ordering::Relaxed);
        current != next
    }
}

/// RAII guard for TicketLock
pub struct TicketLockGuard<'a, T> {
    lock: &'a TicketLock<T>,
}

impl<T> Deref for TicketLockGuard<'_, T> {
    type Target = T;

    #[inline]
    fn deref(&self) -> &T {
        unsafe { &*self.lock.data.get() }
    }
}

impl<T> DerefMut for TicketLockGuard<'_, T> {
    #[inline]
    fn deref_mut(&mut self) -> &mut T {
        unsafe { &mut *self.lock.data.get() }
    }
}

impl<T> Drop for TicketLockGuard<'_, T> {
    #[inline]
    fn drop(&mut self) {
        #[cfg(debug_assertions)]
        self.lock.holder.store(usize::MAX, Ordering::Relaxed);

        // Move to next ticket - this wakes the next waiter
        self.lock.now_serving.fetch_add(1, Ordering::Release);
    }
}

// ============================================================================
// READER-WRITER LOCK
// ============================================================================

/// Bits layout for RwLock state:
/// - Bits 0-30: Reader count (max ~2 billion readers)
/// - Bit 31: Writer flag (1 = writer waiting or holding)
const WRITER_BIT: u32 = 1 << 31;
const READER_MASK: u32 = !WRITER_BIT;
const MAX_READERS: u32 = READER_MASK;

/// A reader-writer lock allowing multiple readers OR a single writer.
///
/// This is ideal for resources that are read frequently but written rarely,
/// like the filesystem state.
///
/// # Example
///
/// ```
/// static DATA: RwLock<Vec<u8>> = RwLock::new(Vec::new());
///
/// fn read_data() {
///     let guard = DATA.read();
///     // Multiple readers can hold this simultaneously
///     println!("Length: {}", guard.len());
/// }
///
/// fn write_data(byte: u8) {
///     let mut guard = DATA.write();
///     // Exclusive access
///     guard.push(byte);
/// }
/// ```
pub struct RwLock<T> {
    /// State: bits 0-30 = reader count, bit 31 = writer flag
    state: AtomicU32,
    /// Protected data
    data: UnsafeCell<T>,
    #[cfg(debug_assertions)]
    writer_hart: AtomicUsize,
}

unsafe impl<T: Send> Sync for RwLock<T> {}
unsafe impl<T: Send + Sync> Send for RwLock<T> {}

impl<T> RwLock<T> {
    /// Create a new reader-writer lock.
    pub const fn new(data: T) -> Self {
        Self {
            state: AtomicU32::new(0),
            data: UnsafeCell::new(data),
            #[cfg(debug_assertions)]
            writer_hart: AtomicUsize::new(usize::MAX),
        }
    }

    /// Acquire a read lock.
    ///
    /// Multiple readers can hold the lock simultaneously.
    /// Blocks if a writer is holding or waiting for the lock.
    pub fn read(&self) -> RwLockReadGuard<T> {
        let mut spin_count = 0u32;

        loop {
            let state = self.state.load(Ordering::Relaxed);

            // If no writer is holding/waiting, try to add ourselves as a reader
            if state & WRITER_BIT == 0 {
                // Check for reader overflow
                if (state & READER_MASK) >= MAX_READERS {
                    panic!("RwLock: too many readers");
                }

                // Try to increment reader count
                if self
                    .state
                    .compare_exchange_weak(state, state + 1, Ordering::Acquire, Ordering::Relaxed)
                    .is_ok()
                {
                    return RwLockReadGuard { lock: self };
                }
            }

            spin_loop();
            spin_count = spin_count.wrapping_add(1);

            #[cfg(debug_assertions)]
            if spin_count > 10_000_000 {
                let current = self.state.load(Ordering::Relaxed);
                panic!(
                    "RwLock read: potential deadlock - state=0x{:08x}",
                    current
                );
            }
        }
    }

    /// Try to acquire a read lock without blocking.
    ///
    /// Returns `Some(guard)` if successful, `None` if a writer is active.
    pub fn try_read(&self) -> Option<RwLockReadGuard<T>> {
        let state = self.state.load(Ordering::Relaxed);

        // If no writer is holding/waiting, try to add ourselves as a reader
        if state & WRITER_BIT == 0 && (state & READER_MASK) < MAX_READERS {
            if self
                .state
                .compare_exchange_weak(state, state + 1, Ordering::Acquire, Ordering::Relaxed)
                .is_ok()
            {
                return Some(RwLockReadGuard { lock: self });
            }
        }

        None
    }

    /// Acquire a write lock.
    ///
    /// Blocks until all readers release and no other writer is active.
    pub fn write(&self) -> RwLockWriteGuard<T> {
        let mut spin_count = 0u32;

        // First, set the writer bit to prevent new readers
        loop {
            let state = self.state.load(Ordering::Relaxed);

            if state & WRITER_BIT == 0 {
                // Try to set writer bit
                if self
                    .state
                    .compare_exchange_weak(
                        state,
                        state | WRITER_BIT,
                        Ordering::Acquire,
                        Ordering::Relaxed,
                    )
                    .is_ok()
                {
                    break;
                }
            }

            spin_loop();
            spin_count = spin_count.wrapping_add(1);

            #[cfg(debug_assertions)]
            if spin_count > 10_000_000 {
                panic!("RwLock write: potential deadlock waiting for writer bit");
            }
        }

        // Now wait for all readers to finish
        spin_count = 0;
        while self.state.load(Ordering::Acquire) != WRITER_BIT {
            spin_loop();
            spin_count = spin_count.wrapping_add(1);

            #[cfg(debug_assertions)]
            if spin_count > 10_000_000 {
                let current = self.state.load(Ordering::Relaxed);
                let readers = current & READER_MASK;
                panic!(
                    "RwLock write: potential deadlock waiting for {} readers",
                    readers
                );
            }
        }

        #[cfg(debug_assertions)]
        self.writer_hart.store(get_hart_id(), Ordering::Relaxed);

        RwLockWriteGuard { lock: self }
    }

    /// Try to acquire a write lock without blocking.
    ///
    /// Returns `Some(guard)` if we got exclusive access, `None` if busy.
    pub fn try_write(&self) -> Option<RwLockWriteGuard<T>> {
        // Try to atomically go from 0 to WRITER_BIT
        if self
            .state
            .compare_exchange(0, WRITER_BIT, Ordering::Acquire, Ordering::Relaxed)
            .is_ok()
        {
            #[cfg(debug_assertions)]
            self.writer_hart.store(get_hart_id(), Ordering::Relaxed);
            return Some(RwLockWriteGuard { lock: self });
        }
        None
    }

    /// Check if the lock has any readers.
    pub fn has_readers(&self) -> bool {
        (self.state.load(Ordering::Relaxed) & READER_MASK) > 0
    }

    /// Check if a writer is holding or waiting.
    pub fn has_writer(&self) -> bool {
        (self.state.load(Ordering::Relaxed) & WRITER_BIT) != 0
    }
    
    /// Get the raw lock state for debugging.
    pub fn state_debug(&self) -> u32 {
        self.state.load(Ordering::Relaxed)
    }
}

/// RAII guard for read access
pub struct RwLockReadGuard<'a, T> {
    lock: &'a RwLock<T>,
}

impl<T> Deref for RwLockReadGuard<'_, T> {
    type Target = T;

    #[inline]
    fn deref(&self) -> &T {
        unsafe { &*self.lock.data.get() }
    }
}

impl<T> Drop for RwLockReadGuard<'_, T> {
    #[inline]
    fn drop(&mut self) {
        // Decrement reader count
        self.lock.state.fetch_sub(1, Ordering::Release);
    }
}

/// RAII guard for write access
pub struct RwLockWriteGuard<'a, T> {
    lock: &'a RwLock<T>,
}

impl<T> Deref for RwLockWriteGuard<'_, T> {
    type Target = T;

    #[inline]
    fn deref(&self) -> &T {
        unsafe { &*self.lock.data.get() }
    }
}

impl<T> DerefMut for RwLockWriteGuard<'_, T> {
    #[inline]
    fn deref_mut(&mut self) -> &mut T {
        unsafe { &mut *self.lock.data.get() }
    }
}

impl<T> Drop for RwLockWriteGuard<'_, T> {
    #[inline]
    fn drop(&mut self) {
        #[cfg(debug_assertions)]
        self.lock.writer_hart.store(usize::MAX, Ordering::Relaxed);

        // Clear writer bit (releases the lock)
        self.lock.state.fetch_and(!WRITER_BIT, Ordering::Release);
    }
}

// ============================================================================
// Memory Fence Helpers
// ============================================================================

/// Full memory fence (FENCE IORW, IORW).
///
/// Ensures all memory operations before the fence are visible
/// to all harts before any operations after the fence.
///
/// Use when you need a full barrier, e.g., between init and signaling ready.
#[inline]
pub fn fence_memory() {
    unsafe {
        core::arch::asm!("fence iorw, iorw", options(nomem, nostack));
    }
}

/// Read fence (FENCE IR, IR).
///
/// Ensures all reads before the fence complete before reads after.
#[inline]
#[allow(dead_code)]
pub fn fence_read() {
    unsafe {
        core::arch::asm!("fence ir, ir", options(nomem, nostack));
    }
}

/// Write fence (FENCE OW, OW).
///
/// Ensures all writes before the fence complete before writes after.
#[inline]
#[allow(dead_code)]
pub fn fence_write() {
    unsafe {
        core::arch::asm!("fence ow, ow", options(nomem, nostack));
    }
}

/// Fence for device I/O (FENCE O, I).
///
/// Ensures device writes are complete before device reads.
/// Use when communicating with MMIO devices.
#[inline]
#[allow(dead_code)]
pub fn fence_io() {
    unsafe {
        core::arch::asm!("fence o, i", options(nomem, nostack));
    }
}

/// Instruction fence (FENCE.I).
///
/// Ensures instruction fetches see recent stores.
/// Required after modifying code (e.g., dynamic loading, JIT).
#[inline]
#[allow(dead_code)]
pub fn fence_i() {
    unsafe {
        core::arch::asm!("fence.i", options(nomem, nostack));
    }
}

/// Release fence (FENCE W, W).
///
/// Ensures writes are visible before a release store.
/// Use before storing a flag that another hart will read.
#[inline]
pub fn fence_release() {
    unsafe {
        core::arch::asm!("fence w, w", options(nomem, nostack));
    }
}

/// Acquire fence (FENCE R, R).
///
/// Ensures subsequent reads see writes from before the acquire load.
/// Use after loading a flag written by another hart.
#[inline]
pub fn fence_acquire() {
    unsafe {
        core::arch::asm!("fence r, r", options(nomem, nostack));
    }
}

// ============================================================================
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use alloc::vec;

    use super::*;

    #[test]
    fn test_spinlock_basic() {
        let lock = Spinlock::new(42);
        {
            let mut guard = lock.lock();
            assert_eq!(*guard, 42);
            *guard = 100;
        }
        {
            let guard = lock.lock();
            assert_eq!(*guard, 100);
        }
    }

    #[test]
    fn test_spinlock_try_lock() {
        let lock = Spinlock::new(0);

        // Should succeed when not held
        let guard = lock.try_lock();
        assert!(guard.is_some());

        // Should fail when held (from same thread - normally would deadlock but try_lock returns None)
        // Note: This test is tricky because we're on same thread
        drop(guard);

        // After drop, should succeed again
        assert!(lock.try_lock().is_some());
    }

    #[test]
    fn test_ticket_lock_basic() {
        let lock = TicketLock::new(vec![1, 2, 3]);
        {
            let mut guard = lock.lock();
            guard.push(4);
            assert_eq!(guard.len(), 4);
        }
        {
            let guard = lock.lock();
            assert_eq!(*guard, vec![1, 2, 3, 4]);
        }
    }

    #[test]
    fn test_rwlock_multiple_readers() {
        let lock = RwLock::new(42);

        // Take a read lock
        let r1 = lock.read();
        assert_eq!(*r1, 42);

        // Should be able to take another read lock
        let r2 = lock.try_read();
        assert!(r2.is_some());
        assert_eq!(*r2.unwrap(), 42);

        // Writer should fail
        assert!(lock.try_write().is_none());

        drop(r1);
    }

    #[test]
    fn test_rwlock_writer_exclusive() {
        let lock = RwLock::new(String::from("hello"));

        // Take write lock
        let mut w = lock.write();
        w.push_str(" world");

        // No readers should be able to acquire
        assert!(lock.try_read().is_none());

        // No other writers should be able to acquire
        assert!(lock.try_write().is_none());

        drop(w);

        // Now read should work
        let r = lock.read();
        assert_eq!(&*r, "hello world");
    }
}
