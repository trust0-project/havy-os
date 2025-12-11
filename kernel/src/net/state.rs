//! Network state management.
//!
//! This module contains the NetState struct that manages the entire network stack,
//! including sockets, interfaces, and device operations.

use alloc::collections::VecDeque;
use alloc::vec;
use alloc::vec::Vec;

use smoltcp::iface::{Interface, SocketHandle, SocketSet, Config};
use smoltcp::socket::{icmp, tcp, udp};
use smoltcp::time::Instant;
use smoltcp::wire::{EthernetAddress, HardwareAddress, IpAddress, IpCidr, IpEndpoint, Ipv4Address};

use crate::virtio_net::VirtioNet;
use super::config::*;
use super::buffers::*;
use super::server::*;
use super::device::DeviceWrapper;
use super::patching::{reset_client_patching_state, reset_server_patching_for_port};

/// Pending loopback ping reply
struct LoopbackReply {
    from: Ipv4Address,
    ident: u16,
    seq: u16,
}

/// Cached ARP entry
struct ArpCache {
    ip: [u8; 4],
    mac: [u8; 6],
}

/// Global network state
pub struct NetState {
    device: VirtioNet,
    iface: Interface,
    sockets: SocketSet<'static>,
    icmp_handle: SocketHandle,
    udp_handle: SocketHandle,
    tcp_handle: SocketHandle,
    arp_cache: Option<ArpCache>,
    /// Pending loopback ping replies (delivered on next poll)
    loopback_replies: VecDeque<LoopbackReply>,
    /// Server socket manager for TCP listen/accept
    server_sockets: TcpServerManager,
    /// Whether IP has been assigned from relay
    ip_assigned: bool,
}

impl NetState {
    /// Initialize the network stack
    /// Note: After storing this in a static, call finalize() to complete RX buffer setup!
    /// Returns Err if no IP is assigned by the relay (networking will be disabled).
    pub fn new(mut device: VirtioNet) -> Result<Self, &'static str> {
        // Initialize the VirtIO device (phase 1 - configures queues but doesn't populate RX)
        device.init()?;

        // --- IP ADDRESS DISCOVERY ---
        // Check once for IP, but don't block if not available yet
        // IP will be checked again in poll() and interface updated when assigned
        let mut my_ip = DEFAULT_IP_ADDR; // Fallback default
        let mut got_ip = false;

        crate::uart::write_str("    \x1b[0;90m+-\x1b[0m Checking for IP assignment...");
        // Quick initial check (no blocking)
        if let Some(ip_bytes) = device.get_config_ip() {
            my_ip = Ipv4Address::new(ip_bytes[0], ip_bytes[1], ip_bytes[2], ip_bytes[3]);
            got_ip = true;
            crate::uart::write_line(" \x1b[1;32m[OK]\x1b[0m");
        } else {
            crate::uart::write_line(" \x1b[1;33m[PENDING]\x1b[0m");
            crate::uart::write_line("    \x1b[0;90m    +- Network will initialize in background\x1b[0m");
        }

        // Save to global for other modules to use
        unsafe {
            MY_IP_ADDR = my_ip;
        }

        let mac = device.mac;
        let hw_addr = HardwareAddress::Ethernet(EthernetAddress(mac));

        // Create interface config with a random seed for TCP ISN generation
        // Use a combination of MAC address, IP, and time to create a unique seed per VM
        let seed = {
            let mac_part = (mac[0] as u64) << 40 
                         | (mac[1] as u64) << 32 
                         | (mac[2] as u64) << 24 
                         | (mac[3] as u64) << 16 
                         | (mac[4] as u64) << 8 
                         | (mac[5] as u64);
            let ip_octets = my_ip.octets();
            let ip_part = (ip_octets[0] as u64) << 24 
                        | (ip_octets[1] as u64) << 16 
                        | (ip_octets[2] as u64) << 8 
                        | (ip_octets[3] as u64);
            let time_part = crate::get_time_ms() as u64;
            mac_part ^ (ip_part << 16) ^ time_part
        };
        
        crate::uart::write_str("[net] smoltcp random_seed: ");
        crate::uart::write_u64(seed);
        crate::uart::write_line("");
        
        let mut config = Config::new(hw_addr);
        config.random_seed = seed;

        // Create the interface
        let mut iface = Interface::new(
            config,
            &mut DeviceWrapper(&mut device),
            Instant::from_millis(0),
        );

        // Configure IP address using the dynamic IP
        iface.update_ip_addrs(|addrs| {
            addrs
                .push(IpCidr::new(
                    IpAddress::Ipv4(my_ip),
                    PREFIX_LEN,
                ))
                .ok();
        });

        // Set default gateway
        iface.routes_mut().add_default_ipv4_route(GATEWAY).ok();

        // Create socket set with static storage
        let sockets = unsafe { SocketSet::new(&mut SOCKET_STORAGE[..]) };

        // Create ICMP socket for ping
        let icmp_rx_buffer =
            unsafe { icmp::PacketBuffer::new(&mut ICMP_RX_META[..], &mut ICMP_RX_DATA[..]) };
        let icmp_tx_buffer =
            unsafe { icmp::PacketBuffer::new(&mut ICMP_TX_META[..], &mut ICMP_TX_DATA[..]) };
        let mut icmp_socket = icmp::Socket::new(icmp_rx_buffer, icmp_tx_buffer);

        // Bind the ICMP socket to receive echo replies for our identifier
        icmp_socket.bind(icmp::Endpoint::Ident(ICMP_IDENT)).ok();

        // Create UDP socket for DNS queries
        let udp_rx_buffer =
            unsafe { udp::PacketBuffer::new(&mut UDP_RX_META[..], &mut UDP_RX_DATA[..]) };
        let udp_tx_buffer =
            unsafe { udp::PacketBuffer::new(&mut UDP_TX_META[..], &mut UDP_TX_DATA[..]) };
        let mut udp_socket = udp::Socket::new(udp_rx_buffer, udp_tx_buffer);

        // Bind UDP socket to local port for DNS
        udp_socket.bind(DNS_LOCAL_PORT).ok();

        // Create TCP socket for client connections (used by telnet, wget, etc.)
        let tcp_rx_buffer = unsafe { tcp::SocketBuffer::new(&mut TCP_RX_DATA[..]) };
        let tcp_tx_buffer = unsafe { tcp::SocketBuffer::new(&mut TCP_TX_DATA[..]) };
        let tcp_socket = tcp::Socket::new(tcp_rx_buffer, tcp_tx_buffer);

        let mut state = NetState {
            device,
            iface,
            sockets,
            icmp_handle: SocketHandle::default(),
            udp_handle: SocketHandle::default(),
            tcp_handle: SocketHandle::default(),
            arp_cache: None,
            loopback_replies: VecDeque::new(),
            server_sockets: TcpServerManager::new(),
            ip_assigned: got_ip,
        };

        state.icmp_handle = state.sockets.add(icmp_socket);
        state.udp_handle = state.sockets.add(udp_socket);
        state.tcp_handle = state.sockets.add(tcp_socket);

        Ok(state)
    }

    /// Finalize initialization (must be called AFTER the NetState is in its final memory location)
    pub fn finalize(&mut self) {
        self.device.finalize_init();
    }

    /// Poll the network stack (call frequently)
    pub fn poll(&mut self, timestamp_ms: i64) {
        let timestamp = Instant::from_millis(timestamp_ms);

        // Check for background IP assignment
        if !self.ip_assigned {
            if let Some(ip_bytes) = self.device.get_config_ip() {
                let new_ip = Ipv4Address::new(ip_bytes[0], ip_bytes[1], ip_bytes[2], ip_bytes[3]);
                
                // Update interface IP
                self.iface.update_ip_addrs(|addrs| {
                    addrs.clear();
                    addrs.push(IpCidr::new(IpAddress::Ipv4(new_ip), PREFIX_LEN)).ok();
                });
                
                // Update global IP
                unsafe { MY_IP_ADDR = new_ip; }
                
                self.ip_assigned = true;
                crate::uart::write_str("\x1b[1;32m[NET]\x1b[0m IP assigned: ");
                let mut ip_buf = [0u8; 16];
                let len = crate::net::format_ipv4(new_ip, &mut ip_buf);
                crate::uart::write_bytes(&ip_buf[..len]);
                crate::uart::write_line("");
            }
        }

        // Poll the device
        self.device.poll();

        self.iface.poll(
            timestamp,
            &mut DeviceWrapper(&mut self.device),
            &mut self.sockets,
        );
    }

    /// Send a raw ARP request
    fn send_arp_request(&mut self, target_ip: [u8; 4]) -> Result<(), &'static str> {
        let my_ip = get_my_ip();
        let mut frame = [0u8; 42]; // 14 (eth) + 28 (arp)

        // Ethernet header
        frame[0..6].copy_from_slice(&[0xff, 0xff, 0xff, 0xff, 0xff, 0xff]); // dst = broadcast
        frame[6..12].copy_from_slice(&self.device.mac); // src = our MAC
        frame[12..14].copy_from_slice(&[0x08, 0x06]); // ethertype = ARP

        // ARP header
        frame[14..16].copy_from_slice(&[0x00, 0x01]); // hardware type = ethernet
        frame[16..18].copy_from_slice(&[0x08, 0x00]); // protocol type = IPv4
        frame[18] = 6; // hardware addr len
        frame[19] = 4; // protocol addr len
        frame[20..22].copy_from_slice(&[0x00, 0x01]); // operation = request
        frame[22..28].copy_from_slice(&self.device.mac); // sender hardware addr
        frame[28..32].copy_from_slice(&my_ip.octets()); // sender protocol addr
        frame[32..38].copy_from_slice(&[0x00; 6]); // target hardware addr (unknown)
        frame[38..42].copy_from_slice(&target_ip); // target protocol addr

        self.device.send(&frame)
    }

    /// Compute ICMP checksum
    fn icmp_checksum(data: &[u8]) -> u16 {
        let mut sum: u32 = 0;
        let mut i = 0;
        while i + 1 < data.len() {
            sum += u16::from_be_bytes([data[i], data[i + 1]]) as u32;
            i += 2;
        }
        if i < data.len() {
            sum += (data[i] as u32) << 8;
        }
        while sum > 0xFFFF {
            sum = (sum & 0xFFFF) + (sum >> 16);
        }
        !(sum as u16)
    }

    /// Resolve MAC address for an IP via ARP (with caching)
    fn resolve_mac(&mut self, target_ip: [u8; 4]) -> Option<[u8; 6]> {
        // Check cache first
        if let Some(ref cache) = self.arp_cache {
            if cache.ip == target_ip {
                return Some(cache.mac);
            }
        }

        // Try multiple times with increasing waits
        for attempt in 0..5 {
            // Send ARP request
            if attempt == 0 || attempt == 2 {
                // Resend ARP on first attempt and after some retries
                if self.send_arp_request(target_ip).is_err() {
                    continue;
                }
            }

            // Wait with increasing delay (100k, 200k, 400k, 800k, 1600k spins)
            let wait_cycles = 100_000 << attempt;
            for _ in 0..wait_cycles {
                core::hint::spin_loop();
            }
            self.device.poll();

            // Check for ARP reply
            if let Some((desc_idx, data)) = self.device.recv_with_desc() {
                if data.len() >= 42 && data[12] == 0x08 && data[13] == 0x06 {
                    // ARP packet - extract sender MAC
                    let mut mac = [0u8; 6];
                    mac.copy_from_slice(&data[22..28]);

                    // Recycle the buffer
                    self.device.recycle_rx(desc_idx);

                    // Cache the result
                    self.arp_cache = Some(ArpCache { ip: target_ip, mac });

                    return Some(mac);
                }
                // Not an ARP reply, recycle and keep trying
                self.device.recycle_rx(desc_idx);
            }
        }

        None
    }

    /// Check if an address is a loopback address (127.x.x.x)
    fn is_loopback(addr: &Ipv4Address) -> bool {
        addr.octets()[0] == 127
    }

    /// Check if an address is our own IP
    fn is_self(addr: &Ipv4Address) -> bool {
        let my_ip = get_my_ip();
        addr.octets() == my_ip.octets()
    }

    /// Check if an address is on the local subnet (10.0.2.x/24)
    fn is_on_local_subnet(addr: &Ipv4Address) -> bool {
        let my_ip = get_my_ip();
        let addr_o = addr.octets();
        let my_o = my_ip.octets();
        addr_o[0] == my_o[0] && addr_o[1] == my_o[1] && addr_o[2] == my_o[2]
    }

    /// Send an ICMP echo request (ping) - directly via VirtIO or loopback
    pub fn send_ping(
        &mut self,
        target: Ipv4Address,
        seq: u16,
        _timestamp_ms: i64,
    ) -> Result<(), &'static str> {
        // Handle loopback addresses (127.x.x.x) and self-ping locally
        if Self::is_loopback(&target) || Self::is_self(&target) {
            // Queue an immediate reply for loopback
            self.loopback_replies.push_back(LoopbackReply {
                from: target,
                ident: ICMP_IDENT,
                seq,
            });
            return Ok(());
        }

        let target_bytes = target.octets();

        // For external IPs (not on local subnet), route through gateway
        let next_hop = if Self::is_on_local_subnet(&target) {
            target_bytes
        } else {
            GATEWAY.octets() // Use gateway for external destinations
        };

        // Resolve MAC address for the next hop (gateway or direct target)
        let dst_mac = self
            .resolve_mac(next_hop)
            .ok_or("Failed to resolve MAC address")?;

        // Build Ethernet + IP + ICMP packet
        let icmp_data = b"RISCV_PING";
        let icmp_len = 8 + icmp_data.len(); // ICMP header (8) + data
        let ip_len = 20 + icmp_len; // IP header (20) + ICMP
        let frame_len = 14 + ip_len; // Ethernet header (14) + IP

        let mut frame = vec![0u8; frame_len];

        // Ethernet header
        frame[0..6].copy_from_slice(&dst_mac); // dst MAC
        frame[6..12].copy_from_slice(&self.device.mac); // src MAC
        frame[12..14].copy_from_slice(&[0x08, 0x00]); // ethertype = IPv4

        // IP header
        let my_ip = get_my_ip();
        frame[14] = 0x45; // version + IHL
        frame[15] = 0; // TOS
        frame[16..18].copy_from_slice(&(ip_len as u16).to_be_bytes()); // total length
        frame[18..20].copy_from_slice(&ICMP_IDENT.to_be_bytes()); // identification
        frame[20..22].copy_from_slice(&[0x00, 0x00]); // flags + fragment
        frame[22] = 64; // TTL
        frame[23] = 1; // protocol = ICMP
        frame[24..26].copy_from_slice(&[0x00, 0x00]); // checksum (fill later)
        frame[26..30].copy_from_slice(&my_ip.octets()); // src IP
        frame[30..34].copy_from_slice(&target_bytes); // dst IP

        // IP checksum
        let ip_header = &frame[14..34];
        let mut sum: u32 = 0;
        for i in (0..20).step_by(2) {
            sum += u16::from_be_bytes([ip_header[i], ip_header[i + 1]]) as u32;
        }
        while sum > 0xFFFF {
            sum = (sum & 0xFFFF) + (sum >> 16);
        }
        let ip_cksum = !(sum as u16);
        frame[24..26].copy_from_slice(&ip_cksum.to_be_bytes());

        // ICMP header
        frame[34] = 8; // type = echo request
        frame[35] = 0; // code
        frame[36..38].copy_from_slice(&[0x00, 0x00]); // checksum (fill later)
        frame[38..40].copy_from_slice(&ICMP_IDENT.to_be_bytes()); // identifier
        frame[40..42].copy_from_slice(&seq.to_be_bytes()); // sequence
        frame[42..].copy_from_slice(icmp_data); // data

        // ICMP checksum
        let icmp_cksum = Self::icmp_checksum(&frame[34..]);
        frame[36..38].copy_from_slice(&icmp_cksum.to_be_bytes());

        // Send the ICMP request
        self.device.send(&frame)?;

        Ok(())
    }

    /// Get MAC address
    #[allow(dead_code)]
    pub fn mac(&self) -> [u8; 6] {
        self.device.mac
    }

    /// Get MAC address as string
    pub fn mac_str(&self) -> [u8; 17] {
        self.device.mac_str()
    }

    /// Check for ICMP echo reply by directly examining received packets
    /// Also handles loopback replies
    pub fn check_ping_reply(&mut self) -> Option<(Ipv4Address, u16, u16)> {
        // First check for loopback replies (highest priority, instant)
        if let Some(reply) = self.loopback_replies.pop_front() {
            return Some((reply.from, reply.ident, reply.seq));
        }

        // Poll the VirtIO device for incoming packets
        self.device.poll();

        // Check for received packet
        if let Some((desc_idx, data)) = self.device.recv_with_desc() {
            // Must be at least: eth(14) + ip(20) + icmp(8) = 42 bytes
            if data.len() >= 42 {
                // Check for IPv4 (ethertype 0x0800)
                if data[12] == 0x08 && data[13] == 0x00 {
                    // Check IP protocol is ICMP (1)
                    if data[23] == 1 {
                        // Check ICMP type is echo reply (0)
                        if data[34] == 0 {
                            // Parse ICMP echo reply
                            let ident = u16::from_be_bytes([data[38], data[39]]);
                            let seq = u16::from_be_bytes([data[40], data[41]]);
                            let src_ip = Ipv4Address::new(data[26], data[27], data[28], data[29]);

                            // Recycle the buffer
                            self.device.recycle_rx(desc_idx);

                            // Check if this is for our identifier
                            if ident == ICMP_IDENT {
                                return Some((src_ip, ident, seq));
                            }
                        }
                    }
                }
            }

            // Recycle buffer if we didn't return earlier
            self.device.recycle_rx(desc_idx);
        }
        None
    }

    /// Send a UDP packet to the specified destination
    pub fn udp_send(
        &mut self,
        dest_ip: Ipv4Address,
        dest_port: u16,
        data: &[u8],
        timestamp_ms: i64,
    ) -> Result<(), &'static str> {
        let timestamp = Instant::from_millis(timestamp_ms);

        // Get the UDP socket
        let socket = self.sockets.get_mut::<udp::Socket>(self.udp_handle);

        // Create destination endpoint
        let endpoint = IpEndpoint::new(IpAddress::Ipv4(dest_ip), dest_port);

        // Check if socket can send
        if !socket.can_send() {
            return Err("UDP socket cannot send");
        }

        // Send the data
        socket
            .send_slice(data, endpoint)
            .map_err(|_| "Failed to send UDP packet")?;

        // Poll to actually transmit
        self.iface.poll(
            timestamp,
            &mut DeviceWrapper(&mut self.device),
            &mut self.sockets,
        );

        Ok(())
    }

    /// Receive a UDP packet (non-blocking)
    /// Returns (source_ip, source_port, data) if a packet is available
    pub fn udp_recv(
        &mut self,
        buf: &mut [u8],
        timestamp_ms: i64,
    ) -> Option<(Ipv4Address, u16, usize)> {
        let timestamp = Instant::from_millis(timestamp_ms);

        // Poll to receive any pending packets
        self.iface.poll(
            timestamp,
            &mut DeviceWrapper(&mut self.device),
            &mut self.sockets,
        );

        // Get the UDP socket
        let socket = self.sockets.get_mut::<udp::Socket>(self.udp_handle);

        // Check if we can receive
        if !socket.can_recv() {
            return None;
        }

        // Try to receive
        match socket.recv_slice(buf) {
            Ok((len, meta)) => {
                let IpAddress::Ipv4(src_ip) = meta.endpoint.addr;
                Some((src_ip, meta.endpoint.port, len))
            }
            Err(_) => None,
        }
    }

    /// Check if UDP socket can receive data
    pub fn udp_can_recv(&mut self) -> bool {
        let socket = self.sockets.get_mut::<udp::Socket>(self.udp_handle);
        socket.can_recv()
    }

    // =============================================================================
    // TCP METHODS (for HTTP connections)
    // =============================================================================

    /// Connect TCP socket to a remote host
    pub fn tcp_connect(
        &mut self,
        dest_ip: Ipv4Address,
        dest_port: u16,
        timestamp_ms: i64,
    ) -> Result<(), &'static str> {
        let timestamp = Instant::from_millis(timestamp_ms);

        // Get the TCP socket and check current state
        let socket = self.sockets.get_mut::<tcp::Socket>(self.tcp_handle);
        let initial_state = socket.state();
        
        crate::klog::klog_info("tcp", &alloc::format!(
            "tcp_connect: socket initial state = {:?}", initial_state
        ));

        // Close any existing connection and wait for it to fully close
        if initial_state != tcp::State::Closed {
            socket.abort();
            drop(socket); // Release borrow before polling
            
            // Poll multiple times to ensure abort is processed
            for _ in 0..10 {
                self.iface.poll(
                    timestamp,
                    &mut DeviceWrapper(&mut self.device),
                    &mut self.sockets,
                );
                
                let socket = self.sockets.get_mut::<tcp::Socket>(self.tcp_handle);
                if socket.state() == tcp::State::Closed {
                    break;
                }
            }
        }

        // Reset client patching state for the new connection
        reset_client_patching_state();

        // Generate a local port that won't conflict with server ports
        // Use a combination of timestamp and a counter for uniqueness
        static mut PORT_COUNTER: u16 = 0;
        let local_port = unsafe {
            PORT_COUNTER = PORT_COUNTER.wrapping_add(1);
            49152 + ((timestamp_ms as u16).wrapping_add(PORT_COUNTER) % 16383)
        };

        // Create destination endpoint  
        let remote = IpEndpoint::new(IpAddress::Ipv4(dest_ip), dest_port);

        crate::klog::klog_info("tcp", &alloc::format!(
            "tcp_connect: local_port={}, remote={}:{}", 
            local_port, dest_ip, dest_port
        ));

        // Get the socket and verify it's in Closed state
        let socket = self.sockets.get_mut::<tcp::Socket>(self.tcp_handle);
        if socket.state() != tcp::State::Closed {
            crate::klog::klog_info("tcp", &alloc::format!(
                "tcp_connect: WARNING socket not closed, state = {:?}", socket.state()
            ));
        }

        // Connect to remote
        socket
            .connect(self.iface.context(), remote, local_port)
            .map_err(|e| {
                crate::klog::klog_info("tcp", &alloc::format!(
                    "tcp_connect: connect failed: {:?}", e
                ));
                "Failed to initiate TCP connection"
            })?;

        // Verify socket transitioned to SynSent
        let state = socket.state();
        crate::klog::klog_info("tcp", &alloc::format!(
            "tcp_connect: socket after connect = {:?}", state
        ));

        Ok(())
    }

    /// Check if TCP socket is connected
    pub fn tcp_is_connected(&mut self) -> bool {
        let socket = self.sockets.get_mut::<tcp::Socket>(self.tcp_handle);
        socket.state() == tcp::State::Established
    }

    /// Get client TCP socket state as string (for debugging)
    pub fn tcp_client_state(&mut self) -> &'static str {
        let socket = self.sockets.get_mut::<tcp::Socket>(self.tcp_handle);
        match socket.state() {
            tcp::State::Closed => "Closed",
            tcp::State::Listen => "Listen",
            tcp::State::SynSent => "SynSent",
            tcp::State::SynReceived => "SynReceived",
            tcp::State::Established => "Established",
            tcp::State::FinWait1 => "FinWait1",
            tcp::State::FinWait2 => "FinWait2",
            tcp::State::CloseWait => "CloseWait",
            tcp::State::Closing => "Closing",
            tcp::State::LastAck => "LastAck",
            tcp::State::TimeWait => "TimeWait",
        }
    }

    /// Check if TCP socket is connecting (SYN sent)
    pub fn tcp_is_connecting(&mut self) -> bool {
        let socket = self.sockets.get_mut::<tcp::Socket>(self.tcp_handle);
        matches!(
            socket.state(),
            tcp::State::SynSent | tcp::State::SynReceived
        )
    }

    /// Check if TCP connection failed
    pub fn tcp_connection_failed(&mut self) -> bool {
        let socket = self.sockets.get_mut::<tcp::Socket>(self.tcp_handle);
        matches!(socket.state(), tcp::State::Closed | tcp::State::TimeWait)
    }

    /// Send data over TCP connection
    pub fn tcp_send(&mut self, data: &[u8], timestamp_ms: i64) -> Result<usize, &'static str> {
        let timestamp = Instant::from_millis(timestamp_ms);

        // Get the TCP socket
        let socket = self.sockets.get_mut::<tcp::Socket>(self.tcp_handle);

        if !socket.may_send() {
            return Err("TCP socket cannot send");
        }

        let sent = socket
            .send_slice(data)
            .map_err(|_| "Failed to send TCP data")?;

        // Poll to transmit
        self.iface.poll(
            timestamp,
            &mut DeviceWrapper(&mut self.device),
            &mut self.sockets,
        );

        Ok(sent)
    }

    /// Receive data from TCP connection (non-blocking)
    pub fn tcp_recv(&mut self, buf: &mut [u8], timestamp_ms: i64) -> Result<usize, &'static str> {
        let timestamp = Instant::from_millis(timestamp_ms);

        // Poll to receive any pending packets
        self.iface.poll(
            timestamp,
            &mut DeviceWrapper(&mut self.device),
            &mut self.sockets,
        );

        // Get the TCP socket
        let socket = self.sockets.get_mut::<tcp::Socket>(self.tcp_handle);

        if !socket.may_recv() {
            if socket.state() == tcp::State::CloseWait || socket.state() == tcp::State::Closed {
                return Err("Connection closed by peer");
            }
            return Ok(0);
        }

        match socket.recv_slice(buf) {
            Ok(len) => Ok(len),
            Err(_) => Ok(0),
        }
    }

    /// Check if TCP socket can receive data
    pub fn tcp_can_recv(&mut self) -> bool {
        let socket = self.sockets.get_mut::<tcp::Socket>(self.tcp_handle);
        socket.may_recv() && socket.recv_queue() > 0
    }

    /// Close TCP connection gracefully
    pub fn tcp_close(&mut self, timestamp_ms: i64) {
        let timestamp = Instant::from_millis(timestamp_ms);

        let socket = self.sockets.get_mut::<tcp::Socket>(self.tcp_handle);
        socket.close();

        // Note: Don't reset patching state here - FIN-ACK exchange still needs it
        // State will be reset when a new connection is initiated

        // Poll to process the close
        self.iface.poll(
            timestamp,
            &mut DeviceWrapper(&mut self.device),
            &mut self.sockets,
        );
    }

    /// Abort TCP connection immediately
    pub fn tcp_abort(&mut self) {
        let socket = self.sockets.get_mut::<tcp::Socket>(self.tcp_handle);
        socket.abort();
        
        // Reset client patching state since abort is immediate (no FIN exchange)
        reset_client_patching_state();
    }

    /// Get TCP socket state (for debugging)
    pub fn tcp_state(&mut self) -> &'static str {
        let socket = self.sockets.get_mut::<tcp::Socket>(self.tcp_handle);
        match socket.state() {
            tcp::State::Closed => "Closed",
            tcp::State::Listen => "Listen",
            tcp::State::SynSent => "SynSent",
            tcp::State::SynReceived => "SynReceived",
            tcp::State::Established => "Established",
            tcp::State::FinWait1 => "FinWait1",
            tcp::State::FinWait2 => "FinWait2",
            tcp::State::CloseWait => "CloseWait",
            tcp::State::Closing => "Closing",
            tcp::State::LastAck => "LastAck",
            tcp::State::TimeWait => "TimeWait",
        }
    }

    // =============================================================================
    // TCP SERVER METHODS (for listening and accepting connections)
    // =============================================================================

    /// Listen on a TCP port (returns socket ID for this listener)
    /// 
    /// Creates a new server socket bound to the specified port.
    /// Use tcp_accept() to accept incoming connections.
    pub fn tcp_listen(&mut self, port: u16) -> Result<TcpSocketId, &'static str> {
        // Find a free server socket slot
        let socket_id = self.server_sockets.allocate()
            .ok_or("No free server socket slots")?;
        
        // Create TCP socket with static buffers for this slot
        // Using static arrays to avoid heap allocation issues
        let (rx_buffer, tx_buffer) = unsafe {
            let rx = tcp::SocketBuffer::new(&mut TCP_SERVER_RX_DATA[socket_id as usize][..]);
            let tx = tcp::SocketBuffer::new(&mut TCP_SERVER_TX_DATA[socket_id as usize][..]);
            (rx, tx)
        };
        
        let mut tcp_socket = tcp::Socket::new(rx_buffer, tx_buffer);
        
        // Put socket in listen state - use explicit IP
        let local_ip = get_my_ip();
        let local_endpoint = smoltcp::wire::IpListenEndpoint {
            addr: Some(IpAddress::Ipv4(local_ip)),
            port,
        };
        tcp_socket.listen(local_endpoint)
            .map_err(|_| "Failed to listen on port")?;
        
        // Add socket to the socket set
        let handle = self.sockets.add(tcp_socket);
        
        // Update server socket manager
        if let Some(slot) = self.server_sockets.get_mut(socket_id) {
            slot.handle = Some(handle);
            slot.port = port;
            slot.state = ServerSocketState::Listening;
        }
        
        Ok(socket_id)
    }
    
    /// Accept an incoming connection on a listening socket
    /// 
    /// Returns (new_socket_id, remote_ip, remote_port) if a connection is pending.
    /// The listening socket is automatically reset to listen for more connections.
    /// Returns None if no connection is pending.
    pub fn tcp_accept(&mut self, listen_id: TcpSocketId) -> Option<(TcpSocketId, Ipv4Address, u16)> {
        // Get the listening socket info
        let (handle, port) = {
            let slot = self.server_sockets.get(listen_id)?;
            if slot.state != ServerSocketState::Listening {
                return None;
            }
            (slot.handle?, slot.port)
        };
        
        // Check if the socket has a connection established
        let socket = self.sockets.get_mut::<tcp::Socket>(handle);
        
        match socket.state() {
            tcp::State::Established => {
                // Connection established! Get remote endpoint
                let remote = socket.remote_endpoint()?;
                let remote_ip = match remote.addr {
                    IpAddress::Ipv4(ip) => ip,
                    _ => return None,
                };
                let remote_port = remote.port;
                
                // Mark this socket as connected (no longer listening)
                if let Some(slot) = self.server_sockets.get_mut(listen_id) {
                    slot.state = ServerSocketState::Connected;
                }
                
                Some((listen_id, remote_ip, remote_port))
            }
            tcp::State::SynReceived => {
                // Connection in progress, keep waiting
                None
            }
            tcp::State::Listen => {
                // Still waiting for connection
                None
            }
            tcp::State::Closed | tcp::State::TimeWait => {
                // Socket closed, need to re-listen
                // Re-bind to the port with explicit IP (same as tcp_listen)
                let local_ip = get_my_ip();
                let local_endpoint = smoltcp::wire::IpListenEndpoint {
                    addr: Some(IpAddress::Ipv4(local_ip)),
                    port,
                };
                if socket.listen(local_endpoint).is_ok() {
                    if let Some(slot) = self.server_sockets.get_mut(listen_id) {
                        slot.state = ServerSocketState::Listening;
                    }
                }
                None
            }
            _ => None,
        }
    }
    
    /// Send data on a specific server socket
    pub fn tcp_send_on(&mut self, socket_id: TcpSocketId, data: &[u8], timestamp_ms: i64) 
        -> Result<usize, &'static str> 
    {
        let timestamp = Instant::from_millis(timestamp_ms);
        
        // Get the socket handle
        let handle = self.server_sockets.get(socket_id)
            .and_then(|s| s.handle)
            .ok_or("Invalid socket ID")?;
        
        // Get the TCP socket
        let socket = self.sockets.get_mut::<tcp::Socket>(handle);
        
        if !socket.may_send() {
            return Err("Socket cannot send");
        }
        
        let sent = socket.send_slice(data)
            .map_err(|_| "Failed to send data")?;
        
        // Poll to transmit
        self.iface.poll(
            timestamp,
            &mut DeviceWrapper(&mut self.device),
            &mut self.sockets,
        );
        
        Ok(sent)
    }
    
    /// Receive data from a specific server socket (non-blocking)
    pub fn tcp_recv_on(&mut self, socket_id: TcpSocketId, buf: &mut [u8], timestamp_ms: i64)
        -> Result<usize, &'static str>
    {
        let timestamp = Instant::from_millis(timestamp_ms);
        
        // Poll to receive any pending packets
        self.iface.poll(
            timestamp,
            &mut DeviceWrapper(&mut self.device),
            &mut self.sockets,
        );
        
        // Get the socket handle
        let handle = self.server_sockets.get(socket_id)
            .and_then(|s| s.handle)
            .ok_or("Invalid socket ID")?;
        
        // Get the TCP socket
        let socket = self.sockets.get_mut::<tcp::Socket>(handle);
        
        if !socket.may_recv() {
            if socket.state() == tcp::State::CloseWait || socket.state() == tcp::State::Closed {
                return Err("Connection closed by peer");
            }
            return Ok(0);
        }
        
        match socket.recv_slice(buf) {
            Ok(len) => Ok(len),
            Err(_) => Ok(0),
        }
    }
    
    /// Close a specific server socket
    pub fn tcp_close_on(&mut self, socket_id: TcpSocketId, timestamp_ms: i64) {
        let timestamp = Instant::from_millis(timestamp_ms);
        
        // Get the socket handle
        let handle = if let Some(slot) = self.server_sockets.get(socket_id) {
            slot.handle
        } else {
            return;
        };
        
        if let Some(h) = handle {
            // Close the socket
            let socket = self.sockets.get_mut::<tcp::Socket>(h);
            socket.close();
            
            // Note: Don't reset patching state here - FIN-ACK exchange still needs it
            // State will be reset in tcp_release_server() after socket is fully closed
            
            // Poll to process the close
            self.iface.poll(
                timestamp,
                &mut DeviceWrapper(&mut self.device),
                &mut self.sockets,
            );
            
            // Mark slot as closing (will be released when socket is fully closed)
            if let Some(slot) = self.server_sockets.get_mut(socket_id) {
                slot.state = ServerSocketState::Closing;
            }
        }
    }
    
    /// Get server socket state (for debugging)
    pub fn tcp_server_state(&mut self, socket_id: TcpSocketId) -> &'static str {
        let handle = match self.server_sockets.get(socket_id) {
            Some(slot) => match slot.handle {
                Some(h) => h,
                None => return "Invalid",
            },
            None => return "Invalid",
        };
        
        let socket = self.sockets.get_mut::<tcp::Socket>(handle);
        match socket.state() {
            tcp::State::Closed => "Closed",
            tcp::State::Listen => "Listen",
            tcp::State::SynSent => "SynSent",
            tcp::State::SynReceived => "SynReceived",
            tcp::State::Established => "Established",
            tcp::State::FinWait1 => "FinWait1",
            tcp::State::FinWait2 => "FinWait2",
            tcp::State::CloseWait => "CloseWait",
            tcp::State::Closing => "Closing",
            tcp::State::LastAck => "LastAck",
            tcp::State::TimeWait => "TimeWait",
        }
    }
    
    /// Check if a server socket can receive data
    pub fn tcp_can_recv_on(&mut self, socket_id: TcpSocketId) -> bool {
        let handle = match self.server_sockets.get(socket_id) {
            Some(slot) => match slot.handle {
                Some(h) => h,
                None => return false,
            },
            None => return false,
        };
        
        let socket = self.sockets.get_mut::<tcp::Socket>(handle);
        socket.may_recv() && socket.recv_queue() > 0
    }
    
    /// Release a server socket slot back to the pool
    /// Call this after the socket is fully closed
    pub fn tcp_release_server(&mut self, socket_id: TcpSocketId) {
        // Get the port before releasing, to reset patching state for this port
        let socket_port = self.server_sockets.get(socket_id).map(|s| s.port);
        
        if let Some(slot) = self.server_sockets.get(socket_id) {
            if let Some(handle) = slot.handle {
                // Remove socket from socket set
                self.sockets.remove(handle);
            }
        }
        self.server_sockets.release(socket_id);
        
        // Reset per-port patching state for this specific port
        if let Some(port) = socket_port {
            reset_server_patching_for_port(port);
        }
    }
}
