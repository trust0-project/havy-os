//! Portable HID events (Linux evdev numbers).
//!
//! Virt delivers these on virtio-input. D1 uses TWI+FT6336U with an
//! emulator MMIO fallback. UI code talks to this module, not a platform
//! MMIO address.

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct InputEvent {
    pub event_type: u16,
    pub code: u16,
    pub value: i32,
}

impl InputEvent {
    pub fn is_key_press(&self) -> bool {
        self.event_type == EV_KEY && self.value == 1
    }

    pub fn is_key_release(&self) -> bool {
        self.event_type == EV_KEY && self.value == 0
    }
}

pub const EV_SYN: u16 = 0x00;
pub const EV_KEY: u16 = 0x01;
pub const EV_ABS: u16 = 0x03;
/// Typed UTF-8/ASCII byte from the host (browser layout). Carried on the
/// virtio-input queue; not an I2C register.
pub const EV_CHAR: u16 = 0x10;

pub const ABS_X: u16 = 0x00;
pub const ABS_Y: u16 = 0x01;

pub const BTN_LEFT: u16 = 0x110;
pub const BTN_RIGHT: u16 = 0x111;
pub const BTN_MIDDLE: u16 = 0x112;
pub const BTN_TOUCH: u16 = 0x14A;

pub const KEY_ESC: u16 = 1;
pub const KEY_1: u16 = 2;
pub const KEY_2: u16 = 3;
pub const KEY_3: u16 = 4;
pub const KEY_4: u16 = 5;
pub const KEY_5: u16 = 6;
pub const KEY_6: u16 = 7;
pub const KEY_7: u16 = 8;
pub const KEY_8: u16 = 9;
pub const KEY_9: u16 = 10;
pub const KEY_0: u16 = 11;
pub const KEY_BACKSPACE: u16 = 14;
pub const KEY_TAB: u16 = 15;
pub const KEY_Q: u16 = 16;
pub const KEY_W: u16 = 17;
pub const KEY_E: u16 = 18;
pub const KEY_R: u16 = 19;
pub const KEY_T: u16 = 20;
pub const KEY_Y: u16 = 21;
pub const KEY_U: u16 = 22;
pub const KEY_I: u16 = 23;
pub const KEY_O: u16 = 24;
pub const KEY_P: u16 = 25;
pub const KEY_ENTER: u16 = 28;
pub const KEY_A: u16 = 30;
pub const KEY_S: u16 = 31;
pub const KEY_D: u16 = 32;
pub const KEY_F: u16 = 33;
pub const KEY_G: u16 = 34;
pub const KEY_H: u16 = 35;
pub const KEY_J: u16 = 36;
pub const KEY_K: u16 = 37;
pub const KEY_L: u16 = 38;
pub const KEY_Z: u16 = 44;
pub const KEY_X: u16 = 45;
pub const KEY_C: u16 = 46;
pub const KEY_V: u16 = 47;
pub const KEY_B: u16 = 48;
pub const KEY_N: u16 = 49;
pub const KEY_M: u16 = 50;
pub const KEY_SPACE: u16 = 57;
pub const KEY_UP: u16 = 103;
pub const KEY_LEFT: u16 = 105;
pub const KEY_RIGHT: u16 = 106;
pub const KEY_DOWN: u16 = 108;

/// Poll the platform HID source into its event queue.
pub fn poll() {
    #[cfg(feature = "d1")]
    crate::platform::d1_touch::poll();
    #[cfg(not(feature = "d1"))]
    crate::virtio_input::poll();
}

/// Pop the next queued event.
pub fn next_event() -> Option<InputEvent> {
    #[cfg(feature = "d1")]
    {
        crate::platform::d1_touch::next_event()
    }
    #[cfg(not(feature = "d1"))]
    {
        crate::virtio_input::next_event()
    }
}

pub fn has_events() -> bool {
    #[cfg(feature = "d1")]
    {
        crate::platform::d1_touch::has_events()
    }
    #[cfg(not(feature = "d1"))]
    {
        crate::virtio_input::has_events()
    }
}
