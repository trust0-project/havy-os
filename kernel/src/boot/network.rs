use alloc::format;

use crate::boot::console::{print_info, print_line, print_section, print_status};
use crate::device::NetworkDevice;
use crate::lock::utils::NET_STATE;
use crate::net;
use crate::platform;

/// Initialize the network stack
pub fn init_network() {
    print_section("NETWORK SUBSYSTEM");
    print_info("Probing", "D1 EMAC...");
    
    if platform::d1_emac::probe() {
        print_info("D1 EMAC PHY", "detected at 0x0450_0000");
        
        // Create D1 EMAC device
        match platform::d1_emac::create_device() {
            Ok(device) => {
                // Format MAC address
                let mac = device.mac_address();
                let mac_str = format!(
                    "{:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
                    mac[0], mac[1], mac[2], mac[3], mac[4], mac[5]
                );
                print_info("D1 EMAC MAC", &mac_str);
                
                // Create NetState (stored in unified NET_STATE)
                match net::NetState::new(device) {
                    Ok(state) => {
                        let mut net_guard = NET_STATE.lock();
                        *net_guard = Some(state);
                        print_status("D1 EMAC network initialized (smoltcp)", true);
                    }
                    Err(e) => {
                        print_status(&format!("D1 network init failed: {}", e), false);
                    }
                }
            }
            Err(_) => {
                print_status("D1 EMAC device creation failed", false);
            }
        }
    } else {
        print_line("    No D1 EMAC detected");
        print_line("    Network features will be unavailable");
    }
}
