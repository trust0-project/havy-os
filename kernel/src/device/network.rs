//! Network device abstraction
//!
//! Provides a unified interface for network devices:
//! - D1 EMAC (DWMAC Ethernet)
//! - (Legacy) VirtIO network

use alloc::boxed::Box;
use alloc::vec::Vec;

/// Network device error types
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetworkError {
    /// Device not ready or not present
    NotReady,
    /// No link detected
    NoLink,
    /// Transmit failed
    TxFailed,
    /// Receive buffer too small
    RxBufferTooSmall,
    /// No packet available
    NoPacket,
    /// DMA error
    DmaError,
    /// PHY initialization failed
    PhyError,
}

/// Network device trait
///
/// Implemented by network device drivers (D1 EMAC, VirtIO net, etc.)
pub trait NetworkDevice: Send + Sync {
    /// Get MAC address
    fn mac_address(&self) -> [u8; 6];

    /// Check if link is up
    fn link_up(&self) -> bool;

    /// Get link speed in Mbps (10, 100, 1000)
    fn link_speed(&self) -> u32 {
        100 // Default to 100Mbps
    }

    /// Transmit a packet
    ///
    /// # Arguments
    /// * `packet` - Complete Ethernet frame (dest MAC, src MAC, ethertype, payload)
    fn transmit(&mut self, packet: &[u8]) -> Result<(), NetworkError>;

    /// Receive a packet
    ///
    /// # Arguments
    /// * `buf` - Buffer to receive into
    ///
    /// # Returns
    /// * `Ok(len)` - Number of bytes received
    /// * `Err(NoPacket)` - No packet available
    fn receive(&mut self, buf: &mut [u8]) -> Result<usize, NetworkError>;

    /// Check if there's a packet available to receive
    fn has_packet(&self) -> bool;

    /// Get MTU (Maximum Transmission Unit)
    fn mtu(&self) -> usize {
        1500
    }
}

/// Global network device instance
static mut NETWORK_DEVICE: Option<Box<dyn NetworkDevice>> = None;

/// Initialize the global network device
///
/// # Safety
/// Must only be called once during kernel init
pub unsafe fn init_network_device(device: Box<dyn NetworkDevice>) {
    NETWORK_DEVICE = Some(device);
}

/// Get a mutable reference to the global network device
pub fn network_device_mut() -> Option<&'static mut dyn NetworkDevice> {
    unsafe { NETWORK_DEVICE.as_mut().map(|d| d.as_mut()) }
}

/// Get MAC address of the network device
pub fn get_mac_address() -> Option<[u8; 6]> {
    unsafe { NETWORK_DEVICE.as_ref().map(|d| d.mac_address()) }
}
