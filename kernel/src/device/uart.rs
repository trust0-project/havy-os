use core::fmt::{self, Write};
use core::sync::atomic::{AtomicU32, Ordering};

use alloc::vec::Vec;

use crate::lock::utils::{BLK_DEV, FS_STATE, OUTPUT_CAPTURE};
use crate::scripting::execute_command;
use crate::utils::{poll_tail_follow, resolve_path};
use crate::services::{klogd, sysmond};

const UART_BASE: usize = 0x1000_0000;

// ============================================================================
// UART SPINLOCK - Prevents interleaved output from multiple harts
// ============================================================================

const UART_UNLOCKED: u32 = 0;
const UART_LOCKED: u32 = 1;

/// Global lock for UART output serialization across harts.
/// Uses a simple atomic spinlock to prevent interleaved log messages.
static UART_LOCK: AtomicU32 = AtomicU32::new(UART_UNLOCKED);

/// Acquire the UART lock (blocking).
#[inline]
fn uart_lock_acquire() {
    loop {
        if UART_LOCK.swap(UART_LOCKED, Ordering::Acquire) == UART_UNLOCKED {
            return;
        }
        core::hint::spin_loop();
    }
}

/// Release the UART lock.
#[inline]
fn uart_lock_release() {
    UART_LOCK.swap(UART_UNLOCKED, Ordering::Release);
}

// NS16550A UART register offsets
const RBR: usize = 0x00; // Receiver Buffer Register (read)
const THR: usize = 0x00; // Transmitter Holding Register (write)
const IER: usize = 0x01; // Interrupt Enable Register
const FCR: usize = 0x02; // FIFO Control Register (write)
const LCR: usize = 0x03; // Line Control Register
const MCR: usize = 0x04; // Modem Control Register
const LSR: usize = 0x05; // Line Status Register

// LSR bits
const LSR_RX_READY: u8 = 0x01; // Data ready
const LSR_TX_IDLE: u8 = 0x20; // THR empty (Transmitter Holding Register Empty)

#[derive(Clone, Copy, PartialEq)]
enum RedirectMode {
    None,
    Overwrite, // >
    Append,    // >>
}


/// Start capturing output to the buffer
fn output_capture_start() {
    let mut cap = OUTPUT_CAPTURE.lock();
    cap.capturing = true;
    cap.len = 0;
}

/// Stop capturing and return the captured bytes as a Vec
fn output_capture_stop() -> Vec<u8> {
    let mut cap = OUTPUT_CAPTURE.lock();
    cap.capturing = false;
    Vec::from(&cap.buffer[..cap.len])
}


pub struct Console;

impl Console {
    pub const fn new() -> Self {
        Self
    }

    /// Initialize the UART for QEMU virt machine compatibility.
    /// Uses minimal configuration - QEMU's NS16550 works well with simple setup.
    #[allow(dead_code)]
    pub fn init() {
        unsafe {
            let base = UART_BASE as *mut u8;

            // Disable all interrupts
            core::ptr::write_volatile(base.add(IER), 0x00);

            // 8 bits, no parity, one stop bit (8N1)
            // QEMU doesn't require baud rate configuration
            core::ptr::write_volatile(base.add(LCR), 0x03);

            // Disable FIFO for simple character-by-character operation
            // Writing 0 to FCR disables FIFO mode
            core::ptr::write_volatile(base.add(FCR), 0x00);
        }
    }

    /// Read a byte, blocking until one is available.
    /// Use this for guaranteed input reception.
    /// While waiting, periodically runs background tasks on hart 0.
    pub fn read_byte_blocking(&self) -> u8 {
        let mut poll_counter: u32 = 0;
        // Spin until data is ready
        while !Self::is_rx_ready() {
            core::hint::spin_loop();
            
            // Every ~1000 iterations, run background tasks
            poll_counter = poll_counter.wrapping_add(1);
            if poll_counter % 1000 == 0 {
                // Run hart0 background tasks (klogd, sysmond)
                klogd::klogd_tick();
                sysmond::sysmond_tick();
                
                // Poll tail -f for new content
                poll_tail_follow();
            }
        }
        unsafe { core::ptr::read_volatile((UART_BASE + RBR) as *const u8) }
    }

    #[inline(always)]
    fn lsr() -> u8 {
        unsafe { core::ptr::read_volatile((UART_BASE + LSR) as *const u8) }
    }

    #[inline(always)]
    fn wait_for_tx_ready() {
        // Wait until THR is empty (LSR bit 5)
        while (Self::lsr() & LSR_TX_IDLE) == 0 {
            core::hint::spin_loop();
        }
    }

    #[inline(always)]
    fn is_rx_ready() -> bool {
        (Self::lsr() & LSR_RX_READY) != 0
    }

    /// Public version of is_rx_ready for external use
    pub fn is_rx_ready_public() -> bool {
        Self::is_rx_ready()
    }

    pub fn write_byte(&mut self, byte: u8) {
        Self::wait_for_tx_ready();
        unsafe {
            core::ptr::write_volatile((UART_BASE + THR) as *mut u8, byte);
        }
    }

    pub fn read_byte(&self) -> u8 {
        // Only return a byte if data is ready, otherwise return 0
        if Self::is_rx_ready() {
            unsafe { core::ptr::read_volatile((UART_BASE + RBR) as *const u8) }
        } else {
            0
        }
    }
}

impl Write for Console {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        for byte in s.bytes() {
            self.write_byte(byte);
        }
        Ok(())
    }
}

/// Write a raw string to the UART without using `core::fmt`.
/// Protected by UART_LOCK to prevent interleaved output from multiple harts.
pub fn write_str(s: &str) {
    uart_lock_acquire();
    let mut console = Console::new();
    let _ = console.write_str(s);
    uart_lock_release();
}

/// Write a raw string followed by `\n`.
/// Protected by UART_LOCK.
pub fn write_line(s: &str) {
    uart_lock_acquire();
    let mut console = Console::new();
    let _ = console.write_str(s);
    let _ = console.write_str("\n");
    uart_lock_release();
}

/// Write a raw byte slice to the UART.
pub fn write_bytes(bytes: &[u8]) {
    let mut console = Console::new();
    for &b in bytes {
        console.write_byte(b);
    }
}

/// Write a single byte to the UART.
pub fn write_byte(byte: u8) {
    Console::new().write_byte(byte);
}

/// Write an unsigned integer in decimal.
pub fn write_u64(mut n: u64) {
    let mut console = Console::new();

    if n == 0 {
        console.write_byte(b'0');
        return;
    }

    let mut buf = [0u8; 20]; // enough for u64
    let mut i = 0;

    while n > 0 && i < buf.len() {
        let digit = (n % 10) as u8;
        buf[i] = b'0' + digit;
        n /= 10;
        i += 1;
    }

    while i > 0 {
        i -= 1;
        console.write_byte(buf[i]);
    }
}

/// Write an unsigned integer in hexadecimal.
pub fn write_hex(mut n: u64) {
    let mut console = Console::new();
    let hex_digits = b"0123456789abcdef";

    if n == 0 {
        console.write_byte(b'0');
        return;
    }

    let mut buf = [0u8; 16]; // enough for u64 hex
    let mut i = 0;

    while n > 0 && i < buf.len() {
        buf[i] = hex_digits[(n & 0xf) as usize];
        n >>= 4;
        i += 1;
    }

    while i > 0 {
        i -= 1;
        console.write_byte(buf[i]);
    }
}

/// Write a single byte in hexadecimal (2 characters).
pub fn write_hex_byte(b: u8) {
    let mut console = Console::new();
    let hex_digits = b"0123456789abcdef";
    console.write_byte(hex_digits[(b >> 4) as usize]);
    console.write_byte(hex_digits[(b & 0xf) as usize]);
}

/// Check if console has pending input
pub fn has_pending_input() -> bool {
    Console::is_rx_ready_public()
}

/// Read a character from console (non-blocking)
/// Returns None if no character is available
pub fn read_char_nonblocking() -> Option<u8> {
    if Console::is_rx_ready_public() {
        Some(Console::new().read_byte())
    } else {
        None
    }
}

/// Format and print to UART using core::fmt::Arguments
/// Protected by UART_LOCK to prevent interleaved output from multiple harts.
pub fn print_fmt(args: fmt::Arguments) {
    uart_lock_acquire();
    let mut console = Console::new();
    let _ = core::fmt::write(&mut console, args);
    uart_lock_release();
}

#[macro_export]
macro_rules! print {
    ($($arg:tt)*) => ({
        $crate::uart::print_fmt(core::format_args!($($arg)*));
    });
}

#[macro_export]
macro_rules! println {
    () => ($crate::print!("\n"));
    ($fmt:expr $(, $($arg:tt)*)?) => ({
        $crate::uart::print_fmt(core::format_args!(concat!($fmt, "\n") $(, $($arg)*)?));
    });
}



/// Trim whitespace from byte slice
fn trim_bytes(bytes: &[u8]) -> &[u8] {
    let mut start = 0;
    let mut end = bytes.len();

    while start < end && (bytes[start] == b' ' || bytes[start] == b'\t') {
        start += 1;
    }
    while end > start && (bytes[end - 1] == b' ' || bytes[end - 1] == b'\t') {
        end -= 1;
    }

    &bytes[start..end]
}





/// Parse a command line for redirection operators
/// Returns: (command_part, redirect_mode, filename)
fn parse_redirection(line: &[u8]) -> (&[u8], RedirectMode, &[u8]) {
    // Look for >> first (must check before >)
    for i in 0..line.len().saturating_sub(1) {
        if line[i] == b'>' && line[i + 1] == b'>' {
            let cmd_part = trim_bytes(&line[..i]);
            let file_part = trim_bytes(&line[i + 2..]);
            return (cmd_part, RedirectMode::Append, file_part);
        }
    }

    // Look for single >
    for i in 0..line.len() {
        if line[i] == b'>' {
            let cmd_part = trim_bytes(&line[..i]);
            let file_part = trim_bytes(&line[i + 1..]);
            return (cmd_part, RedirectMode::Overwrite, file_part);
        }
    }

    (line, RedirectMode::None, &[])
}


pub fn handle_line(buffer: &[u8], len: usize, _count: &mut usize) {
    // Trim leading/trailing whitespace (spaces and tabs only)
    let mut start = 0;
    let mut end = len;

    while start < end && (buffer[start] == b' ' || buffer[start] == b'\t') {
        start += 1;
    }
    while end > start && (buffer[end - 1] == b' ' || buffer[end - 1] == b'\t') {
        end -= 1;
    }

    if start >= end {
        // Empty line -> do nothing
        return;
    }

    let full_line = &buffer[start..end];

    // Parse for redirection
    let (line, redirect_mode, redirect_file) = parse_redirection(full_line);

    // Validate redirection target
    if redirect_mode != RedirectMode::None && redirect_file.is_empty() {
        write_line("");
        write_line("\x1b[1;31mError:\x1b[0m Missing filename for redirection");
        return;
    }

    // Split into command and arguments (first whitespace)
    let mut i = 0;
    while i < line.len() && line[i] != b' ' && line[i] != b'\t' {
        i += 1;
    }
    let cmd = &line[..i];

    let mut arg_start = i;
    while arg_start < line.len() && (line[arg_start] == b' ' || line[arg_start] == b'\t') {
        arg_start += 1;
    }
    let args = &line[arg_start..];

    // Start capturing if redirecting
    if redirect_mode != RedirectMode::None {
        output_capture_start();
    }

    // Execute the command
    execute_command(cmd, args);

    // Handle redirection output
    if redirect_mode != RedirectMode::None {
        let output = output_capture_stop();

        if let Ok(filename) = core::str::from_utf8(redirect_file) {
            let filename = filename.trim();
            // Resolve path relative to CWD
            let resolved_path = resolve_path(filename);

            let mut fs_guard = FS_STATE.write();
            let mut blk_guard = BLK_DEV.write();
            if let (Some(fs), Some(dev)) = (fs_guard.as_mut(), blk_guard.as_mut()) {
                let final_data = if redirect_mode == RedirectMode::Append {
                    // Read existing file content and append
                    let mut combined = match fs.read_file(dev, &resolved_path) {
                        Some(existing) => existing,
                        None => Vec::new(),
                    };
                    combined.extend_from_slice(&output);
                    combined
                } else {
                    // Overwrite mode - just use new output
                    output
                };

                match fs.write_file(dev, &resolved_path, &final_data) {
                    Ok(()) => {
                        // Sync to ensure data is written to disk
                        let _ = fs.sync(dev);
                        write_line("");
                        write_str("\x1b[1;32m[OK]\x1b[0m Output written to ");
                        write_line(&resolved_path);
                    }
                    Err(e) => {
                        write_line("");
                        write_str("\x1b[1;31mError:\x1b[0m Failed to write to file: ");
                        write_line(e);
                    }
                }
            } else {
                write_line("");
                write_line("\x1b[1;31mError:\x1b[0m Filesystem not available");
            }
        } else {
            write_line("");
            write_line("\x1b[1;31mError:\x1b[0m Invalid filename");
        }
    }
}
