//! FDT (Flattened Device Tree) Parser
//!
//! Parses the Device Tree Blob (DTB) to extract device information.
//! This allows the kernel to discover devices dynamically rather than
//! relying on hardcoded addresses.

use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::ptr::read_volatile;
use crate::services::klogd::klog_debug;

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

/// Count CPU nodes in a bounded walk of the structure block.
///
/// `device_type = "cpu"` is the normative discriminator.  Node names are
/// retained as a compatibility fallback for small synthetic DTBs.
pub fn count_cpus(dtb_addr: usize) -> usize {
    if dtb_addr == 0 {
        return 0;
    }
    let magic = read_be32(dtb_addr);
    if magic != FDT_MAGIC {
        return 0;
    }

    let struct_off = read_be32(dtb_addr + 8) as usize;
    let strings_off = read_be32(dtb_addr + 12) as usize;
    let struct_size = read_be32(dtb_addr + 36) as usize;
    let struct_base = dtb_addr + struct_off;
    let strings_base = dtb_addr + strings_off;
    let struct_end = match struct_base.checked_add(struct_size) {
        Some(end) => end,
        None => return 0,
    };

    let mut pos = struct_base;
    let mut count = 0usize;
    let mut name = String::new();
    let mut compatible = String::new();
    let mut device_type = String::new();
    let mut stack: Vec<(String, String, String)> = Vec::new();

    while pos.checked_add(4).is_some_and(|next| next <= struct_end) {
        let token = read_be32(pos);
        pos += 4;

        match token {
            FDT_BEGIN_NODE => {
                stack.push((name, compatible, device_type));
                let (n, consumed) = read_node_name(pos);
                if pos.checked_add(consumed).is_none_or(|next| next > struct_end) {
                    break;
                }
                pos += consumed;
                name = n;
                compatible = String::new();
                device_type = String::new();
            }
            FDT_END_NODE => {
                let is_cpu = device_type == "cpu"
                    || name == "cpu"
                    || name.starts_with("cpu@");
                if is_cpu {
                    klog_debug(
                        "dtb",
                        &alloc::format!("CPU node: {} ({})", name, compatible),
                    );
                    crate::device::uart::write_line(
                        &alloc::format!("[dtb] CPU node: {} ({})", name, compatible),
                    );
                    count += 1;
                }
                if let Some((n, c, d)) = stack.pop() {
                    name = n;
                    compatible = c;
                    device_type = d;
                }
            }
            FDT_PROP => {
                if pos.checked_add(8).is_none_or(|next| next > struct_end) {
                    break;
                }
                let len = read_be32(pos) as usize;
                pos += 4;
                let name_off = read_be32(pos);
                pos += 4;
                let padded_len = match len.checked_add(3) {
                    Some(value) => value & !3,
                    None => break,
                };
                if pos.checked_add(padded_len).is_none_or(|next| next > struct_end) {
                    break;
                }
                let prop_name = read_string(strings_base, name_off);
                if prop_name == "compatible" || prop_name == "device_type" {
                    let mut bytes = Vec::new();
                    for offset in 0..len.min(256) {
                        let byte = unsafe { read_volatile((pos + offset) as *const u8) };
                        if byte == 0 {
                            break;
                        }
                        bytes.push(byte);
                    }
                    let value = String::from_utf8(bytes).unwrap_or_default();
                    if prop_name == "compatible" {
                        compatible = value;
                    } else {
                        device_type = value;
                    }
                }
                pos += padded_len;
            }
            FDT_NOP => {}
            FDT_END => break,
            _ => break,
        }
    }

    crate::device::uart::write_line(&alloc::format!("[dtb] CPU count: {}", count));
    count
}

/// Find devices by compatible string
pub fn find_by_compatible(dtb_addr: usize, compat: &str) -> Vec<DeviceNode> {
    parse_devices(dtb_addr)
        .into_iter()
        .filter(|d| d.compatible == compat || d.compatible.starts_with(compat))
        .collect()
}

/// Host-advertised HDL mailbox region.
///
/// Parsed from `/reserved-memory/hdl-mailbox@*` (`compatible = "havy,hdl-mailbox"`)
/// or `/chosen` properties `havy,hdl-mailbox` + `havy,hdl-abi-major`.
#[derive(Clone, Copy, Debug)]
pub struct HdlMailbox {
    pub base: u64,
    pub size: u64,
    pub abi_major: u8,
    pub abi_minor: u8,
}

struct HdlWalkFrame {
    name: String,
    compatible: String,
    reg_base: u64,
    reg_size: u64,
    abi_major: Option<u8>,
    abi_minor: Option<u8>,
    addr_cells: u32,
    size_cells: u32,
    child_addr_cells: u32,
    child_size_cells: u32,
}

fn parse_reg_cells(data_addr: usize, len: usize, addr_cells: u32, size_cells: u32) -> (u64, u64) {
    let addr_bytes = (addr_cells as usize).saturating_mul(4);
    let size_bytes = (size_cells as usize).saturating_mul(4);
    if len < addr_bytes.saturating_add(size_bytes) {
        return (0, 0);
    }
    let addr = read_cells(data_addr, addr_cells);
    let size = read_cells(data_addr + addr_bytes, size_cells);
    (addr, size)
}

fn parse_reg_by_len(data_addr: usize, len: usize) -> (u64, u64) {
    match len {
        16 => parse_reg_cells(data_addr, len, 2, 2),
        12 => parse_reg_cells(data_addr, len, 2, 1),
        8 => parse_reg_cells(data_addr, len, 1, 1),
        _ => (0, 0),
    }
}

fn read_cells(addr: usize, cells: u32) -> u64 {
    match cells {
        1 => read_be32(addr) as u64,
        2 => {
            let hi = read_be32(addr) as u64;
            let lo = read_be32(addr + 4) as u64;
            (hi << 32) | lo
        }
        _ => 0,
    }
}

fn parse_abi_cell(data_addr: usize, len: usize) -> Option<u8> {
    if len >= 4 {
        Some(read_be32(data_addr) as u8)
    } else if len >= 1 {
        Some(unsafe { read_volatile(data_addr as *const u8) })
    } else {
        None
    }
}

fn is_hdl_node_name(name: &str) -> bool {
    name == "hdl-mailbox" || name.starts_with("hdl-mailbox@")
}

fn is_reserved_memory_name(name: &str) -> bool {
    name == "reserved-memory" || name.starts_with("reserved-memory@")
}

/// Walk the DTB for the private Havy HDL mailbox capability.
pub fn find_hdl_mailbox(dtb_addr: usize) -> Option<HdlMailbox> {
    if dtb_addr == 0 {
        return None;
    }
    let magic = read_be32(dtb_addr);
    if magic != FDT_MAGIC {
        return None;
    }

    let struct_off = read_be32(dtb_addr + 8) as usize;
    let strings_off = read_be32(dtb_addr + 12) as usize;
    let struct_size = read_be32(dtb_addr + 36) as usize;
    let struct_base = dtb_addr + struct_off;
    let strings_base = dtb_addr + strings_off;
    let struct_end = struct_base.checked_add(struct_size)?;

    let mut pos = struct_base;
    let mut depth = 0u32;
    let mut in_chosen = false;
    let mut in_reserved = false;
    let mut stack: Vec<HdlWalkFrame> = Vec::new();
    let mut current = HdlWalkFrame {
        name: String::new(),
        compatible: String::new(),
        reg_base: 0,
        reg_size: 0,
        abi_major: None,
        abi_minor: None,
        addr_cells: 2,
        size_cells: 1,
        child_addr_cells: 2,
        child_size_cells: 1,
    };

    struct Found {
        base: u64,
        size: u64,
        abi_major: Option<u8>,
        abi_minor: Option<u8>,
    }
    let mut from_compat: Option<Found> = None;
    let mut from_reserved: Option<Found> = None;
    let mut chosen_reg: Option<(u64, u64)> = None;
    let mut chosen_major: Option<u8> = None;
    let mut chosen_minor: Option<u8> = None;

    while pos.checked_add(4).is_some_and(|next| next <= struct_end) {
        let token = read_be32(pos);
        pos += 4;
        match token {
            FDT_BEGIN_NODE => {
                depth += 1;
                let (name, consumed) = read_node_name(pos);
                if pos.checked_add(consumed).is_none_or(|next| next > struct_end) {
                    break;
                }
                pos += consumed;
                stack.push(current);
                let parent = stack.last().unwrap();
                current = HdlWalkFrame {
                    name: name.clone(),
                    compatible: String::new(),
                    reg_base: 0,
                    reg_size: 0,
                    abi_major: None,
                    abi_minor: None,
                    addr_cells: parent.child_addr_cells,
                    size_cells: parent.child_size_cells,
                    child_addr_cells: parent.child_addr_cells,
                    child_size_cells: parent.child_size_cells,
                };
                if depth == 2 && name == "chosen" {
                    in_chosen = true;
                }
                if depth == 2 && is_reserved_memory_name(&name) {
                    in_reserved = true;
                }
            }
            FDT_END_NODE => {
                let compat_hdl = current.compatible == "havy,hdl-mailbox"
                    || current.compatible.starts_with("havy,hdl-mailbox");
                if (compat_hdl || (in_reserved && is_hdl_node_name(&current.name)))
                    && current.reg_size != 0
                {
                    let found = Found {
                        base: current.reg_base,
                        size: current.reg_size,
                        abi_major: current.abi_major,
                        abi_minor: current.abi_minor,
                    };
                    if compat_hdl {
                        from_compat = Some(found);
                    } else if from_reserved.is_none() {
                        from_reserved = Some(found);
                    }
                }
                if depth == 2 {
                    in_chosen = false;
                    in_reserved = false;
                }
                depth = depth.saturating_sub(1);
                if let Some(prev) = stack.pop() {
                    current = prev;
                }
            }
            FDT_PROP => {
                if pos.checked_add(8).is_none_or(|next| next > struct_end) {
                    break;
                }
                let len = read_be32(pos) as usize;
                pos += 4;
                let name_off = read_be32(pos);
                pos += 4;
                let padded_len = match len.checked_add(3) {
                    Some(value) => value & !3,
                    None => break,
                };
                if pos.checked_add(padded_len).is_none_or(|next| next > struct_end) {
                    break;
                }
                let prop_name = read_string(strings_base, name_off);
                let data_addr = pos;
                match prop_name.as_str() {
                    "#address-cells" if len >= 4 => {
                        current.child_addr_cells = read_be32(data_addr);
                    }
                    "#size-cells" if len >= 4 => {
                        current.child_size_cells = read_be32(data_addr);
                    }
                    "reg" => {
                        let (base, size) =
                            parse_reg_cells(data_addr, len, current.addr_cells, current.size_cells);
                        current.reg_base = base;
                        current.reg_size = size;
                    }
                    "compatible" => {
                        let (compat, _) = read_node_name(data_addr);
                        current.compatible = compat;
                    }
                    "havy,abi-major" | "havy,hdl-abi-major" => {
                        if let Some(v) = parse_abi_cell(data_addr, len) {
                            current.abi_major = Some(v);
                            if in_chosen {
                                chosen_major = Some(v);
                            }
                        }
                    }
                    "havy,abi-minor" | "havy,hdl-abi-minor" => {
                        if let Some(v) = parse_abi_cell(data_addr, len) {
                            current.abi_minor = Some(v);
                            if in_chosen {
                                chosen_minor = Some(v);
                            }
                        }
                    }
                    "havy,hdl-mailbox" if in_chosen => {
                        let (base, size) = parse_reg_by_len(data_addr, len);
                        if size != 0 {
                            chosen_reg = Some((base, size));
                        }
                    }
                    _ => {}
                }
                pos += padded_len;
            }
            FDT_NOP => {}
            FDT_END => break,
            _ => break,
        }
    }

    let finish = |f: Found| -> HdlMailbox {
        HdlMailbox {
            base: f.base,
            size: f.size,
            abi_major: f.abi_major.or(chosen_major).unwrap_or(1),
            abi_minor: f.abi_minor.or(chosen_minor).unwrap_or(0),
        }
    };

    if let Some(f) = from_compat {
        return Some(finish(f));
    }
    if let Some(f) = from_reserved {
        return Some(finish(f));
    }
    if let Some((base, size)) = chosen_reg {
        return Some(HdlMailbox {
            base,
            size,
            abi_major: chosen_major.unwrap_or(1),
            abi_minor: chosen_minor.unwrap_or(0),
        });
    }
    None
}
