//! Allwinner D1 SD/MMC Controller Driver
//!
//! Driver for the Allwinner SMHC (SD/MMC Host Controller) found in the D1 SoC.
//! Used on Lichee RV 86 for SD card access.
//!
//! # Memory Map
//! - MMC0: 0x0402_0000 (SD card slot)
//! - MMC1: 0x0402_1000
//! - MMC2: 0x0402_2000 (eMMC if present)

use crate::device::{BlockDevice, BlockError};
use core::ptr::{read_volatile, write_volatile};

// =============================================================================
// Register Definitions
// =============================================================================

const MMC0_BASE: usize = 0x0402_0000;

// SMHC Register Offsets
const SMHC_CTRL: usize = 0x00;       // Control Register
const SMHC_CLKDIV: usize = 0x04;     // Clock Divider Register
const SMHC_TMOUT: usize = 0x08;      // Timeout Register
const SMHC_CTYPE: usize = 0x0C;      // Card Type Register
const SMHC_BLKSIZ: usize = 0x10;     // Block Size Register
const SMHC_BYTCNT: usize = 0x14;     // Byte Count Register
const SMHC_CMD: usize = 0x18;        // Command Register
const SMHC_CMDARG: usize = 0x1C;     // Command Argument Register
const SMHC_RESP0: usize = 0x20;      // Response Register 0
const SMHC_RESP1: usize = 0x24;      // Response Register 1
const SMHC_RESP2: usize = 0x28;      // Response Register 2
const SMHC_RESP3: usize = 0x2C;      // Response Register 3
const SMHC_INTMASK: usize = 0x30;    // Interrupt Mask Register
const SMHC_MINTSTS: usize = 0x34;    // Masked Interrupt Status
const SMHC_RINTSTS: usize = 0x38;    // Raw Interrupt Status
const SMHC_STATUS: usize = 0x3C;     // Status Register
const SMHC_FIFOTH: usize = 0x40;     // FIFO Threshold Register
const SMHC_FUNS: usize = 0x44;       // Card Detect Register
const SMHC_CBCR: usize = 0x48;       // CIU Byte Count Register
const SMHC_BBCR: usize = 0x4C;       // BIU Byte Count Register
const SMHC_DBGC: usize = 0x50;       // Debug Enable Register
const SMHC_A12A: usize = 0x58;       // Auto CMD12 Argument
const SMHC_NTSR: usize = 0x5C;       // SD New Timing Set Register
const SMHC_HWRST: usize = 0x78;      // Hardware Reset Register
const SMHC_DMAC: usize = 0x80;       // DMA Control Register
const SMHC_DLBA: usize = 0x84;       // Descriptor List Base Address
const SMHC_IDST: usize = 0x88;       // Internal DMA Status Register
const SMHC_IDIE: usize = 0x8C;       // Internal DMA Interrupt Enable
const SMHC_THLD: usize = 0x100;      // Card Threshold Control
const SMHC_EDSD: usize = 0x10C;      // eMMC DDR Start Bit Detection
const SMHC_CSDC: usize = 0x110;      // CRC Status Detect Control
const SMHC_FIFO: usize = 0x200;      // FIFO Access Address

// Command Register Bits
const CMD_START: u32 = 1 << 31;
const CMD_USE_HOLD: u32 = 1 << 29;
const CMD_UPDATE_CLK: u32 = 1 << 21;
const CMD_WAIT_PRE_OVER: u32 = 1 << 13;
const CMD_STOP_ABORT_CMD: u32 = 1 << 12;
const CMD_SEND_INIT_SEQ: u32 = 1 << 15;
const CMD_CHK_RESP_CRC: u32 = 1 << 8;
const CMD_LONG_RESP: u32 = 1 << 7;
const CMD_RESP_EXP: u32 = 1 << 6;
const CMD_DATA_EXP: u32 = 1 << 9;
const CMD_WRITE: u32 = 1 << 10;

// Status Register Bits
const STATUS_FIFO_EMPTY: u32 = 1 << 2;
const STATUS_FIFO_FULL: u32 = 1 << 3;
const STATUS_DATA_BUSY: u32 = 1 << 9;

// Interrupt Status Bits
const INT_CMD_DONE: u32 = 1 << 2;
const INT_DATA_OVER: u32 = 1 << 3;
const INT_RESP_ERR: u32 = 1 << 1;
const INT_RESP_CRC_ERR: u32 = 1 << 6;
const INT_DATA_CRC_ERR: u32 = 1 << 7;
const INT_RESP_TIMEOUT: u32 = 1 << 8;
const INT_DATA_TIMEOUT: u32 = 1 << 9;

// =============================================================================
// Driver Implementation
// =============================================================================

/// D1 MMC controller driver
pub struct D1Mmc {
    base: usize,
    sector_count: u64,
    /// Partition offset in sectors (for accessing SFS on partition 2)
    partition_offset: u64,
    initialized: bool,
}

impl D1Mmc {
    /// Create new MMC driver for MMC0 (SD card slot)
    pub const fn new() -> Self {
        Self {
            base: MMC0_BASE,
            sector_count: 0,
            partition_offset: 0,
            initialized: false,
        }
    }

    /// Get capacity in sectors (for compatibility with VirtioBlock API)
    pub fn capacity(&self) -> u64 {
        self.sector_count
    }

    /// Initialize the MMC controller and detect SD card
    pub fn init(&mut self) -> Result<(), BlockError> {
        use crate::device::uart::{write_str, write_hex};
        
        self.write_reg(SMHC_CTRL, 0x7);  // Software reset
        self.wait_reset()?;

        // Set clock divider (low speed for init)
        self.write_reg(SMHC_CLKDIV, 0x40000000);  // Enable clock
        self.update_clock()?;

        // Set timeout
        self.write_reg(SMHC_TMOUT, 0xFFFFFF40);

        // Set block size to 512 bytes
        self.write_reg(SMHC_BLKSIZ, 512);

        // Send CMD0 (GO_IDLE_STATE)
        self.send_cmd(0, 0, 0)?;

        // Send CMD8 (SEND_IF_COND) - check for SD v2.0
        if self.send_cmd(8, 0x1AA, CMD_RESP_EXP | CMD_CHK_RESP_CRC).is_ok() {
            let resp = self.read_reg(SMHC_RESP0);
            if (resp & 0xFF) != 0xAA {
                return Err(BlockError::NotReady);
            }
        } 

        // Send ACMD41 (SD_SEND_OP_COND) repeatedly until card is ready
        for i in 0..100 {
            // CMD55 (APP_CMD) precedes ACMD
            self.send_cmd(55, 0, CMD_RESP_EXP)?;
            
            // ACMD41 with HCS bit set for SDHC support
            if self.send_cmd(41, 0x40FF8000, CMD_RESP_EXP).is_ok() {
                let resp = self.read_reg(SMHC_RESP0);
                if (resp & 0x80000000) != 0 {
                    // Card is ready
                    break;
                }
            }
            
            // Small delay
            for _ in 0..10000 {
                core::hint::spin_loop();
            }
        }

        // CMD2 (ALL_SEND_CID)
        self.send_cmd(2, 0, CMD_RESP_EXP | CMD_LONG_RESP)?;

        // CMD3 (SEND_RELATIVE_ADDR)
        self.send_cmd(3, 0, CMD_RESP_EXP)?;
        let rca = self.read_reg(SMHC_RESP0) & 0xFFFF0000;

        // CMD7 (SELECT_CARD)
        self.send_cmd(7, rca, CMD_RESP_EXP)?;

        // CMD9 to get card capacity
        self.send_cmd(9, rca, CMD_RESP_EXP | CMD_LONG_RESP)?;
        self.sector_count = self.parse_csd_capacity();

        // Switch to high speed mode
        self.write_reg(SMHC_CLKDIV, 0x40000002);  // Higher clock
        self.update_clock()?;

        // Parse MBR to find filesystem partition (type 0x83 Linux)
        // This offset is added to all sector accesses so fs.rs can use sector 0
        if let Some(offset) = self.find_linux_partition() {
            self.partition_offset = offset;
        }

        self.initialized = true;
        Ok(())
    }

    fn write_reg(&self, offset: usize, value: u32) {
        unsafe {
            write_volatile((self.base + offset) as *mut u32, value);
        }
    }

    fn read_reg(&self, offset: usize) -> u32 {
        unsafe {
            read_volatile((self.base + offset) as *const u32)
        }
    }

    fn wait_reset(&self) -> Result<(), BlockError> {
        for _ in 0..10000 {
            if (self.read_reg(SMHC_CTRL) & 0x7) == 0 {
                return Ok(());
            }
            core::hint::spin_loop();
        }
        Err(BlockError::Timeout)
    }

    fn update_clock(&self) -> Result<(), BlockError> {
        self.write_reg(SMHC_CMD, CMD_START | CMD_UPDATE_CLK | CMD_WAIT_PRE_OVER);
        for _ in 0..10000 {
            if (self.read_reg(SMHC_CMD) & CMD_START) == 0 {
                return Ok(());
            }
            core::hint::spin_loop();
        }
        Err(BlockError::Timeout)
    }

    fn send_cmd(&self, cmd: u32, arg: u32, flags: u32) -> Result<(), BlockError> {
        // Clear interrupts
        self.write_reg(SMHC_RINTSTS, 0xFFFFFFFF);

        // Set argument
        self.write_reg(SMHC_CMDARG, arg);

        // Send command
        let cmd_val = CMD_START | CMD_USE_HOLD | (cmd & 0x3F) | flags;
        self.write_reg(SMHC_CMD, cmd_val);

        // Wait for completion
        for _ in 0..100000 {
            let status = self.read_reg(SMHC_RINTSTS);
            if (status & INT_CMD_DONE) != 0 {
                if (status & (INT_RESP_ERR | INT_RESP_CRC_ERR | INT_RESP_TIMEOUT)) != 0 {
                    return Err(BlockError::ReadFailed);
                }
                return Ok(());
            }
            core::hint::spin_loop();
        }
        Err(BlockError::Timeout)
    }

    /// Find the Linux partition (type 0x83) by reading MBR at sector 0
    /// Returns the start sector of the first Linux partition found
    fn find_linux_partition(&self) -> Option<u64> {
        let mut mbr = [0u8; 512];
        // Read MBR at absolute sector 0 (no offset applied)
        if self.read_block_raw(0, &mut mbr).is_err() {
            return None;
        }

        // Check MBR signature
        if mbr[510] != 0x55 || mbr[511] != 0xAA {
            return None;
        }

        // Parse partition table (starts at offset 446, 4 entries of 16 bytes)
        for i in 0..4 {
            let offset = 446 + i * 16;
            let part_type = mbr[offset + 4];
            
            // Look for Linux partition (0x83)
            if part_type == 0x83 {
                let start_lba = u32::from_le_bytes([
                    mbr[offset + 8],
                    mbr[offset + 9],
                    mbr[offset + 10],
                    mbr[offset + 11],
                ]);
                return Some(start_lba as u64);
            }
        }

        None
    }

    /// Low-level block read without partition offset (for reading MBR)
    fn read_block_raw(&self, sector: u64, buf: &mut [u8]) -> Result<(), BlockError> {
        if buf.len() < 512 {
            return Err(BlockError::BufferSize);
        }

        // Set byte count
        self.write_reg(SMHC_BYTCNT, 512);
        self.write_reg(SMHC_BLKSIZ, 512);

        // Clear interrupts
        self.write_reg(SMHC_RINTSTS, 0xFFFFFFFF);

        // CMD17 (READ_SINGLE_BLOCK)
        self.write_reg(SMHC_CMDARG, sector as u32);
        self.write_reg(SMHC_CMD, CMD_START | CMD_USE_HOLD | 17 | CMD_RESP_EXP | CMD_DATA_EXP | CMD_CHK_RESP_CRC);

        // Read data from FIFO
        let mut offset = 0;
        while offset < 512 {
            for _ in 0..10000 {
                if (self.read_reg(SMHC_STATUS) & STATUS_FIFO_EMPTY) == 0 {
                    break;
                }
                core::hint::spin_loop();
            }

            let word = self.read_reg(SMHC_FIFO);
            buf[offset..offset + 4].copy_from_slice(&word.to_le_bytes());
            offset += 4;
        }

        // Wait for transfer complete
        for _ in 0..10000 {
            if (self.read_reg(SMHC_RINTSTS) & INT_DATA_OVER) != 0 {
                return Ok(());
            }
            core::hint::spin_loop();
        }
        Err(BlockError::Timeout)
    }

    fn parse_csd_capacity(&self) -> u64 {
        // Read CSD register values
        let csd0 = self.read_reg(SMHC_RESP0);
        let csd1 = self.read_reg(SMHC_RESP1);
        
        // For SDHC/SDXC cards (CSD version 2)
        // C_SIZE is bits 69:48 of CSD
        let c_size = ((csd1 & 0x3F) << 16) | ((csd0 >> 16) & 0xFFFF);
        
        // Capacity = (C_SIZE + 1) * 512KB
        ((c_size as u64) + 1) * 1024
    }

    fn read_block(&self, sector: u64, buf: &mut [u8]) -> Result<(), BlockError> {
        if buf.len() < 512 {
            return Err(BlockError::BufferSize);
        }

        // Set byte count
        self.write_reg(SMHC_BYTCNT, 512);
        self.write_reg(SMHC_BLKSIZ, 512);

        // Clear interrupts
        self.write_reg(SMHC_RINTSTS, 0xFFFFFFFF);

        // CMD17 (READ_SINGLE_BLOCK) - apply partition offset
        let actual_sector = sector + self.partition_offset;
        self.write_reg(SMHC_CMDARG, actual_sector as u32);
        self.write_reg(SMHC_CMD, CMD_START | CMD_USE_HOLD | 17 | CMD_RESP_EXP | CMD_DATA_EXP | CMD_CHK_RESP_CRC);

        // Read data from FIFO
        let mut offset = 0;
        while offset < 512 {
            // Wait for data in FIFO
            for _ in 0..10000 {
                if (self.read_reg(SMHC_STATUS) & STATUS_FIFO_EMPTY) == 0 {
                    break;
                }
                core::hint::spin_loop();
            }

            let word = self.read_reg(SMHC_FIFO);
            buf[offset..offset + 4].copy_from_slice(&word.to_le_bytes());
            offset += 4;
        }

        // Wait for transfer complete
        for _ in 0..10000 {
            if (self.read_reg(SMHC_RINTSTS) & INT_DATA_OVER) != 0 {
                return Ok(());
            }
            core::hint::spin_loop();
        }
        Err(BlockError::Timeout)
    }

    fn write_block(&self, sector: u64, buf: &[u8]) -> Result<(), BlockError> {
        if buf.len() < 512 {
            return Err(BlockError::BufferSize);
        }

        // Set byte count
        self.write_reg(SMHC_BYTCNT, 512);
        self.write_reg(SMHC_BLKSIZ, 512);

        // Clear interrupts
        self.write_reg(SMHC_RINTSTS, 0xFFFFFFFF);

        // CMD24 (WRITE_BLOCK) - apply partition offset
        let actual_sector = sector + self.partition_offset;
        self.write_reg(SMHC_CMDARG, actual_sector as u32);
        self.write_reg(SMHC_CMD, CMD_START | CMD_USE_HOLD | 24 | CMD_RESP_EXP | CMD_DATA_EXP | CMD_WRITE | CMD_CHK_RESP_CRC);

        // Write data to FIFO
        let mut offset = 0;
        while offset < 512 {
            // Wait for space in FIFO
            for _ in 0..10000 {
                if (self.read_reg(SMHC_STATUS) & STATUS_FIFO_FULL) == 0 {
                    break;
                }
                core::hint::spin_loop();
            }

            let word = u32::from_le_bytes([buf[offset], buf[offset+1], buf[offset+2], buf[offset+3]]);
            self.write_reg(SMHC_FIFO, word);
            offset += 4;
        }

        // Wait for transfer complete
        for _ in 0..10000 {
            if (self.read_reg(SMHC_RINTSTS) & INT_DATA_OVER) != 0 {
                return Ok(());
            }
            core::hint::spin_loop();
        }
        Err(BlockError::Timeout)
    }

    /// Read a sector from the block device (fs.rs compatibility wrapper)
    pub fn read_sector(&mut self, sector: u64, buf: &mut [u8]) -> Result<(), &'static str> {
        self.read_block(sector, buf).map_err(|_| "IO Error")
    }

    /// Write a sector to the block device (fs.rs compatibility wrapper)
    pub fn write_sector(&mut self, sector: u64, buf: &[u8]) -> Result<(), &'static str> {
        self.write_block(sector, buf).map_err(|_| "IO Error")
    }
}

// =============================================================================
// BlockDevice Trait Implementation
// =============================================================================

impl BlockDevice for D1Mmc {
    fn read(&self, start_sector: u64, buf: &mut [u8]) -> Result<(), BlockError> {
        if !self.initialized {
            return Err(BlockError::NotReady);
        }
        if start_sector >= self.sector_count {
            return Err(BlockError::InvalidSector);
        }

        let sector_count = buf.len() / 512;
        for i in 0..sector_count {
            let sector = start_sector + i as u64;
            let offset = i * 512;
            self.read_block(sector, &mut buf[offset..offset + 512])?;
        }
        Ok(())
    }

    fn write(&self, start_sector: u64, buf: &[u8]) -> Result<(), BlockError> {
        if !self.initialized {
            return Err(BlockError::NotReady);
        }
        if start_sector >= self.sector_count {
            return Err(BlockError::InvalidSector);
        }

        let sector_count = buf.len() / 512;
        for i in 0..sector_count {
            let sector = start_sector + i as u64;
            let offset = i * 512;
            self.write_block(sector, &buf[offset..offset + 512])?;
        }
        Ok(())
    }

    fn sector_count(&self) -> u64 {
        self.sector_count
    }
}

// =============================================================================
// Module Interface
// =============================================================================

/// Initialize MMC and register as global block device
pub fn init() -> Result<(), BlockError> {
    let mut mmc = D1Mmc::new();
    mmc.init()?;
    
    // Register as global block device
    unsafe {
        crate::device::block::init_block_device(alloc::boxed::Box::new(mmc));
    }
    
    Ok(())
}
