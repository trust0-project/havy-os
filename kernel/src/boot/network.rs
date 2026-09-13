use alloc::format;

use crate::boot::console::{print_info, print_line, print_section, print_status};
use crate::device::NetworkDevice;
use crate::lock::utils::NET_STATE;
use crate::net;
#[cfg(feature = "d1")]
use crate::platform;

pub fn init_network() {
    print_section("NETWORK SUBSYSTEM");

    #[cfg(feature = "d1")]
    {
        print_info("Probing", "D1 EMAC...");
        if platform::d1_emac::probe() {
            print_info("D1 EMAC PHY", "detected at 0x0450_0000");
            match platform::d1_emac::create_device() {
                Ok(device) => {
                    let mac = device.mac_address();
                    let mac_str = format!(
                        "{:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
                        mac[0], mac[1], mac[2], mac[3], mac[4], mac[5]
                    );
                    print_info("D1 EMAC MAC", &mac_str);
                    match net::NetState::new(crate::lock::state::net::Nic::Emac(device)) {
                        Ok(state) => {
                            *NET_STATE.lock() = Some(state);
                            print_status("D1 EMAC network initialized (smoltcp + DHCP)", true);
                        }
                        Err(e) => print_status(&format!("D1 network init failed: {}", e), false),
                    }
                }
                Err(_) => print_status("D1 EMAC device creation failed", false),
            }
        } else {
            print_line("    No D1 EMAC detected");
            print_line("    Network features will be unavailable");
        }
    }

    #[cfg(not(feature = "d1"))]
    {
        print_info("Probing", "virtio-net...");
        match crate::device::virtio_net::create_device() {
            Ok(device) => {
                let mac = device.mac_address();
                let mac_str = format!(
                    "{:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
                    mac[0], mac[1], mac[2], mac[3], mac[4], mac[5]
                );
                print_info("virtio-net MAC", &mac_str);
                match net::NetState::new(crate::lock::state::net::Nic::Virtio(device)) {
                    Ok(state) => {
                        *NET_STATE.lock() = Some(state);
                        print_status("virtio-net initialized (smoltcp + DHCP)", true);
                    }
                    Err(e) => print_status(&format!("virtio-net init failed: {}", e), false),
                }
            }
            Err(_) => {
                print_line("    No virtio-net device");
                print_line("    Network features will be unavailable");
            }
        }
    }
}
