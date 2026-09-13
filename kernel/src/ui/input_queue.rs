//! Bounded UI work queue: non-owners enqueue, `gpuid` drains on hart 0.
//!
//! Scene state (`MAIN_SCREEN_*`, compositor, drag, terminal buffers) is
//! mutated only by the owner. WASM and other non-gpuid callers post work
//! here instead of touching `static mut` UI.

use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use crate::input::{self, InputEvent, EV_CHAR};
use crate::Spinlock;

/// Capacity of the input ring. Overflow drops coalescable moves first.
const CAP: usize = 64;

struct InputRing {
    buf: [InputEvent; CAP],
    head: usize,
    len: usize,
}

impl InputRing {
    const fn new() -> Self {
        Self {
            buf: [InputEvent {
                event_type: 0,
                code: 0,
                value: 0,
            }; CAP],
            head: 0,
            len: 0,
        }
    }

    fn push(&mut self, event: InputEvent) -> bool {
        if self.len == CAP {
            if event.event_type == input::EV_ABS {
                return false;
            }
            // Keep clicks/keys: drop the oldest event (usually a move).
            self.head = (self.head + 1) % CAP;
            self.len -= 1;
        }
        let i = (self.head + self.len) % CAP;
        self.buf[i] = event;
        self.len += 1;
        true
    }

    fn pop(&mut self) -> Option<InputEvent> {
        if self.len == 0 {
            return None;
        }
        let event = self.buf[self.head];
        self.head = (self.head + 1) % CAP;
        self.len -= 1;
        Some(event)
    }

    fn len(&self) -> usize {
        self.len
    }
}

static RING: Spinlock<InputRing> = Spinlock::new(InputRing::new());
static RING_LEN: AtomicUsize = AtomicUsize::new(0);

/// Terminal/live-output redraw requested. Coalesced.
static REFRESH: AtomicBool = AtomicBool::new(false);
/// gpuid should call `main_screen::request_cancel` (draw ^C, etc.).
static CANCEL_APPLY: AtomicBool = AtomicBool::new(false);
/// WASM/guest-visible cancel flag (does not touch MAIN_SCREEN).
static CANCEL_FLAG: AtomicBool = AtomicBool::new(false);
/// True while `wasm::execute` is on the stack, so gpuid can mirror
/// MAIN_SCREEN cancel into `CANCEL_FLAG` without stealing from native ELF.
static GUEST_UI_CLIENT: AtomicBool = AtomicBool::new(false);

/// Hart 0 is the UI owner (`gpuid` is pinned there).
#[inline]
pub fn is_owner() -> bool {
    crate::get_hart_id() == 0
}

#[inline]
pub fn has_pending() -> bool {
    REFRESH.load(Ordering::Acquire)
        || CANCEL_APPLY.load(Ordering::Acquire)
        || RING_LEN.load(Ordering::Acquire) > 0
}

/// Mark that a WASM (or similar) guest is the cancel consumer.
pub fn set_guest_ui_client(active: bool) {
    GUEST_UI_CLIENT.store(active, Ordering::Release);
}

#[inline]
pub fn guest_ui_client() -> bool {
    GUEST_UI_CLIENT.load(Ordering::Acquire)
}

/// Set the guest-visible cancel flag without applying UI (owner does that).
pub fn publish_cancel() {
    CANCEL_FLAG.store(true, Ordering::Release);
}

/// Consume the guest-visible cancel flag. Safe from any hart.
pub fn take_cancel() -> bool {
    CANCEL_FLAG.swap(false, Ordering::AcqRel)
}

/// Enqueue a HID event for `gpuid`. Cancel keys also set the guest flag.
pub fn enqueue_input(event: InputEvent) -> bool {
    if is_cancel_event(event) {
        enqueue_cancel();
    }
    let pushed = {
        let mut ring = RING.lock();
        let ok = ring.push(event);
        RING_LEN.store(ring.len(), Ordering::Release);
        ok
    };
    if pushed {
        wake_owner();
    }
    pushed
}

/// Ask `gpuid` to copy live terminal output. Coalesced.
pub fn enqueue_refresh() {
    REFRESH.store(true, Ordering::Release);
    wake_owner();
}

/// Guest cancel (`q` / ESC / Ctrl+C). Sets the flag immediately; gpuid applies UI.
pub fn enqueue_cancel() {
    CANCEL_FLAG.store(true, Ordering::Release);
    CANCEL_APPLY.store(true, Ordering::Release);
    wake_owner();
}

/// Pop one queued input event. No-op if the caller is not the owner hart.
pub fn pop_input() -> Option<InputEvent> {
    if !is_owner() {
        return None;
    }
    let mut ring = RING.lock();
    let event = ring.pop();
    RING_LEN.store(ring.len(), Ordering::Release);
    event
}

/// Take a coalesced terminal-refresh request. Owner only.
pub fn take_refresh() -> bool {
    if !is_owner() {
        return false;
    }
    REFRESH.swap(false, Ordering::AcqRel)
}

/// Take a coalesced "apply request_cancel" request. Owner only.
pub fn take_cancel_apply() -> bool {
    if !is_owner() {
        return false;
    }
    CANCEL_APPLY.swap(false, Ordering::AcqRel)
}

fn is_cancel_event(event: InputEvent) -> bool {
    // Match the WASM host-function special case: 'q' is cancel, but
    // handle_main_screen_input treats it as a typed character. ESC/Ctrl+C
    // are applied only when gpuid drains the event (avoids double ^C).
    event.event_type == EV_CHAR && (event.code == b'q' as u16 || event.code == b'Q' as u16)
}

fn wake_owner() {
    crate::services::gpuid::signal_input_ready();
    if crate::get_hart_id() != 0 {
        crate::cpu::send_ipi(0);
    }
}
