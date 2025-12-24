//! FDT (Flattened Device Tree) Parser
//!
//! Parses the Device Tree Blob (DTB) to extract device information.
//! This allows the kernel to discover devices dynamically rather than
//! relying on hardcoded addresses.

use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::ptr::read_volatile;

/// FDT header magic number
const FDT_MAGIC: u32 = 0xd00dfeed;

/// FDT tokens
const FDT_BEGIN_NODE: u32 = 0x00000001;
const FDT_END_NODE: u32 = 0x00000002;
const FDT_PROP: u32 = 0x00000003;
const FDT_NOP: u32 = 0x00000004;
const FDT_END: u32 = 0x00000009;

/// Discovered device from DTB
#[derive(Clone, Debug)]
pub struct DeviceNode {
    /// Node name (e.g., "serial@10000000")
    pub name: String,
    /// Compatible string (e.g., "ns16550a", "virtio,mmio")
    pub compatible: String,
    /// MMIO base address
    pub reg_base: u64,
    /// MMIO region size
    pub reg_size: u64,
    /// Interrupt number (if present)
    pub interrupts: Option<u32>,
}

/// FDT Header structure
#[repr(C)]
struct FdtHeader {
    magic: u32,
    totalsize: u32,
    off_dt_struct: u32,
    off_dt_strings: u32,
    off_mem_rsvmap: u32,
    version: u32,
    last_comp_version: u32,
    boot_cpuid_phys: u32,
    size_dt_strings: u32,
    size_dt_struct: u32,
}

/// Read a big-endian u32 from memory
#[inline]
fn read_be32(addr: usize) -> u32 {
    unsafe { u32::from_be(read_volatile(addr as *const u32)) }
}

/// Read a string from DTB strings block
fn read_string(strings_base: usize, offset: u32) -> String {
    let addr = strings_base + offset as usize;
    let mut len = 0usize;
    
    // Find null terminator (limit to 256 chars)
    while len < 256 {
        let byte = unsafe { read_volatile((addr + len) as *const u8) };
        if byte == 0 {
            break;
        }
        len += 1;
    }
    
    if len == 0 {
        return String::new();
    }
    
    let mut bytes = Vec::with_capacity(len);
    for i in 0..len {
        let byte = unsafe { read_volatile((addr + i) as *const u8) };
        bytes.push(byte);
    }
    
    String::from_utf8(bytes).unwrap_or_default()
}

/// Read a null-terminated string from structure block
fn read_node_name(addr: usize) -> (String, usize) {
    let mut len = 0usize;
    
    while len < 256 {
        let byte = unsafe { read_volatile((addr + len) as *const u8) };
        if byte == 0 {
            break;
        }
        len += 1;
    }
    
    let mut bytes = Vec::with_capacity(len);
    for i in 0..len {
        let byte = unsafe { read_volatile((addr + i) as *const u8) };
        bytes.push(byte);
    }
    
    // Align to 4 bytes (include null terminator in alignment calculation)
    let consumed = ((len + 1) + 3) & !3;
    
    (String::from_utf8(bytes).unwrap_or_default(), consumed)
}

/// Parse all devices from DTB
pub fn parse_devices(dtb_addr: usize) -> Vec<DeviceNode> {
    let mut devices = Vec::new();
    
    if dtb_addr == 0 {
        return devices;
    }
    
    // Validate magic
    let magic = read_be32(dtb_addr);
    if magic != FDT_MAGIC {
        return devices;
    }
    
    // Read header offsets
    let struct_off = read_be32(dtb_addr + 8) as usize;
    let strings_off = read_be32(dtb_addr + 12) as usize;
    
    let struct_base = dtb_addr + struct_off;
    let strings_base = dtb_addr + strings_off;
    
    // Parse structure block
    let mut pos = struct_base;
    let mut current_node = DeviceNode {
        name: String::new(),
        compatible: String::new(),
        reg_base: 0,
        reg_size: 0,
        interrupts: None,
    };
    let mut in_soc = false;
    let mut depth = 0u32;
    let mut soc_depth = 0u32;
    
    // Track address/size cells (default: 2 each for 64-bit)
    let mut address_cells: u32 = 2;
    let mut size_cells: u32 = 2;
    
    loop {
        let token = read_be32(pos);
        pos += 4;
        
        match token {
            FDT_BEGIN_NODE => {
                depth += 1;
                let (name, consumed) = read_node_name(pos);
                pos += consumed;
                
                // Check if entering /soc
                if depth == 2 && name == "soc" {
                    in_soc = true;
                    soc_depth = depth;
                }
                
                // Start new device node if in /soc
                if in_soc && depth > soc_depth {
                    current_node = DeviceNode {
                        name: name.clone(),
                        compatible: String::new(),
                        reg_base: 0,
                        reg_size: 0,
                        interrupts: None,
                    };
                }
            }
            FDT_END_NODE => {
                // Save device if it has both name and compatible
                if in_soc && depth > soc_depth && !current_node.compatible.is_empty() {
                    devices.push(current_node.clone());
                }
                
                if depth == soc_depth {
                    in_soc = false;
                }
                depth = depth.saturating_sub(1);
            }
            FDT_PROP => {
                let len = read_be32(pos) as usize;
                pos += 4;
                let name_off = read_be32(pos);
                pos += 4;
                
                let prop_name = read_string(strings_base, name_off);
                let data_addr = pos;
                
                // Parse known properties
                if in_soc && depth > soc_depth {
                    match prop_name.as_str() {
                        "compatible" => {
                            // Read first string from compatible (may be stringlist)
                            let (compat, _) = read_node_name(data_addr);
                            current_node.compatible = compat;
                        }
                        "reg" => {
                            // Parse reg based on address-cells and size-cells
                            if address_cells == 2 && len >= 16 {
                                // 64-bit address
                                let addr_hi = read_be32(data_addr) as u64;
                                let addr_lo = read_be32(data_addr + 4) as u64;
                                current_node.reg_base = (addr_hi << 32) | addr_lo;
                                
                                if size_cells == 2 && len >= 16 {
                                    let size_hi = read_be32(data_addr + 8) as u64;
                                    let size_lo = read_be32(data_addr + 12) as u64;
                                    current_node.reg_size = (size_hi << 32) | size_lo;
                                } else if size_cells == 1 && len >= 12 {
                                    current_node.reg_size = read_be32(data_addr + 8) as u64;
                                }
                            } else if address_cells == 1 && len >= 8 {
                                // 32-bit address
                                current_node.reg_base = read_be32(data_addr) as u64;
                                if size_cells == 1 && len >= 8 {
                                    current_node.reg_size = read_be32(data_addr + 4) as u64;
                                }
                            }
                        }
                        "interrupts" => {
                            if len >= 4 {
                                current_node.interrupts = Some(read_be32(data_addr));
                            }
                        }
                        "#address-cells" => {
                            if len >= 4 {
                                address_cells = read_be32(data_addr);
                            }
                        }
                        "#size-cells" => {
                            if len >= 4 {
                                size_cells = read_be32(data_addr);
                            }
                        }
                        _ => {}
                    }
                } else if depth == 2 {
                    // Track cells at /soc level
                    match prop_name.as_str() {
                        "#address-cells" => {
                            if len >= 4 {
                                address_cells = read_be32(data_addr);
                            }
                        }
                        "#size-cells" => {
                            if len >= 4 {
                                size_cells = read_be32(data_addr);
                            }
                        }
                        _ => {}
                    }
                }
                
                // Skip property data (aligned to 4 bytes)
                pos += (len + 3) & !3;
            }
            FDT_NOP => {
                // Skip
            }
            FDT_END => {
                break;
            }
            _ => {
                // Unknown token, stop parsing
                break;
            }
        }
    }
    
    devices
}

/// Find devices by compatible string
pub fn find_by_compatible(dtb_addr: usize, compat: &str) -> Vec<DeviceNode> {
    parse_devices(dtb_addr)
        .into_iter()
        .filter(|d| d.compatible == compat || d.compatible.starts_with(compat))
        .collect()
}
