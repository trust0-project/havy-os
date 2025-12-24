//! Wait queue state for blocking tasks until events occur
//!
//! Provides Linux-like wait queue semantics for process synchronization.

use crate::Spinlock;
use alloc::collections::VecDeque;
use alloc::string::String;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicUsize, Ordering};

/// Process identifier type
pub type Pid = u32;

/// Events that a task can wait on
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum WaitEvent {
    /// Timer expired
    Timer = 0,
    /// I/O available (network, block device)
    IoReady = 1,
    /// IPC message available
    IpcMessage = 2,
    /// Child process exited
    ChildExit = 3,
    /// Generic signal
    Signal = 4,
}

/// A waiter entry in a wait queue
#[derive(Clone)]
pub struct Waiter {
    /// PID of the waiting task
    pub pid: Pid,
    /// Event being waited on
    pub event: WaitEvent,
    /// Optional timeout (absolute timestamp in ms)
    pub timeout: Option<u64>,
    /// Data associated with the wait (e.g., channel ID for IPC)
    pub data: u64,
}

/// A wait queue for blocking tasks until an event occurs
pub struct WaitQueueState {
    /// Name for debugging
    name: String,
    /// Waiters in FIFO order
    waiters: Spinlock<VecDeque<Waiter>>,
    /// Number of wake signals pending
    pending_wakes: AtomicUsize,
}

impl WaitQueueState {
    /// Create a new wait queue
    pub fn new(name: &str) -> Self {
        Self {
            name: String::from(name),
            waiters: Spinlock::new(VecDeque::new()),
            pending_wakes: AtomicUsize::new(0),
        }
    }

    /// Add a task to the wait queue
    pub fn wait(&self, pid: Pid, event: WaitEvent, timeout: Option<u64>, data: u64) {
        let waiter = Waiter {
            pid,
            event,
            timeout,
            data,
        };
        self.waiters.lock().push_back(waiter);
        crate::services::klogd::klog_trace(
            "waitq",
            &alloc::format!("Task {} waiting on {:?} (queue={})", pid, event, self.name),
        );
    }

    /// Wake one waiter (FIFO order)
    /// Returns the PID of the woken task, if any
    pub fn wake_one(&self) -> Option<Pid> {
        let waiter = self.waiters.lock().pop_front()?;
        crate::services::klogd::klog_trace(
            "waitq",
            &alloc::format!(
                "Waking task {} from {:?} (queue={})",
                waiter.pid,
                waiter.event,
                self.name
            ),
        );

        // Mark the process as ready (using new process system)
        if let Some(process) = crate::cpu::process::PROCESS_TABLE.get(waiter.pid) {
            process.mark_ready();
        }

        Some(waiter.pid)
    }

    /// Wake all waiters
    /// Returns the number of tasks woken
    pub fn wake_all(&self) -> usize {
        let mut count = 0;
        let mut waiters = self.waiters.lock();
        while let Some(waiter) = waiters.pop_front() {
            crate::services::klogd::klog_trace(
                "waitq",
                &alloc::format!(
                    "Waking task {} from {:?} (queue={})",
                    waiter.pid,
                    waiter.event,
                    self.name
                ),
            );
            if let Some(process) = crate::cpu::process::PROCESS_TABLE.get(waiter.pid) {
                process.mark_ready();
            }
            count += 1;
        }
        count
    }

    /// Wake waiters matching a specific event type
    pub fn wake_event(&self, event: WaitEvent) -> usize {
        let mut count = 0;
        let mut waiters = self.waiters.lock();
        let mut remaining = VecDeque::new();

        while let Some(waiter) = waiters.pop_front() {
            if waiter.event == event {
                crate::services::klogd::klog_trace(
                    "waitq",
                    &alloc::format!(
                        "Waking task {} from {:?} (queue={})",
                        waiter.pid,
                        waiter.event,
                        self.name
                    ),
                );
                if let Some(process) = crate::cpu::process::PROCESS_TABLE.get(waiter.pid) {
                    process.mark_ready();
                }
                count += 1;
            } else {
                remaining.push_back(waiter);
            }
        }

        *waiters = remaining;
        count
    }

    /// Check for and remove timed-out waiters
    /// Returns PIDs of timed-out tasks
    pub fn check_timeouts(&self, current_time: u64) -> Vec<Pid> {
        let mut timed_out = Vec::new();
        let mut waiters = self.waiters.lock();
        let mut remaining = VecDeque::new();

        while let Some(waiter) = waiters.pop_front() {
            if let Some(timeout) = waiter.timeout {
                if current_time >= timeout {
                    crate::services::klogd::klog_trace(
                        "waitq",
                        &alloc::format!(
                            "Task {} timed out on {:?} (queue={})",
                            waiter.pid,
                            waiter.event,
                            self.name
                        ),
                    );
                    if let Some(process) = crate::cpu::process::PROCESS_TABLE.get(waiter.pid) {
                        process.mark_ready();
                    }
                    timed_out.push(waiter.pid);
                    continue;
                }
            }
            remaining.push_back(waiter);
        }

        *waiters = remaining;
        timed_out
    }

    /// Get number of waiters
    pub fn len(&self) -> usize {
        self.waiters.lock().len()
    }

    /// Check if queue is empty
    pub fn is_empty(&self) -> bool {
        self.waiters.lock().is_empty()
    }

    /// Check if a specific PID is waiting
    pub fn is_waiting(&self, pid: Pid) -> bool {
        self.waiters.lock().iter().any(|w| w.pid == pid)
    }
}

// Type alias for backwards compatibility
pub type WaitQueue = WaitQueueState;



