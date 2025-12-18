//! Allwinner D1 EMAC (Ethernet MAC) Driver
//!
//! Driver for the DWMAC-based Ethernet controller in the D1 SoC.
//! Used with RTL8201F PHY on Lichee RV 86.
//!
//! # Memory Map
//! - EMAC: 0x0450_0000
//! - SYSCON for EMAC: 0x0300_0030

use crate::device::{NetworkDevice, NetworkError};
use core::ptr::{read_volatile, write_volatile};
use alloc::vec::Vec;

// =============================================================================
// Register Definitions
// =============================================================================

const EMAC_BASE: usize = 0x0450_0000;
const SYSCON_EMAC: usize = 0x0300_0030;

// EMAC Register Offsets
const EMAC_BASIC_CTL0: usize = 0x00;     // Basic Control 0
const EMAC_BASIC_CTL1: usize = 0x04;     // Basic Control 1
const EMAC_INT_STA: usize = 0x08;        // Interrupt Status
const EMAC_INT_EN: usize = 0x0C;         // Interrupt Enable
const EMAC_TX_CTL0: usize = 0x10;        // TX Control 0
const EMAC_TX_CTL1: usize = 0x14;        // TX Control 1
const EMAC_TX_FLOW_CTL: usize = 0x1C;    // TX Flow Control
const EMAC_TX_DMA_DESC: usize = 0x20;    // TX DMA Descriptor Address
const EMAC_RX_CTL0: usize = 0x24;        // RX Control 0
const EMAC_RX_CTL1: usize = 0x28;        // RX Control 1
const EMAC_RX_DMA_DESC: usize = 0x34;    // RX DMA Descriptor Address
const EMAC_RX_FRM_FLT: usize = 0x38;     // RX Frame Filter
const EMAC_RX_HASH0: usize = 0x40;       // RX Hash Table 0
const EMAC_RX_HASH1: usize = 0x44;       // RX Hash Table 1
const EMAC_MII_CMD: usize = 0x48;        // MII Command Register
const EMAC_MII_DATA: usize = 0x4C;       // MII Data Register
const EMAC_ADDR_HIGH: usize = 0x50;      // MAC Address High
const EMAC_ADDR_LOW: usize = 0x54;       // MAC Address Low
const EMAC_TX_DMA_STA: usize = 0xB0;     // TX DMA Status
const EMAC_TX_CUR_DESC: usize = 0xB4;    // TX Current Descriptor
const EMAC_TX_CUR_BUF: usize = 0xB8;     // TX Current Buffer
const EMAC_RX_DMA_STA: usize = 0xC0;     // RX DMA Status
const EMAC_RX_CUR_DESC: usize = 0xC4;    // RX Current Descriptor
const EMAC_RX_CUR_BUF: usize = 0xC8;     // RX Current Buffer
const EMAC_RGMII_STA: usize = 0xD0;      // RGMII Status

// Custom VM extension register (for relay IP assignment)
const EMAC_IP_CONFIG: usize = 0x100;     // IP address (VM extension)

// Control Register Bits
const CTL0_FULL_DUPLEX: u32 = 1 << 0;
const CTL0_LOOPBACK: u32 = 1 << 1;
const CTL0_SPEED_1000: u32 = 0 << 2;
const CTL0_SPEED_100: u32 = 3 << 2;
const CTL0_SPEED_10: u32 = 2 << 2;

const CTL1_SOFT_RST: u32 = 1 << 0;
const CTL1_RX_TX_PRI: u32 = 1 << 1;
const CTL1_BURST_LEN: u32 = 8 << 24;

const TX_CTL0_TX_EN: u32 = 1 << 31;
const TX_CTL1_TX_DMA_EN: u32 = 1 << 30;

const RX_CTL0_RX_EN: u32 = 1 << 31;
const RX_CTL1_RX_DMA_EN: u32 = 1 << 30;

// PHY Address (RTL8201F)
const PHY_ADDR: u32 = 1;

// MII Register Addresses
const MII_BMCR: u32 = 0x00;      // Basic Mode Control
const MII_BMSR: u32 = 0x01;      // Basic Mode Status
const MII_PHYSID1: u32 = 0x02;   // PHY ID 1
const MII_PHYSID2: u32 = 0x03;   // PHY ID 2
const MII_ADVERTISE: u32 = 0x04; // Advertisement Control
const MII_LPA: u32 = 0x05;       // Link Partner Ability

// BMSR Bits
const BMSR_LINK: u32 = 1 << 2;
const BMSR_100FULL: u32 = 1 << 14;
const BMSR_100HALF: u32 = 1 << 13;
const BMSR_10FULL: u32 = 1 << 12;
const BMSR_10HALF: u32 = 1 << 11;

// =============================================================================
// DMA Descriptor
// =============================================================================

#[repr(C, align(4))]
struct DmaDescriptor {
    status: u32,
    size: u32,
    buf_addr: u32,
    next: u32,
}

const DESC_OWN: u32 = 1 << 31;        // DMA owns this descriptor
const DESC_FIRST: u32 = 1 << 29;      // First segment of frame
const DESC_LAST: u32 = 1 << 30;       // Last segment of frame
const DESC_END_RING: u32 = 1 << 25;   // End of descriptor ring
const DESC_CHAIN: u32 = 1 << 24;      // Second address is next descriptor

const TX_DESC_COUNT: usize = 32;
const RX_DESC_COUNT: usize = 32;
const BUFFER_SIZE: usize = 2048;

// =============================================================================
// Driver Implementation
// =============================================================================

/// D1 EMAC controller driver
pub struct D1Emac {
    base: usize,
    mac_addr: [u8; 6],
    tx_desc: Vec<DmaDescriptor>,
    rx_desc: Vec<DmaDescriptor>,
    tx_buffers: Vec<Vec<u8>>,
    rx_buffers: Vec<Vec<u8>>,
    tx_head: usize,
    rx_head: usize,
    initialized: bool,
}

impl D1Emac {
    /// Create new EMAC driver
    pub fn new() -> Self {
        Self {
            base: EMAC_BASE,
            mac_addr: [0x02, 0x00, 0x00, 0x00, 0x00, 0x01], // Default MAC
            tx_desc: Vec::new(),
            rx_desc: Vec::new(),
            tx_buffers: Vec::new(),
            rx_buffers: Vec::new(),
            tx_head: 0,
            rx_head: 0,
            initialized: false,
        }
    }

    /// Initialize the EMAC controller
    pub fn init(&mut self) -> Result<(), NetworkError> {
        // Reset controller
        self.write_reg(EMAC_BASIC_CTL1, CTL1_SOFT_RST);
        for _ in 0..10000 {
            if (self.read_reg(EMAC_BASIC_CTL1) & CTL1_SOFT_RST) == 0 {
                break;
            }
            core::hint::spin_loop();
        }

        // Read MAC address from MMIO registers (set by emulator/VM)
        // This allows the kernel to use the MAC assigned by the relay
        let addr_low = self.read_reg(EMAC_ADDR_LOW);
        let addr_high = self.read_reg(EMAC_ADDR_HIGH);
        if addr_low != 0 || addr_high != 0 {
            // Use MAC from MMIO registers
            self.mac_addr[0] = (addr_low & 0xFF) as u8;
            self.mac_addr[1] = ((addr_low >> 8) & 0xFF) as u8;
            self.mac_addr[2] = ((addr_low >> 16) & 0xFF) as u8;
            self.mac_addr[3] = ((addr_low >> 24) & 0xFF) as u8;
            self.mac_addr[4] = (addr_high & 0xFF) as u8;
            self.mac_addr[5] = ((addr_high >> 8) & 0xFF) as u8;
        }
        // Otherwise keep the default MAC [0x02, 0x00, 0x00, 0x00, 0x00, 0x01]

        // Initialize PHY
        self.phy_init()?;

        // Allocate DMA descriptors and buffers
        self.alloc_dma_resources();

        // Set MAC address (write it back to registers)
        self.set_mac_address(&self.mac_addr.clone());

        // Configure TX
        self.write_reg(EMAC_TX_CTL1, TX_CTL1_TX_DMA_EN | CTL1_BURST_LEN);
        self.write_reg(EMAC_TX_DMA_DESC, self.tx_desc.as_ptr() as u32);

        // Configure RX
        self.write_reg(EMAC_RX_CTL1, RX_CTL1_RX_DMA_EN | CTL1_BURST_LEN);
        self.write_reg(EMAC_RX_DMA_DESC, self.rx_desc.as_ptr() as u32);

        // Enable RX frame filter (accept our MAC + broadcast)
        self.write_reg(EMAC_RX_FRM_FLT, 0x00000000);

        // Configure speed/duplex based on PHY
        let speed_ctl = self.get_speed_ctl();
        self.write_reg(EMAC_BASIC_CTL0, speed_ctl | CTL0_FULL_DUPLEX);

        // Enable TX and RX
        self.write_reg(EMAC_TX_CTL0, TX_CTL0_TX_EN);
        self.write_reg(EMAC_RX_CTL0, RX_CTL0_RX_EN);

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

    /// Read IP address assigned by VM relay (VM extension register)
    /// Returns Some([a, b, c, d]) if IP is assigned, None if not
    pub fn get_config_ip(&self) -> Option<[u8; 4]> {
        let ip_val = self.read_reg(EMAC_IP_CONFIG);
        if ip_val == 0 {
            None  // No IP assigned
        } else {
            Some([
                ((ip_val >> 24) & 0xFF) as u8,
                ((ip_val >> 16) & 0xFF) as u8,
                ((ip_val >> 8) & 0xFF) as u8,
                (ip_val & 0xFF) as u8,
            ])
        }
    }

    fn mdio_read(&self, phy: u32, reg: u32) -> u32 {
        // Wait for MII not busy
        for _ in 0..10000 {
            let cmd = self.read_reg(EMAC_MII_CMD);
            if (cmd & (1 << 0)) == 0 {
                break;
            }
            core::hint::spin_loop();
        }

        // Issue read command
        let cmd = (phy << 12) | (reg << 4) | (1 << 1) | (1 << 0);
        self.write_reg(EMAC_MII_CMD, cmd);

        // Wait for completion
        for _ in 0..10000 {
            let cmd = self.read_reg(EMAC_MII_CMD);
            if (cmd & (1 << 0)) == 0 {
                break;
            }
            core::hint::spin_loop();
        }

        self.read_reg(EMAC_MII_DATA) & 0xFFFF
    }

    fn mdio_write(&self, phy: u32, reg: u32, value: u32) {
        // Wait for MII not busy
        for _ in 0..10000 {
            let cmd = self.read_reg(EMAC_MII_CMD);
            if (cmd & (1 << 0)) == 0 {
                break;
            }
            core::hint::spin_loop();
        }

        // Write data
        self.write_reg(EMAC_MII_DATA, value);

        // Issue write command
        let cmd = (phy << 12) | (reg << 4) | (1 << 0);
        self.write_reg(EMAC_MII_CMD, cmd);

        // Wait for completion
        for _ in 0..10000 {
            let cmd = self.read_reg(EMAC_MII_CMD);
            if (cmd & (1 << 0)) == 0 {
                break;
            }
            core::hint::spin_loop();
        }
    }

    fn phy_init(&self) -> Result<(), NetworkError> {
        // Check PHY ID
        let id1 = self.mdio_read(PHY_ADDR, MII_PHYSID1);
        let id2 = self.mdio_read(PHY_ADDR, MII_PHYSID2);

        if id1 == 0xFFFF || id1 == 0x0000 {
            return Err(NetworkError::PhyError);
        }

        // Reset PHY
        self.mdio_write(PHY_ADDR, MII_BMCR, 0x8000);
        for _ in 0..100000 {
            let bmcr = self.mdio_read(PHY_ADDR, MII_BMCR);
            if (bmcr & 0x8000) == 0 {
                break;
            }
            core::hint::spin_loop();
        }

        // Enable auto-negotiation
        self.mdio_write(PHY_ADDR, MII_ADVERTISE, 0x01E1); // 10/100 FD/HD
        self.mdio_write(PHY_ADDR, MII_BMCR, 0x1200);      // Enable AN, restart

        Ok(())
    }

    fn get_speed_ctl(&self) -> u32 {
        let bmsr = self.mdio_read(PHY_ADDR, MII_BMSR);
        let lpa = self.mdio_read(PHY_ADDR, MII_LPA);

        if (lpa & 0x0100) != 0 || (bmsr & BMSR_100FULL) != 0 {
            CTL0_SPEED_100
        } else if (lpa & 0x0080) != 0 || (bmsr & BMSR_100HALF) != 0 {
            CTL0_SPEED_100
        } else {
            CTL0_SPEED_10
        }
    }

    fn alloc_dma_resources(&mut self) {
        // Allocate TX descriptors and buffers
        for i in 0..TX_DESC_COUNT {
            let buf = alloc::vec![0u8; BUFFER_SIZE];
            let next = if i == TX_DESC_COUNT - 1 { 0 } else { i + 1 };
            
            self.tx_desc.push(DmaDescriptor {
                status: 0,
                size: 0,
                buf_addr: buf.as_ptr() as u32,
                next: 0, // Set after all allocated
            });
            self.tx_buffers.push(buf);
        }

        // Link TX descriptors
        for i in 0..TX_DESC_COUNT {
            let next_idx = (i + 1) % TX_DESC_COUNT;
            self.tx_desc[i].next = &self.tx_desc[next_idx] as *const _ as u32;
        }

        // Allocate RX descriptors and buffers
        for i in 0..RX_DESC_COUNT {
            let buf = alloc::vec![0u8; BUFFER_SIZE];
            
            self.rx_desc.push(DmaDescriptor {
                status: DESC_OWN,  // Give to DMA
                size: (BUFFER_SIZE as u32) << 16,
                buf_addr: buf.as_ptr() as u32,
                next: 0,
            });
            self.rx_buffers.push(buf);
        }

        // Link RX descriptors
        for i in 0..RX_DESC_COUNT {
            let next_idx = (i + 1) % RX_DESC_COUNT;
            self.rx_desc[i].next = &self.rx_desc[next_idx] as *const _ as u32;
        }
    }

    fn set_mac_address(&self, mac: &[u8; 6]) {
        let low = (mac[0] as u32)
            | ((mac[1] as u32) << 8)
            | ((mac[2] as u32) << 16)
            | ((mac[3] as u32) << 24);
        let high = (mac[4] as u32) | ((mac[5] as u32) << 8);

        self.write_reg(EMAC_ADDR_LOW, low);
        self.write_reg(EMAC_ADDR_HIGH, high);
    }
}

// =============================================================================
// NetworkDevice Trait Implementation
// =============================================================================

impl NetworkDevice for D1Emac {
    fn mac_address(&self) -> [u8; 6] {
        self.mac_addr
    }

    fn link_up(&self) -> bool {
        if !self.initialized {
            return false;
        }
        let bmsr = self.mdio_read(PHY_ADDR, MII_BMSR);
        (bmsr & BMSR_LINK) != 0
    }

    fn link_speed(&self) -> u32 {
        let lpa = self.mdio_read(PHY_ADDR, MII_LPA);
        if (lpa & 0x0180) != 0 {
            100
        } else {
            10
        }
    }

    fn transmit(&mut self, packet: &[u8]) -> Result<(), NetworkError> {
        if !self.initialized {
            return Err(NetworkError::NotReady);
        }

        let desc = &mut self.tx_desc[self.tx_head];
        
        // Check if descriptor is free
        if (desc.status & DESC_OWN) != 0 {
            return Err(NetworkError::TxFailed);
        }

        // Copy packet to buffer
        let buf = &mut self.tx_buffers[self.tx_head];
        let len = packet.len().min(BUFFER_SIZE);
        buf[..len].copy_from_slice(&packet[..len]);

        // Setup descriptor
        desc.size = len as u32;
        desc.status = DESC_OWN | DESC_FIRST | DESC_LAST;
        
        // Trigger TX
        self.write_reg(EMAC_TX_CTL1, self.read_reg(EMAC_TX_CTL1) | (1 << 31));

        self.tx_head = (self.tx_head + 1) % TX_DESC_COUNT;
        Ok(())
    }

    fn receive(&mut self, buf: &mut [u8]) -> Result<usize, NetworkError> {
        if !self.initialized {
            return Err(NetworkError::NotReady);
        }

        let desc = &mut self.rx_desc[self.rx_head];
        let desc_addr = desc as *const DmaDescriptor as usize;
        
        // Debug: Use volatile read to ensure we see the latest value
        let status = unsafe { core::ptr::read_volatile(&desc.status) };
        
        // Check if descriptor has data (OWN = 0 means DMA finished, packet available)
        if (status & DESC_OWN) != 0 {
            // Only log occasionally to avoid spam
            static mut RX_CHECK_COUNT: u32 = 0;
            unsafe {
                RX_CHECK_COUNT += 1;
                if RX_CHECK_COUNT % 10000 == 0 {
                    crate::println!("[D1_EMAC RX poll #{}] rx_head={}, desc=0x{:08x}, status=0x{:08x}, no packet",
                        RX_CHECK_COUNT, self.rx_head, desc_addr, status);
                }
            }
            return Err(NetworkError::NoPacket);
        }

        // Get frame length
        let frame_len = ((status >> 16) & 0x1FFF) as usize;  // 13 bits for frame length (bits 16-28)
        if frame_len > buf.len() {
            // Give back to DMA and skip
            unsafe { core::ptr::write_volatile(&mut desc.status, DESC_OWN); }
            self.rx_head = (self.rx_head + 1) % RX_DESC_COUNT;
            return Err(NetworkError::RxBufferTooSmall);
        }

        // Copy data from the buffer (which VM wrote to)
        let rx_buf = &self.rx_buffers[self.rx_head];
        buf[..frame_len].copy_from_slice(&rx_buf[..frame_len]);

        // Give descriptor back to DMA
        unsafe { core::ptr::write_volatile(&mut desc.status, DESC_OWN); }

        self.rx_head = (self.rx_head + 1) % RX_DESC_COUNT;
        Ok(frame_len)
    }

    fn has_packet(&self) -> bool {
        if !self.initialized {
            return false;
        }
        let desc = &self.rx_desc[self.rx_head];
        // Use volatile read to see the DMA-modified value
        let status = unsafe { core::ptr::read_volatile(&desc.status) };
        (status & DESC_OWN) == 0
    }
}

// =============================================================================
// Module Interface
// =============================================================================

/// Check if D1 EMAC is present by probing PHY ID
pub fn probe() -> bool {
    // Try to read PHY ID from MII registers
    // VM emulation returns 0x001C (RTL8201F), real hardware may vary
    let emac = D1Emac::new();
    let id1 = emac.mdio_read(PHY_ADDR, MII_PHYSID1);
    
    // Valid PHY IDs are not 0xFFFF (no device) and not 0x0000 (no response)
    id1 != 0xFFFF && id1 != 0x0000
}

/// Create a new D1 EMAC device instance
/// Returns initialized EMAC or error if init fails
pub fn create_device() -> Result<D1Emac, NetworkError> {
    let mut emac = D1Emac::new();
    emac.init()?;
    Ok(emac)
}

/// Initialize EMAC and register as global network device
#[allow(dead_code)]
pub fn init() -> Result<(), NetworkError> {
    let mut emac = D1Emac::new();
    emac.init()?;
    
    // Register as global network device
    unsafe {
        crate::device::network::init_network_device(alloc::boxed::Box::new(emac));
    }
    
    Ok(())
}

// =============================================================================
// smoltcp Device Implementation
// =============================================================================

use smoltcp::phy::{Device, DeviceCapabilities, Medium, RxToken, TxToken};
use smoltcp::time::Instant;

/// Wrapper for D1Emac to implement smoltcp Device trait
pub struct D1EmacDevice<'a>(pub &'a mut D1Emac);

impl Device for D1EmacDevice<'_> {
    type RxToken<'a> = D1RxToken where Self: 'a;
    type TxToken<'a> = D1TxToken<'a> where Self: 'a;

    fn capabilities(&self) -> DeviceCapabilities {
        let mut caps = DeviceCapabilities::default();
        caps.medium = Medium::Ethernet;
        caps.max_transmission_unit = 1500;
        caps.max_burst_size = Some(1);
        caps.checksum = smoltcp::phy::ChecksumCapabilities::default();
        caps
    }

    fn receive(&mut self, _timestamp: Instant) -> Option<(Self::RxToken<'_>, Self::TxToken<'_>)> {
        if !self.0.has_packet() {
            return None;
        }
        
        // Receive packet into buffer
        let mut buf = alloc::vec![0u8; BUFFER_SIZE];
        match self.0.receive(&mut buf) {
            Ok(len) => {
                buf.truncate(len);
                Some((
                    D1RxToken { buffer: buf },
                    D1TxToken { device: self.0 },
                ))
            }
            Err(_) => None,
        }
    }

    fn transmit(&mut self, _timestamp: Instant) -> Option<Self::TxToken<'_>> {
        // Always allow transmit (device will handle buffer exhaustion)
        Some(D1TxToken { device: self.0 })
    }
}

/// RX token for received packets
pub struct D1RxToken {
    buffer: Vec<u8>,
}

impl RxToken for D1RxToken {
    fn consume<R, F>(self, f: F) -> R
    where
        F: FnOnce(&[u8]) -> R,
    {
        f(&self.buffer)
    }
}

/// TX token for transmitting packets
pub struct D1TxToken<'a> {
    device: &'a mut D1Emac,
}

impl TxToken for D1TxToken<'_> {
    fn consume<R, F>(self, len: usize, f: F) -> R
    where
        F: FnOnce(&mut [u8]) -> R,
    {
        let mut buffer = alloc::vec![0u8; len];
        let result = f(&mut buffer);
        
        // Send the packet (ignore errors, smoltcp handles retransmission)
        let _ = self.device.transmit(&buffer);
        
        result
    }
}

