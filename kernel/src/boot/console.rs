//! Unified Boot Console
//!
//! Provides a common interface for boot output that can render to both
//! UART (serial console) and GPU (graphical framebuffer) simultaneously.
//!
//! This ensures consistent boot messages across all output devices.

use core::sync::atomic::{AtomicBool, Ordering};

use crate::device::uart;

/// Whether the GPU console is available (set after GPU init)
static GPU_AVAILABLE: AtomicBool = AtomicBool::new(false);

/// Boot output message types
#[derive(Clone, Copy)]
pub enum BootMsgType {
    /// Plain text line
    Line,
    /// Section header (major boot phase)
    Section,
    /// Status message with OK/FAIL indicator
    Status { ok: bool },
    /// Info line with key-value pair
    Info,
}

/// Boot output trait - implemented by different console backends
pub trait BootOutput {
    /// Print a plain line of text
    fn print_line(&self, text: &str);
    
    /// Print a section header (major boot phase)
    fn print_section(&self, title: &str);
    
    /// Print a status message with OK/FAIL indicator
    fn print_status(&self, component: &str, ok: bool);
    
    /// Print an info line with key-value pair
    fn print_info(&self, key: &str, value: &str);
    
    /// Print a blank line
    fn print_blank(&self) {
        self.print_line("");
    }
}

// ============================================================================
// UART Console Implementation
// ============================================================================

/// UART console backend - outputs to serial with ANSI color codes
pub struct UartConsole;

impl UartConsole {
    pub const fn new() -> Self {
        Self
    }
}

impl BootOutput for UartConsole {
    fn print_line(&self, text: &str) {
        uart::write_line(text);
    }
    
    fn print_section(&self, title: &str) {
        uart::write_line("");
        uart::write_line(
            "\x1b[1;33m------------------------------------------------------------------------\x1b[0m",
        );
        uart::write_str("\x1b[1;33m  * ");
        uart::write_str(title);
        uart::write_line("\x1b[0m");
        uart::write_line(
            "\x1b[1;33m------------------------------------------------------------------------\x1b[0m",
        );
    }
    
    fn print_status(&self, component: &str, ok: bool) {
        if ok {
            uart::write_str("    \x1b[1;32m[OK]\x1b[0m ");
        } else {
            uart::write_str("    \x1b[1;31m[X]\x1b[0m ");
        }
        uart::write_line(component);
    }
    
    fn print_info(&self, key: &str, value: &str) {
        uart::write_str("    \x1b[0;90m+-\x1b[0m ");
        uart::write_str(key);
        uart::write_str(": \x1b[1;97m");
        uart::write_str(value);
        uart::write_line("\x1b[0m");
    }
}

// ============================================================================
// GPU Console Implementation
// ============================================================================

/// GPU console backend - outputs to framebuffer via boot console
pub struct GpuConsole;

impl GpuConsole {
    pub const fn new() -> Self {
        Self
    }
    
    /// Check if GPU console is available
    pub fn is_available() -> bool {
        GPU_AVAILABLE.load(Ordering::Acquire)
    }
    
    /// Mark GPU console as available (called after GPU init)
    pub fn set_available(available: bool) {
        GPU_AVAILABLE.store(available, Ordering::Release);
    }
}

impl BootOutput for GpuConsole {
    fn print_line(&self, text: &str) {
        if Self::is_available() {
            crate::ui::boot::print_line(text);
        }
    }
    
    fn print_section(&self, title: &str) {
        if Self::is_available() {
            // Batch the 4 lines to avoid 4 separate flushes
            crate::ui::boot::batch_begin();
            crate::ui::boot::print_line("");
            crate::ui::boot::print_line("========================================");
            crate::ui::boot::print_boot_msg("", title);
            crate::ui::boot::print_line("========================================");
            crate::ui::boot::batch_end();
        }
    }
    
    fn print_status(&self, component: &str, ok: bool) {
        if Self::is_available() {
            let prefix = if ok { "OK" } else { "FAIL" };
            crate::ui::boot::print_boot_msg(prefix, component);
        }
    }
    
    fn print_info(&self, key: &str, value: &str) {
        if Self::is_available() {
            // Format as "key: value" for GPU
            let mut buf = [0u8; 128];
            let mut pos = 0;
            
            // Copy key
            let key_bytes = key.as_bytes();
            let key_len = key_bytes.len().min(60);
            buf[pos..pos + key_len].copy_from_slice(&key_bytes[..key_len]);
            pos += key_len;
            
            // Add separator
            buf[pos] = b':';
            pos += 1;
            buf[pos] = b' ';
            pos += 1;
            
            // Copy value
            let val_bytes = value.as_bytes();
            let val_len = val_bytes.len().min(128 - pos);
            buf[pos..pos + val_len].copy_from_slice(&val_bytes[..val_len]);
            pos += val_len;
            
            if let Ok(line) = core::str::from_utf8(&buf[..pos]) {
                crate::ui::boot::print_boot_msg("INFO", line);
            }
        }
    }
}

// ============================================================================
// Unified Console Implementation
// ============================================================================

/// Unified console that outputs to both UART and GPU
pub struct UnifiedConsole {
    uart: UartConsole,
    gpu: GpuConsole,
}

impl UnifiedConsole {
    pub const fn new() -> Self {
        Self {
            uart: UartConsole::new(),
            gpu: GpuConsole::new(),
        }
    }
}

impl BootOutput for UnifiedConsole {
    fn print_line(&self, text: &str) {
        self.uart.print_line(text);
        self.gpu.print_line(text);
    }
    
    fn print_section(&self, title: &str) {
        self.uart.print_section(title);
        self.gpu.print_section(title);
    }
    
    fn print_status(&self, component: &str, ok: bool) {
        self.uart.print_status(component, ok);
        self.gpu.print_status(component, ok);
    }
    
    fn print_info(&self, key: &str, value: &str) {
        self.uart.print_info(key, value);
        self.gpu.print_info(key, value);
    }
}

// ============================================================================
// Global Boot Console
// ============================================================================

/// Global unified boot console instance
static BOOT_CONSOLE: UnifiedConsole = UnifiedConsole::new();

/// Get reference to the boot console
pub fn console() -> &'static UnifiedConsole {
    &BOOT_CONSOLE
}

// ============================================================================
// Convenience Functions
// ============================================================================

/// Print a plain line to boot console (both UART and GPU)
pub fn print_line(text: &str) {
    BOOT_CONSOLE.print_line(text);
}

/// Print a section header (major boot phase)
pub fn print_section(title: &str) {
    BOOT_CONSOLE.print_section(title);
}

/// Print a status message with OK/FAIL indicator
pub fn print_status(component: &str, ok: bool) {
    BOOT_CONSOLE.print_status(component, ok);
}

/// Print an info line with key-value pair
pub fn print_info(key: &str, value: &str) {
    BOOT_CONSOLE.print_info(key, value);
}

/// Print a blank line
pub fn print_blank() {
    BOOT_CONSOLE.print_blank();
}



/// Begin batching GPU output - defers flushes until batch_end_gpu is called
/// This is for use in main.rs to batch multiple print calls together
pub fn batch_begin_gpu() {
    if GpuConsole::is_available() {
        crate::ui::boot::batch_begin();
    }
}

/// End batching GPU output and flush all accumulated changes
pub fn batch_end_gpu() {
    if GpuConsole::is_available() {
        crate::ui::boot::batch_end();
    }
}