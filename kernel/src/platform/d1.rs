//! Allwinner D1 / Lichee RV 86 platform configuration
//!
//! Memory map and peripheral addresses for the Allwinner D1 SoC
//! with XuanTie C906 RISC-V core.

use super::Platform;

/// Allwinner D1 platform
pub struct D1;

impl Platform for D1 {
    const DRAM_BASE: usize = 0x4000_0000;
    const UART_BASE: usize = 0x0250_0000;
    const TIMER_FREQ_HZ: u64 = 24_000_000; // 24 MHz oscillator
}

// ============================================================================
// Memory Map Constants
// ============================================================================

/// DRAM base address (D1 uses 0x4000_0000, not 0x8000_0000)
pub const DRAM_BASE: usize = 0x4000_0000;

/// Kernel load address (after OpenSBI's 2MB reservation)
pub const KERNEL_START: usize = 0x4020_0000;

/// Heap is sized by `d1.ld` (`_sheap` .. `_eheap`); leftover DRAM after FB + stacks.

/// Total DRAM size (512 MB on Lichee RV 86)
pub const DRAM_SIZE: usize = 512 * 1024 * 1024;

// ============================================================================
// Clock Control Unit (CCU)
// ============================================================================

/// CCU base address
pub const CCU_BASE: usize = 0x0200_1000;

/// UART Bus Gating Reset Register
pub const CCU_UART_BGR: usize = CCU_BASE + 0x90C;

/// MMC Bus Gating Reset Register
pub const CCU_MMC_BGR: usize = CCU_BASE + 0x84C;

/// EMAC Bus Gating Reset Register  
pub const CCU_EMAC_BGR: usize = CCU_BASE + 0x97C;

// ============================================================================
// GPIO / Pin Control (PIO)
// ============================================================================

/// GPIO base address
pub const GPIO_BASE: usize = 0x0200_0000;

// ============================================================================
// UART (DesignWare APB UART)
// ============================================================================

/// UART0 base address (debug console)
pub const UART_BASE: usize = 0x0250_0000;

/// DesignWare APB UART register stride (16550 numbers × 4)
pub const UART_STRIDE: usize = 4;

/// UART1 base address
pub const UART1_BASE: usize = 0x0250_0400;

/// UART2 base address
pub const UART2_BASE: usize = 0x0250_0800;

// ============================================================================
// PLIC (T-Head)
// ============================================================================

/// T-Head PLIC base address
pub const PLIC_BASE: usize = 0x1000_0000;

// ============================================================================
// SD/MMC Controller
// ============================================================================

/// MMC0 base address (SD card slot)
pub const MMC0_BASE: usize = 0x0402_0000;

/// MMC1 base address
pub const MMC1_BASE: usize = 0x0402_1000;

/// MMC2 base address (eMMC if present)
pub const MMC2_BASE: usize = 0x0402_2000;

// ============================================================================
// Ethernet (DWMAC/EMAC)
// ============================================================================

/// EMAC base address
pub const EMAC_BASE: usize = 0x0450_0000;

/// EMAC PHY interface control
pub const EMAC_EPHY_CLK: usize = 0x0303_4030;

// ============================================================================
// Display Engine
// ============================================================================

/// Display Engine (DE) base address
pub const DE_BASE: usize = 0x0500_0000;

/// Display Engine Mixer 0
pub const DE_MUX0: usize = 0x0510_0000;

/// TCON LCD0 base address
pub const TCON_LCD0: usize = 0x0546_1000;

/// MIPI DSI base address
pub const MIPI_DSI: usize = 0x0545_0000;

/// DSI PHY base address
pub const DPHY: usize = 0x0545_1000;

// ============================================================================
// USB
// ============================================================================

/// USB OTG base address
pub const USB_OTG: usize = 0x0410_0000;

/// USB EHCI/OHCI base address
pub const USB_HCI: usize = 0x0420_0000;

// ============================================================================
// Timer Configuration
// ============================================================================

/// Timer frequency in Hz (24 MHz oscillator on D1)
pub const TIMER_FREQ_HZ: u64 = 24_000_000;

/// Timer interval for 10ms tick
pub const TIMER_INTERVAL: u64 = TIMER_FREQ_HZ / 100;

// ============================================================================
// UART Type
// ============================================================================

/// UART type for this platform
pub const UART_TYPE: &str = "designware";

// ============================================================================
// VirtIO Configuration
// ============================================================================

/// VirtIO is NOT available on real hardware
pub const HAS_VIRTIO: bool = false;

// ============================================================================
// Display Configuration (Lichee RV 86 Panel)
// ============================================================================

/// ST7701 panel on Lichee RV 86: 480×480.
pub const DISPLAY_WIDTH: u32 = 480;
pub const DISPLAY_HEIGHT: u32 = 480;
/// 480×4 = 1920 is not a multiple of 256; pad for WebGPU `bytesPerRow`.
pub const FB_STRIDE: usize = 2048;
/// Same DRAM-relative offsets as virt, but inside D1 DRAM (`0x4000_0000`).
pub const FB_META_OFFSET: usize = 0x00FF_F000;
pub const FB_OFFSET: usize = 0x0100_0000;
pub const FB_META_ADDR: usize = DRAM_BASE + FB_META_OFFSET;
pub const FB_ADDR: usize = DRAM_BASE + FB_OFFSET;

/// Inclusive max hart id. Matches `d1.ld`.
pub const MAX_HART_ID: usize = 0;

/// Panel controller: ST7701S
pub const PANEL_TYPE: &str = "st7701s";

/// Touch controller: FT6336U (I2C address 0x38)
pub const TOUCH_I2C_ADDR: u8 = 0x38;
