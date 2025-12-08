//! Network stack using smoltcp.
//!
//! This module provides the TCP/IP stack for the kernel using the smoltcp crate.

use crate::virtio_net::VirtioNet;
use alloc::boxed::Box;
use alloc::collections::VecDeque;
use alloc::vec;
use alloc::vec::Vec;
use smoltcp::iface::{Config, Interface, SocketHandle, SocketSet, SocketStorage};
use smoltcp::phy::{Device, DeviceCapabilities, Medium, RxToken, TxToken};
use smoltcp::socket::{icmp, tcp, udp};
use smoltcp::time::Instant;
use smoltcp::wire::{EthernetAddress, HardwareAddress, IpAddress, IpCidr, IpEndpoint, Ipv4Address};

/// Network configuration
/// Default IP address (used as fallback if no IP is assigned by relay)
pub const DEFAULT_IP_ADDR: Ipv4Address = Ipv4Address::new(10, 0, 2, 15);
pub const GATEWAY: Ipv4Address = Ipv4Address::new(10, 0, 2, 2);
pub const PREFIX_LEN: u8 = 24;

/// Dynamic IP address assigned by the relay/network controller
/// This is set during network initialization
pub static mut MY_IP_ADDR: Ipv4Address = Ipv4Address::new(10, 0, 2, 15);

/// Get the current IP address (safe wrapper)
pub fn get_my_ip() -> Ipv4Address {
    unsafe { MY_IP_ADDR }
}

/// Store the last received SYN's sequence number so we can patch SYN-ACK if needed
/// This is a workaround for the smoltcp ack=0/1 bug (SERVER ROLE)
static mut LAST_SYN_SEQ: Option<u32> = None;

/// Store the server's SYN-ACK seq number so we can patch incoming ACKs (SERVER ROLE)
static mut SERVER_SYNACK_SEQ: Option<u32> = None;

/// Store the connection port for matching (SERVER ROLE - destination port of incoming SYN)
static mut PATCHING_PORT: Option<u16> = None;

// CLIENT ROLE patching state (for outgoing connections like telnet)
/// Store the server's seq from SYN-ACK so we can patch outgoing ACKs (CLIENT ROLE)
static mut CLIENT_SERVER_SEQ: Option<u32> = None;

/// Store the remote port we're connecting to (CLIENT ROLE)
static mut CLIENT_REMOTE_PORT: Option<u16> = None;

/// DNS server (Google Public DNS)
pub const DNS_SERVER: Ipv4Address = Ipv4Address::new(8, 8, 8, 8);
/// DNS port
pub const DNS_PORT: u16 = 53;

/// Loopback address
pub const LOOPBACK: Ipv4Address = Ipv4Address::new(127, 0, 0, 1);

/// ICMP identifier for our ping socket
const ICMP_IDENT: u16 = 0x1234;

/// Local port for DNS queries
const DNS_LOCAL_PORT: u16 = 10053;

/// Pending loopback ping reply
struct LoopbackReply {
    from: Ipv4Address,
    ident: u16,
    seq: u16,
}

/// Static storage for sockets (expanded for server sockets)
static mut SOCKET_STORAGE: [SocketStorage<'static>; 16] = [SocketStorage::EMPTY; 16];

/// Static storage for ICMP buffers - need larger buffers for proper ICMP
static mut ICMP_RX_META: [icmp::PacketMetadata; 8] = [icmp::PacketMetadata::EMPTY; 8];
static mut ICMP_TX_META: [icmp::PacketMetadata; 8] = [icmp::PacketMetadata::EMPTY; 8];
static mut ICMP_RX_DATA: [u8; 512] = [0; 512];
static mut ICMP_TX_DATA: [u8; 512] = [0; 512];

/// Static storage for UDP buffers (for DNS queries)
static mut UDP_RX_META: [udp::PacketMetadata; 8] = [udp::PacketMetadata::EMPTY; 8];
static mut UDP_TX_META: [udp::PacketMetadata; 8] = [udp::PacketMetadata::EMPTY; 8];
static mut UDP_RX_DATA: [u8; 1024] = [0; 1024];
static mut UDP_TX_DATA: [u8; 1024] = [0; 1024];

/// Static storage for TCP buffers (for HTTP client connections)
static mut TCP_RX_DATA: [u8; 8192] = [0; 8192];
static mut TCP_TX_DATA: [u8; 4096] = [0; 4096];

// =============================================================================
// TCP SERVER SOCKET INFRASTRUCTURE
// =============================================================================

/// Maximum number of server TCP sockets
pub const MAX_SERVER_SOCKETS: usize = 4;

/// TCP socket ID for multi-socket operations
pub type TcpSocketId = u8;

/// Static storage for server TCP buffers
static mut TCP_SERVER_RX_DATA: [[u8; 2048]; MAX_SERVER_SOCKETS] = [[0; 2048]; MAX_SERVER_SOCKETS];
static mut TCP_SERVER_TX_DATA: [[u8; 1024]; MAX_SERVER_SOCKETS] = [[0; 1024]; MAX_SERVER_SOCKETS];

/// Server socket state
#[derive(Clone, Copy, PartialEq)]
pub enum ServerSocketState {
    /// Socket slot is free
    Free,
    /// Socket is listening for connections
    Listening,
    /// Socket has an active connection
    Connected,
    /// Socket is closing
    Closing,
}

/// A server socket entry
struct ServerSocket {
    handle: Option<SocketHandle>,
    port: u16,
    state: ServerSocketState,
}

impl ServerSocket {
    const fn new() -> Self {
        Self {
            handle: None,
            port: 0,
            state: ServerSocketState::Free,
        }
    }
}

/// Manager for server TCP sockets
struct TcpServerManager {
    sockets: [ServerSocket; MAX_SERVER_SOCKETS],
}

impl TcpServerManager {
    const fn new() -> Self {
        Self {
            sockets: [
                ServerSocket::new(),
                ServerSocket::new(),
                ServerSocket::new(),
                ServerSocket::new(),
            ],
        }
    }
    
    /// Allocate a free socket slot, returns socket ID
    fn allocate(&mut self) -> Option<TcpSocketId> {
        for (i, slot) in self.sockets.iter_mut().enumerate() {
            if slot.state == ServerSocketState::Free {
                return Some(i as TcpSocketId);
            }
        }
        None
    }
    
    /// Get socket info by ID
    fn get(&self, id: TcpSocketId) -> Option<&ServerSocket> {
        self.sockets.get(id as usize)
    }
    
    /// Get mutable socket info by ID    
    fn get_mut(&mut self, id: TcpSocketId) -> Option<&mut ServerSocket> {
        self.sockets.get_mut(id as usize)
    }
    
    /// Release a socket slot
    fn release(&mut self, id: TcpSocketId) {
        if let Some(slot) = self.sockets.get_mut(id as usize) {
            slot.handle = None;
            slot.port = 0;
            slot.state = ServerSocketState::Free;
        }
    }
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
}

impl NetState {
    /// Initialize the network stack
    /// Note: After storing this in a static, call finalize() to complete RX buffer setup!
    /// Returns Err if no IP is assigned by the relay (networking will be disabled).
    pub fn new(mut device: VirtioNet) -> Result<Self, &'static str> {
        // Initialize the VirtIO device (phase 1 - configures queues but doesn't populate RX)
        device.init()?;

        // --- IP ADDRESS DISCOVERY ---
        // Wait for an IP assignment from the host/relay
        // In WASM, async tasks need the JS event loop to run, so we use many
        // short iterations to give the browser more chances to process events
        let mut my_ip = DEFAULT_IP_ADDR; // Fallback default
        let mut got_ip = false;

        crate::uart::write_str("    \x1b[0;90m+-\x1b[0m Waiting for IP assignment");
        // 200 iterations with shorter delays = more chances for async tasks in WASM
        // Total wait time is roughly 2-3 seconds
        for i in 0..200 {
            if let Some(ip_bytes) = device.get_config_ip() {
                my_ip = Ipv4Address::new(ip_bytes[0], ip_bytes[1], ip_bytes[2], ip_bytes[3]);
                got_ip = true;
                crate::uart::write_line(" \x1b[1;32m[OK]\x1b[0m");
                break;
            }
            // Print a dot every 10 iterations to show progress
            if i % 10 == 0 {
                crate::uart::write_str(".");
            }
            // Shorter delay to allow more frequent checks and JS event loop to run in WASM
            for _ in 0..200_000 {
                core::hint::spin_loop();
            }
        }

        // If we didn't get an IP, network is unavailable - return error
        if !got_ip {
            crate::uart::write_line(" \x1b[1;31m[FAILED]\x1b[0m");
            crate::uart::write_line("    \x1b[1;31m[X]\x1b[0m No IP address assigned by relay");
            crate::uart::write_line(
                "    \x1b[0;90m    +- Check relay connection and certificate hash\x1b[0m",
            );
            return Err("No IP address assigned - networking disabled");
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
        if let Some(slot) = self.server_sockets.get(socket_id) {
            if let Some(handle) = slot.handle {
                // Remove socket from socket set
                self.sockets.remove(handle);
            }
        }
        self.server_sockets.release(socket_id);
    }
}

/// Wrapper for VirtioNet to implement smoltcp Device trait
struct DeviceWrapper<'a>(&'a mut VirtioNet);

impl Device for DeviceWrapper<'_> {
    type RxToken<'a>
        = VirtioRxToken
    where
        Self: 'a;
    type TxToken<'a>
        = VirtioTxToken<'a>
    where
        Self: 'a;

    fn capabilities(&self) -> DeviceCapabilities {
        let mut caps = DeviceCapabilities::default();
        caps.medium = Medium::Ethernet;
        caps.max_transmission_unit = 1500; // Standard Ethernet MTU (IP payload size)
        caps.max_burst_size = Some(1);
        // Explicitly tell smoltcp to compute all checksums in software
        // (no hardware offload)
        caps.checksum = smoltcp::phy::ChecksumCapabilities::default();
        caps
    }

    fn receive(&mut self, _timestamp: Instant) -> Option<(Self::RxToken<'_>, Self::TxToken<'_>)> {
        // Check if there's a received packet
        if let Some((desc_idx, data)) = self.0.recv_with_desc() {
            // Copy data since we need to recycle the buffer
            let mut buf = vec![0u8; data.len()];
            buf.copy_from_slice(data);

            // Recycle the RX buffer immediately
            self.0.recycle_rx(desc_idx);

            Some((
                VirtioRxToken { buffer: buf },
                VirtioTxToken { device: self.0 },
            ))
        } else {
            None
        }
    }

    fn transmit(&mut self, _timestamp: Instant) -> Option<Self::TxToken<'_>> {
        // Always allow transmit (the device will handle buffer exhaustion)
        Some(VirtioTxToken { device: self.0 })
    }
}

/// RX token for received packets
struct VirtioRxToken {
    buffer: Vec<u8>,
}

impl RxToken for VirtioRxToken {
    fn consume<R, F>(self, f: F) -> R
    where
        F: FnOnce(&[u8]) -> R,
    {
        let mut buffer = self.buffer;
        
        // Process TCP packets for state tracking and patching
        if buffer.len() >= 54 && buffer[12] == 0x08 && buffer[13] == 0x00 && buffer[23] == 6 {
            let ip_ihl = (buffer[14] & 0x0F) as usize * 4;
            let tcp_offset = 14 + ip_ihl;
            
            if buffer.len() >= tcp_offset + 20 {
                let seq = u32::from_be_bytes([buffer[tcp_offset + 4], buffer[tcp_offset + 5], buffer[tcp_offset + 6], buffer[tcp_offset + 7]]);
                let ack = u32::from_be_bytes([buffer[tcp_offset + 8], buffer[tcp_offset + 9], buffer[tcp_offset + 10], buffer[tcp_offset + 11]]);
                let tcp_flags = buffer[tcp_offset + 13];
                let src_port = u16::from_be_bytes([buffer[tcp_offset], buffer[tcp_offset + 1]]);
                let dst_port = u16::from_be_bytes([buffer[tcp_offset + 2], buffer[tcp_offset + 3]]);
                
                // Store sequence numbers for patching workaround
                if tcp_flags & 0x02 != 0 && tcp_flags & 0x10 == 0 {
                    // Pure SYN - store seq and port for later patching of SYN-ACK (SERVER ROLE)
                    unsafe { 
                        LAST_SYN_SEQ = Some(seq);
                        PATCHING_PORT = Some(dst_port);
                    }
                } else if tcp_flags & 0x02 != 0 && tcp_flags & 0x10 != 0 {
                    // SYN-ACK received - store server's seq for client role patching
                    unsafe {
                        CLIENT_SERVER_SEQ = Some(seq);
                        CLIENT_REMOTE_PORT = Some(src_port);
                    }
                }
                
                // Patch incoming ACK packets (SERVER ROLE)
                if tcp_flags & 0x10 != 0 && tcp_flags & 0x02 == 0 && tcp_flags & 0x04 == 0 {
                    if let (Some(server_seq), Some(port)) = unsafe { (SERVER_SYNACK_SEQ, PATCHING_PORT) } {
                        if dst_port == port {
                            let expected_ack = server_seq.wrapping_add(1);
                            if ack != expected_ack && ack < 1000 {
                                // Patch the ack number
                                let ack_bytes = expected_ack.to_be_bytes();
                                buffer[tcp_offset + 8] = ack_bytes[0];
                                buffer[tcp_offset + 9] = ack_bytes[1];
                                buffer[tcp_offset + 10] = ack_bytes[2];
                                buffer[tcp_offset + 11] = ack_bytes[3];
                                
                                // Recalculate TCP checksum
                                recalculate_tcp_checksum(&mut buffer, tcp_offset);
                            }
                        }
                    }
                }
            }
        }
        
        f(&buffer)
    }
}

/// TX token for transmitting packets
struct VirtioTxToken<'a> {
    device: &'a mut VirtioNet,
}

/// Helper function to recalculate TCP checksum after patching
fn recalculate_tcp_checksum(buffer: &mut [u8], tcp_offset: usize) {
    // Zero out the checksum field
    buffer[tcp_offset + 16] = 0;
    buffer[tcp_offset + 17] = 0;
    
    // Calculate pseudo-header checksum
    let ip_offset = 14;
    let src_ip = u32::from_be_bytes([buffer[ip_offset + 12], buffer[ip_offset + 13], buffer[ip_offset + 14], buffer[ip_offset + 15]]);
    let dst_ip = u32::from_be_bytes([buffer[ip_offset + 16], buffer[ip_offset + 17], buffer[ip_offset + 18], buffer[ip_offset + 19]]);
    let tcp_len = buffer.len() - tcp_offset;
    
    let mut sum: u32 = 0;
    sum += (src_ip >> 16) as u32;
    sum += (src_ip & 0xFFFF) as u32;
    sum += (dst_ip >> 16) as u32;
    sum += (dst_ip & 0xFFFF) as u32;
    sum += 6u32; // TCP protocol number
    sum += tcp_len as u32;
    
    // Add TCP segment (16-bit words)
    let tcp_data = &buffer[tcp_offset..];
    let mut i = 0;
    while i + 1 < tcp_data.len() {
        sum += u16::from_be_bytes([tcp_data[i], tcp_data[i + 1]]) as u32;
        i += 2;
    }
    if i < tcp_data.len() {
        sum += (tcp_data[i] as u32) << 8;
    }
    
    // Fold 32-bit sum to 16 bits
    while sum >> 16 != 0 {
        sum = (sum & 0xFFFF) + (sum >> 16);
    }
    let checksum = !(sum as u16);
    let checksum_bytes = checksum.to_be_bytes();
    buffer[tcp_offset + 16] = checksum_bytes[0];
    buffer[tcp_offset + 17] = checksum_bytes[1];
}

impl TxToken for VirtioTxToken<'_> {
    fn consume<R, F>(self, len: usize, f: F) -> R
    where
        F: FnOnce(&mut [u8]) -> R,
    {
        let mut buffer = vec![0u8; len];
        let result = f(&mut buffer);

        // Apply TCP ACK patching workaround for smoltcp bug
        if buffer.len() >= 40 && buffer[12] == 0x08 && buffer[13] == 0x00 {
            let ip_ihl = (buffer[14] & 0x0F) as usize * 4;
            let tcp_offset = 14 + ip_ihl;
            
            if buffer.len() >= tcp_offset + 20 && buffer[23] == 6 {
                let tcp_flags = buffer[tcp_offset + 13];
                let src_port = u16::from_be_bytes([buffer[tcp_offset], buffer[tcp_offset + 1]]);
                let dst_port = u16::from_be_bytes([buffer[tcp_offset + 2], buffer[tcp_offset + 3]]);
                let seq_num = u32::from_be_bytes([buffer[tcp_offset + 4], buffer[tcp_offset + 5], buffer[tcp_offset + 6], buffer[tcp_offset + 7]]);
                let ack_num = u32::from_be_bytes([buffer[tcp_offset + 8], buffer[tcp_offset + 9], buffer[tcp_offset + 10], buffer[tcp_offset + 11]]);
                
                // SYN-ACK patching (SERVER ROLE)
                if tcp_flags & 0x02 != 0 && tcp_flags & 0x10 != 0 {
                    if let Some(syn_seq) = unsafe { LAST_SYN_SEQ } {
                        let correct_ack = syn_seq.wrapping_add(1);
                        if ack_num != correct_ack {
                            // Patch the ack number
                            let ack_bytes = correct_ack.to_be_bytes();
                            buffer[tcp_offset + 8] = ack_bytes[0];
                            buffer[tcp_offset + 9] = ack_bytes[1];
                            buffer[tcp_offset + 10] = ack_bytes[2];
                            buffer[tcp_offset + 11] = ack_bytes[3];
                            
                            recalculate_tcp_checksum(&mut buffer, tcp_offset);
                            
                            // Store server's seq for patching incoming ACKs
                            unsafe { SERVER_SYNACK_SEQ = Some(seq_num); }
                        }
                    }
                } else if tcp_flags & 0x10 != 0 {
                    // ACK packets - patch if ack is suspiciously low
                    
                    // SERVER ROLE: we're sending from our listening port
                    if let Some(syn_seq) = unsafe { LAST_SYN_SEQ } {
                        if let Some(port) = unsafe { PATCHING_PORT } {
                            if src_port == port && ack_num < 1000 {
                                let correct_ack = syn_seq.wrapping_add(1);
                                let ack_bytes = correct_ack.to_be_bytes();
                                buffer[tcp_offset + 8] = ack_bytes[0];
                                buffer[tcp_offset + 9] = ack_bytes[1];
                                buffer[tcp_offset + 10] = ack_bytes[2];
                                buffer[tcp_offset + 11] = ack_bytes[3];
                                recalculate_tcp_checksum(&mut buffer, tcp_offset);
                            }
                        }
                    }
                    
                    // CLIENT ROLE: we're sending TO a remote server port
                    if let Some(server_seq) = unsafe { CLIENT_SERVER_SEQ } {
                        if let Some(remote_port) = unsafe { CLIENT_REMOTE_PORT } {
                            if dst_port == remote_port && ack_num < 1000 {
                                let correct_ack = server_seq.wrapping_add(1);
                                let ack_bytes = correct_ack.to_be_bytes();
                                buffer[tcp_offset + 8] = ack_bytes[0];
                                buffer[tcp_offset + 9] = ack_bytes[1];
                                buffer[tcp_offset + 10] = ack_bytes[2];
                                buffer[tcp_offset + 11] = ack_bytes[3];
                                recalculate_tcp_checksum(&mut buffer, tcp_offset);
                            }
                        }
                    }
                }
            }
        }

        // Send the packet
        let _ = self.device.send(&buffer);

        result
    }
}

/// Parse an IPv4 address from bytes
pub fn parse_ipv4(s: &[u8]) -> Option<Ipv4Address> {
    let mut octets = [0u8; 4];
    let mut octet_idx = 0;
    let mut current = 0u16;
    let mut has_digit = false;

    for &b in s {
        if b >= b'0' && b <= b'9' {
            current = current * 10 + (b - b'0') as u16;
            has_digit = true;
            if current > 255 {
                return None;
            }
        } else if b == b'.' {
            if !has_digit || octet_idx >= 3 {
                return None;
            }
            octets[octet_idx] = current as u8;
            octet_idx += 1;
            current = 0;
            has_digit = false;
        } else {
            return None;
        }
    }

    if !has_digit || octet_idx != 3 {
        return None;
    }
    octets[3] = current as u8;

    Some(Ipv4Address::new(octets[0], octets[1], octets[2], octets[3]))
}

/// Format an IPv4 address to a buffer
pub fn format_ipv4(addr: Ipv4Address, buf: &mut [u8]) -> usize {
    let octets = addr.octets();
    let mut pos = 0;

    for (i, &octet) in octets.iter().enumerate() {
        // Write octet
        if octet >= 100 {
            buf[pos] = b'0' + (octet / 100);
            pos += 1;
        }
        if octet >= 10 {
            buf[pos] = b'0' + ((octet / 10) % 10);
            pos += 1;
        }
        buf[pos] = b'0' + (octet % 10);
        pos += 1;

        // Write dot (except after last octet)
        if i < 3 {
            buf[pos] = b'.';
            pos += 1;
        }
    }

    pos
}
