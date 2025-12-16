//! DesignWare APB UART driver for Allwinner D1
//!
//! This driver is used on real D1 hardware (Lichee RV 86).
//! The D1 uses a DesignWare 8250-compatible UART with 32-bit register access.

use core::fmt::{self, Write};

/// UART0 base address on Allwinner D1
const DW_UART_BASE: usize = 0x0250_0000;

// Register offsets (32-bit registers, 4-byte spacing)
const DW_RBR: usize = 0x00;   // Receive Buffer Register (read)
const DW_THR: usize = 0x00;   // Transmit Holding Register (write)
const DW_DLL: usize = 0x00;   // Divisor Latch Low (when DLAB=1)
const DW_DLH: usize = 0x04;   // Divisor Latch High (when DLAB=1)
const DW_IER: usize = 0x04;   // Interrupt Enable Register (when DLAB=0)
const DW_FCR: usize = 0x08;   // FIFO Control Register (write)
const DW_IIR: usize = 0x08;   // Interrupt Identification Register (read)
const DW_LCR: usize = 0x0C;   // Line Control Register
const DW_MCR: usize = 0x10;   // Modem Control Register
const DW_LSR: usize = 0x14;   // Line Status Register
const DW_MSR: usize = 0x18;   // Modem Status Register
const DW_USR: usize = 0x7C;   // UART Status Register

// LSR bits
const LSR_DR: u32 = 1 << 0;     // Data Ready
const LSR_THRE: u32 = 1 << 5;   // Transmit Holding Register Empty
const LSR_TEMT: u32 = 1 << 6;   // Transmitter Empty

// LCR bits
const LCR_DLAB: u32 = 1 << 7;   // Divisor Latch Access Bit
const LCR_8N1: u32 = 0x03;      // 8 data bits, no parity, 1 stop bit

// FCR bits
const FCR_FIFO_EN: u32 = 1 << 0;   // FIFO Enable
const FCR_RX_RST: u32 = 1 << 1;    // Receiver FIFO Reset
const FCR_TX_RST: u32 = 1 << 2;    // Transmitter FIFO Reset

/// DesignWare UART console
pub struct DwConsole {
    base: usize,
}

impl DwConsole {
    pub const fn new() -> Self {
        Self { base: DW_UART_BASE }
    }

    /// Initialize UART (assumes clocks already enabled by U-Boot/OpenSBI)
    pub fn init(&self) {
        unsafe {
            let base = self.base as *mut u32;
            
            // Disable interrupts
            core::ptr::write_volatile(base.add(DW_IER / 4), 0);
            
            // Enable FIFO, clear FIFOs
            core::ptr::write_volatile(
                base.add(DW_FCR / 4),
                FCR_FIFO_EN | FCR_RX_RST | FCR_TX_RST
            );
            
            // 8N1 mode (8 data bits, no parity, 1 stop bit)
            core::ptr::write_volatile(base.add(DW_LCR / 4), LCR_8N1);
            
            // Note: Baud rate divisor typically set by bootloader
            // For manual setup: set DLAB=1, write DLL/DLH, set DLAB=0
        }
    }

    #[inline]
    fn read_lsr(&self) -> u32 {
        unsafe {
            core::ptr::read_volatile((self.base + DW_LSR) as *const u32)
        }
    }

    #[inline]
    fn tx_ready(&self) -> bool {
        (self.read_lsr() & LSR_THRE) != 0
    }

    #[inline]
    fn rx_ready(&self) -> bool {
        (self.read_lsr() & LSR_DR) != 0
    }

    pub fn write_byte(&self, byte: u8) {
        // Wait for transmit buffer empty
        while !self.tx_ready() {
            core::hint::spin_loop();
        }
        unsafe {
            core::ptr::write_volatile((self.base + DW_THR) as *mut u32, byte as u32);
        }
    }

    pub fn read_byte(&self) -> Option<u8> {
        if self.rx_ready() {
            unsafe {
                Some(core::ptr::read_volatile((self.base + DW_RBR) as *const u32) as u8)
            }
        } else {
            None
        }
    }

    pub fn read_byte_blocking(&self) -> u8 {
        loop {
            if let Some(byte) = self.read_byte() {
                return byte;
            }
            core::hint::spin_loop();
        }
    }
}

impl Write for DwConsole {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        for byte in s.bytes() {
            self.write_byte(byte);
        }
        Ok(())
    }
}

// ============================================================================
// Global Functions (matching uart.rs interface)
// ============================================================================

/// Write a string to the UART
pub fn write_str(s: &str) {
    let console = DwConsole::new();
    for byte in s.bytes() {
        console.write_byte(byte);
    }
}

/// Write a string followed by newline
pub fn write_line(s: &str) {
    write_str(s);
    write_str("\n");
}

/// Write a single byte
pub fn write_byte(byte: u8) {
    DwConsole::new().write_byte(byte);
}

/// Check if input is available
pub fn has_pending_input() -> bool {
    DwConsole::new().rx_ready()
}

/// Read a character non-blocking
pub fn read_char_nonblocking() -> Option<u8> {
    DwConsole::new().read_byte()
}
