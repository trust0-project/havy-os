//! D1 EMAC Network State management
//!
//! Simplified network state for D1 EMAC devices (real hardware and VM emulation).
//! Provides basic smoltcp integration without VirtIO-specific features.

use alloc::collections::VecDeque;

use smoltcp::iface::{Interface, SocketHandle, SocketSet, Config, SocketStorage};
use smoltcp::socket::{icmp, tcp, udp};
use smoltcp::time::Instant;
use smoltcp::wire::{EthernetAddress, HardwareAddress, IpAddress, IpCidr, IpEndpoint, Ipv4Address};

use crate::d1_emac::{D1Emac, D1EmacDevice};
use crate::device::NetworkDevice;  // Trait for mac_address()
use super::config::*;
use super::server::*;

/// Pending loopback ping reply
struct LoopbackReply {
    from: Ipv4Address,
    ident: u16,
    seq: u16,
}

/// D1 EMAC Network state
pub struct D1NetState {
    device: D1Emac,
    iface: Interface,
    sockets: SocketSet<'static>,
    icmp_handle: SocketHandle,
    udp_handle: SocketHandle,
    tcp_handle: SocketHandle,
    loopback_replies: VecDeque<LoopbackReply>,
    server_sockets: TcpServerManager,
    mac: [u8; 6],
    /// Whether IP has been assigned from relay
    ip_assigned: bool,
}

impl D1NetState {
    /// Initialize the network stack with D1 EMAC
    /// Note: IP address is not set here - netd will poll for it from relay
    pub fn new(mut device: D1Emac) -> Result<Self, &'static str> {
        // Use 0.0.0.0 initially - netd will poll for assigned IP from relay
        let my_ip = DEFAULT_IP_ADDR;
        
        // Don't set global MY_IP_ADDR here - netd will do it when relay assigns IP

        let mac = device.mac_address();
        let hw_addr = HardwareAddress::Ethernet(EthernetAddress(mac));

        // Create interface config
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
        
        let mut config = Config::new(hw_addr);
        config.random_seed = seed;

        // Create the interface
        let mut iface = Interface::new(
            config,
            &mut D1EmacDevice(&mut device),
            Instant::from_millis(0),
        );

        // Configure IP address
        iface.update_ip_addrs(|addrs| {
            addrs.push(IpCidr::new(IpAddress::Ipv4(my_ip), PREFIX_LEN)).ok();
        });

        // Set default gateway
        iface.routes_mut().add_default_ipv4_route(GATEWAY).ok();

        // Create socket set with static storage
        let sockets = unsafe { SocketSet::new(&mut D1_SOCKET_STORAGE[..]) };

        // Create ICMP socket for ping
        let icmp_rx_buffer = unsafe { icmp::PacketBuffer::new(&mut D1_ICMP_RX_META[..], &mut D1_ICMP_RX_DATA[..]) };
        let icmp_tx_buffer = unsafe { icmp::PacketBuffer::new(&mut D1_ICMP_TX_META[..], &mut D1_ICMP_TX_DATA[..]) };
        let mut icmp_socket = icmp::Socket::new(icmp_rx_buffer, icmp_tx_buffer);
        icmp_socket.bind(icmp::Endpoint::Ident(ICMP_IDENT)).ok();

        // Create UDP socket
        let udp_rx_buffer = unsafe { udp::PacketBuffer::new(&mut D1_UDP_RX_META[..], &mut D1_UDP_RX_DATA[..]) };
        let udp_tx_buffer = unsafe { udp::PacketBuffer::new(&mut D1_UDP_TX_META[..], &mut D1_UDP_TX_DATA[..]) };
        let mut udp_socket = udp::Socket::new(udp_rx_buffer, udp_tx_buffer);
        udp_socket.bind(DNS_LOCAL_PORT).ok();

        // Create TCP socket
        let tcp_rx_buffer = unsafe { tcp::SocketBuffer::new(&mut D1_TCP_RX_DATA[..]) };
        let tcp_tx_buffer = unsafe { tcp::SocketBuffer::new(&mut D1_TCP_TX_DATA[..]) };
        let tcp_socket = tcp::Socket::new(tcp_rx_buffer, tcp_tx_buffer);

        let mut state = D1NetState {
            device,
            iface,
            sockets,
            icmp_handle: SocketHandle::default(),
            udp_handle: SocketHandle::default(),
            tcp_handle: SocketHandle::default(),
            loopback_replies: VecDeque::new(),
            server_sockets: TcpServerManager::new(),
            mac,
            ip_assigned: false,
        };

        state.icmp_handle = state.sockets.add(icmp_socket);
        state.udp_handle = state.sockets.add(udp_socket);
        state.tcp_handle = state.sockets.add(tcp_socket);

        Ok(state)
    }

    /// Poll the network stack
    pub fn poll(&mut self, timestamp_ms: i64) {
        let timestamp = Instant::from_millis(timestamp_ms);
        
        // Check for background IP assignment from relay
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
            }
        }
        
        self.iface.poll(
            timestamp,
            &mut D1EmacDevice(&mut self.device),
            &mut self.sockets,
        );
    }

    /// Get MAC address
    pub fn mac(&self) -> [u8; 6] {
        self.mac
    }

    /// Get MAC address as string
    pub fn mac_str(&self) -> [u8; 17] {
        let hex_chars = b"0123456789abcdef";
        let mut s = [0u8; 17];
        for i in 0..6 {
            s[i * 3] = hex_chars[(self.mac[i] >> 4) as usize];
            s[i * 3 + 1] = hex_chars[(self.mac[i] & 0x0F) as usize];
            if i < 5 {
                s[i * 3 + 2] = b':';
            }
        }
        s
    }

    // =========================================================================
    // UDP METHODS (for DNS resolution)
    // =========================================================================

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
            &mut D1EmacDevice(&mut self.device),
            &mut self.sockets,
        );

        Ok(())
    }

    /// Receive a UDP packet (non-blocking)
    /// Returns (source_ip, source_port, length) if a packet is available
    pub fn udp_recv(
        &mut self,
        buf: &mut [u8],
        timestamp_ms: i64,
    ) -> Option<(Ipv4Address, u16, usize)> {
        let timestamp = Instant::from_millis(timestamp_ms);

        // Poll to receive any pending packets
        self.iface.poll(
            timestamp,
            &mut D1EmacDevice(&mut self.device),
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

    // =========================================================================
    // TCP SERVER METHODS (for tcpd/httpd)
    // =========================================================================

    /// Listen on a TCP port (returns socket ID for accepting connections)
    pub fn tcp_listen(&mut self, port: u16) -> Result<TcpSocketId, &'static str> {
        // Allocate a server socket slot
        let socket_id = self.server_sockets.allocate()
            .ok_or("No server socket slots available")?;
        
        // Get server socket buffers
        let (rx_data, tx_data) = unsafe {
            let idx = socket_id as usize;
            if idx >= MAX_SERVER_SOCKETS {
                return Err("Invalid socket index");
            }
            (&mut D1_TCP_SERVER_RX_DATA[idx][..], &mut D1_TCP_SERVER_TX_DATA[idx][..])
        };
        
        // Create TCP socket
        let tcp_rx_buffer = tcp::SocketBuffer::new(rx_data);
        let tcp_tx_buffer = tcp::SocketBuffer::new(tx_data);
        let mut tcp_socket = tcp::Socket::new(tcp_rx_buffer, tcp_tx_buffer);
        
        // Listen on the port
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
    pub fn tcp_accept(&mut self, listen_id: TcpSocketId) -> Option<(TcpSocketId, Ipv4Address, u16)> {
        let (handle, port) = {
            let slot = self.server_sockets.get(listen_id)?;
            if slot.state != ServerSocketState::Listening {
                return None;
            }
            (slot.handle?, slot.port)
        };
        
        let socket = self.sockets.get_mut::<tcp::Socket>(handle);
        
        match socket.state() {
            tcp::State::Established => {
                let remote = socket.remote_endpoint()?;
                let remote_ip = match remote.addr {
                    IpAddress::Ipv4(ip) => ip,
                    _ => return None,
                };
                let remote_port = remote.port;
                
                if let Some(slot) = self.server_sockets.get_mut(listen_id) {
                    slot.state = ServerSocketState::Connected;
                }
                
                Some((listen_id, remote_ip, remote_port))
            }
            tcp::State::Closed | tcp::State::TimeWait => {
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

    /// Get TCP server socket state as string
    pub fn tcp_server_state(&mut self, socket_id: TcpSocketId) -> &'static str {
        let handle = match self.server_sockets.get(socket_id).and_then(|s| s.handle) {
            Some(h) => h,
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

    /// Send data on a specific server socket
    pub fn tcp_send_on(&mut self, socket_id: TcpSocketId, data: &[u8], timestamp_ms: i64) 
        -> Result<usize, &'static str> 
    {
        let timestamp = Instant::from_millis(timestamp_ms);
        
        let handle = self.server_sockets.get(socket_id)
            .and_then(|s| s.handle)
            .ok_or("Invalid socket ID")?;
        
        let socket = self.sockets.get_mut::<tcp::Socket>(handle);
        
        if !socket.may_send() {
            return Err("Socket cannot send");
        }
        
        let sent = socket.send_slice(data)
            .map_err(|_| "Failed to send data")?;
        
        // Poll to transmit
        self.iface.poll(
            timestamp,
            &mut D1EmacDevice(&mut self.device),
            &mut self.sockets,
        );
        
        Ok(sent)
    }

    /// Close a server socket
    pub fn tcp_close_on(&mut self, socket_id: TcpSocketId, timestamp_ms: i64) {
        let timestamp = Instant::from_millis(timestamp_ms);
        
        if let Some(handle) = self.server_sockets.get(socket_id).and_then(|s| s.handle) {
            let socket = self.sockets.get_mut::<tcp::Socket>(handle);
            socket.close();
            
            self.iface.poll(
                timestamp,
                &mut D1EmacDevice(&mut self.device),
                &mut self.sockets,
            );
        }
    }

    /// Release a server socket slot back to the pool
    pub fn tcp_release_server(&mut self, socket_id: TcpSocketId) {
        if let Some(slot) = self.server_sockets.get_mut(socket_id) {
            if let Some(handle) = slot.handle.take() {
                self.sockets.remove(handle);
            }
            slot.state = ServerSocketState::Free;
            slot.port = 0;
        }
    }

    // =========================================================================
    // ICMP PING METHODS
    // =========================================================================

    /// Check if address is loopback (127.x.x.x)
    fn is_loopback(addr: &Ipv4Address) -> bool {
        addr.octets()[0] == 127
    }

    /// Check if address is our own IP
    fn is_self(addr: &Ipv4Address) -> bool {
        *addr == get_my_ip()
    }
    
    /// Send an ICMP echo request (ping) using smoltcp ICMP socket
    pub fn send_ping(
        &mut self,
        target: Ipv4Address,
        seq: u16,
        timestamp_ms: i64,
    ) -> Result<(), &'static str> {
        // Handle loopback addresses (127.x.x.x) and self-ping locally
        if Self::is_loopback(&target) || Self::is_self(&target) {
            self.loopback_replies.push_back(LoopbackReply {
                from: target,
                ident: ICMP_IDENT,
                seq,
            });
            return Ok(());
        }

        let timestamp = Instant::from_millis(timestamp_ms);

        // Build ICMP echo request payload
        let echo_payload = b"RISCV_PING";
        
        // Poll first to ensure interface is ready
        self.iface.poll(timestamp, &mut D1EmacDevice(&mut self.device), &mut self.sockets);

        // Get ICMP socket
        let socket = self.sockets.get_mut::<icmp::Socket>(self.icmp_handle);
        
        // Check if socket can send
        if !socket.can_send() {
            return Err("ICMP socket cannot send");
        }

        // Build ICMP echo request packet manually
        // ICMP header: type(1) + code(1) + checksum(2) + ident(2) + seq(2) + data
        let icmp_len = 8 + echo_payload.len();
        let mut icmp_packet = alloc::vec![0u8; icmp_len];
        
        icmp_packet[0] = 8; // type = echo request
        icmp_packet[1] = 0; // code = 0
        icmp_packet[2] = 0; // checksum (will fill later)
        icmp_packet[3] = 0;
        icmp_packet[4..6].copy_from_slice(&ICMP_IDENT.to_be_bytes()); // identifier
        icmp_packet[6..8].copy_from_slice(&seq.to_be_bytes()); // sequence
        icmp_packet[8..].copy_from_slice(echo_payload); // data
        
        // Calculate ICMP checksum
        let checksum = Self::icmp_checksum(&icmp_packet);
        icmp_packet[2..4].copy_from_slice(&checksum.to_be_bytes());
        
        // Send via smoltcp ICMP socket
        socket.send_slice(
            &icmp_packet,
            IpAddress::Ipv4(target),
        ).map_err(|_| "Failed to send ICMP")?;

        // Poll to actually transmit
        self.iface.poll(timestamp, &mut D1EmacDevice(&mut self.device), &mut self.sockets);

        Ok(())
    }
    
    /// Calculate ICMP checksum
    fn icmp_checksum(data: &[u8]) -> u16 {
        let mut sum: u32 = 0;
        let len = data.len();
        let mut i = 0;
        while i < len - 1 {
            sum += u16::from_be_bytes([data[i], data[i + 1]]) as u32;
            i += 2;
        }
        if len % 2 == 1 {
            sum += (data[len - 1] as u32) << 8;
        }
        while sum > 0xFFFF {
            sum = (sum & 0xFFFF) + (sum >> 16);
        }
        !(sum as u16)
    }

    /// Check for ICMP echo reply
    pub fn check_ping_reply(&mut self) -> Option<(Ipv4Address, u16, u16)> {
        // First check loopback replies
        if let Some(reply) = self.loopback_replies.pop_front() {
            return Some((reply.from, reply.ident, reply.seq));
        }

        // Check ICMP socket for received replies
        let socket = self.sockets.get_mut::<icmp::Socket>(self.icmp_handle);
        
        if socket.can_recv() {
            let mut buf = [0u8; 64];
            if let Ok((size, addr)) = socket.recv_slice(&mut buf) {
                // Parse ICMP echo reply using the received data
                let data = &buf[..size];
                if data.len() >= 8 {
                    let icmp_type = data[0];
                    let _icmp_code = data[1];
                    
                    // Check for echo reply (type 0, code 0)
                    if icmp_type == 0 {
                        let ident = u16::from_be_bytes([data[4], data[5]]);
                        let seq = u16::from_be_bytes([data[6], data[7]]);
                        
                        // Extract source IP from address
                        if let IpAddress::Ipv4(src_ip) = addr {
                            if ident == ICMP_IDENT {
                                return Some((src_ip, ident, seq));
                            }
                        }
                    }
                }
            }
        }
        
        None
    }
}

// =============================================================================
// D1 EMAC Static Buffers (separate from VirtIO buffers)
// =============================================================================

static mut D1_SOCKET_STORAGE: [SocketStorage<'static>; 16] = [SocketStorage::EMPTY; 16];

static mut D1_ICMP_RX_META: [icmp::PacketMetadata; 8] = [icmp::PacketMetadata::EMPTY; 8];
static mut D1_ICMP_RX_DATA: [u8; 512] = [0; 512];
static mut D1_ICMP_TX_META: [icmp::PacketMetadata; 8] = [icmp::PacketMetadata::EMPTY; 8];
static mut D1_ICMP_TX_DATA: [u8; 512] = [0; 512];

static mut D1_UDP_RX_META: [udp::PacketMetadata; 8] = [udp::PacketMetadata::EMPTY; 8];
static mut D1_UDP_RX_DATA: [u8; 1024] = [0; 1024];
static mut D1_UDP_TX_META: [udp::PacketMetadata; 8] = [udp::PacketMetadata::EMPTY; 8];
static mut D1_UDP_TX_DATA: [u8; 1024] = [0; 1024];

static mut D1_TCP_RX_DATA: [u8; 8192] = [0; 8192];
static mut D1_TCP_TX_DATA: [u8; 4096] = [0; 4096];

// Server socket buffers (separate from VirtIO)
static mut D1_TCP_SERVER_RX_DATA: [[u8; 2048]; MAX_SERVER_SOCKETS] = [[0; 2048]; MAX_SERVER_SOCKETS];
static mut D1_TCP_SERVER_TX_DATA: [[u8; 1024]; MAX_SERVER_SOCKETS] = [[0; 1024]; MAX_SERVER_SOCKETS];

