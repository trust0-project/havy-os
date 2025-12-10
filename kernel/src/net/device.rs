//! VirtIO network device wrapper and token implementations.
//!
//! This module provides the smoltcp Device trait implementation for VirtioNet,
//! including RX/TX token handling and TCP packet patching.

use alloc::vec;
use alloc::vec::Vec;

use smoltcp::phy::{Device, DeviceCapabilities, Medium, RxToken, TxToken};
use smoltcp::time::Instant;

use crate::virtio_net::VirtioNet;
use super::patching::{
    get_server_patching, find_server_patching, find_server_patching_mut,
    recalculate_tcp_checksum, SERVER_PATCHING,
    CLIENT_SERVER_SEQ, CLIENT_REMOTE_PORT, CLIENT_EXPECTED_ACK,
    CLIENT_OUR_SEQ, CLIENT_PEER_ACK_EXPECTED,
};

/// Wrapper for VirtioNet to implement smoltcp Device trait
pub struct DeviceWrapper<'a>(pub &'a mut VirtioNet);

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
pub struct VirtioRxToken {
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
            let ip_total_len_header = u16::from_be_bytes([buffer[16], buffer[17]]) as usize;
            // WORKAROUND: Use actual buffer size if ip_total_len seems corrupted
            // This handles cases where the network layer corrupts the IP header
            let actual_ip_len = buffer.len().saturating_sub(14); // 14 = ethernet header
            let ip_total_len = if ip_total_len_header < actual_ip_len / 2 {
                // IP header claims less than half the actual size - it's corrupted
                actual_ip_len
            } else {
                ip_total_len_header
            };
            let tcp_offset = 14 + ip_ihl;
            let tcp_header_len = ((buffer[tcp_offset + 12] >> 4) as usize) * 4;
            
            if buffer.len() >= tcp_offset + 20 {
                let seq = u32::from_be_bytes([buffer[tcp_offset + 4], buffer[tcp_offset + 5], buffer[tcp_offset + 6], buffer[tcp_offset + 7]]);
                let ack = u32::from_be_bytes([buffer[tcp_offset + 8], buffer[tcp_offset + 9], buffer[tcp_offset + 10], buffer[tcp_offset + 11]]);
                let tcp_flags = buffer[tcp_offset + 13];
                let src_port = u16::from_be_bytes([buffer[tcp_offset], buffer[tcp_offset + 1]]);
                let dst_port = u16::from_be_bytes([buffer[tcp_offset + 2], buffer[tcp_offset + 3]]);
                
                // Calculate TCP payload length
                let tcp_len = ip_total_len.saturating_sub(ip_ihl);
                let payload_len = tcp_len.saturating_sub(tcp_header_len);
                
                // SYN and FIN flags consume 1 sequence number each
                let syn_flag = tcp_flags & 0x02 != 0;
                let fin_flag = tcp_flags & 0x01 != 0;
                let ack_flag = tcp_flags & 0x10 != 0;
                let rst_flag = tcp_flags & 0x04 != 0;
                
                // NOTE: We do NOT translate incoming SEQ values
                // smoltcp's internal state correctly tracks the remote's seq from the SYN
                // Only the OUTGOING packet's ack value is buggy (ack=0 or ack=1)
                // We patch outgoing packets in TxToken, not incoming packets here
                
                // Calculate how much sequence space this packet consumes
                let mut seq_consumed = payload_len as u32;
                if syn_flag { seq_consumed += 1; }
                if fin_flag { seq_consumed += 1; }
                
                // Store sequence numbers for patching workaround
                if syn_flag && !ack_flag {
                    // Pure SYN received (SERVER ROLE) - initiate handshake
                    // Use per-port patching state
                    if let Some(state) = get_server_patching(dst_port) {
                        state.last_syn_seq = Some(seq);
                        // Next ACK we send should be seq + 1 (SYN consumes 1 seq)
                        state.expected_ack = Some(seq.wrapping_add(1));
                    }
                } else if syn_flag && ack_flag {
                    // SYN-ACK received (CLIENT ROLE) - server responding
                    unsafe {
                        CLIENT_SERVER_SEQ = Some(seq);
                        CLIENT_REMOTE_PORT = Some(src_port);
                        // Next ACK we send should be seq + 1 (SYN consumes 1 seq)
                        CLIENT_EXPECTED_ACK = Some(seq.wrapping_add(1));
                    }
                }
                
                // Update expected ACK for data we receive (for our outgoing ACKs)
                // IMPORTANT: Skip duplicate/retransmitted packets to avoid state confusion
                if ack_flag && !syn_flag && !rst_flag && payload_len > 0 {
                    // SERVER ROLE: receiving data from client (per-port)
                    if let Some(state) = find_server_patching_mut(dst_port) {
                        let new_expected = seq.wrapping_add(seq_consumed);
                        
                        // Check if this is a duplicate/retransmission by comparing to current expected_ack
                        let is_duplicate = if let Some(current_expected) = state.expected_ack {
                            // If seq + len <= current expected, this is old/duplicate data
                            new_expected <= current_expected || seq < current_expected.wrapping_sub(seq_consumed)
                        } else {
                            false
                        };
                        
                        if !is_duplicate {
                            state.expected_ack = Some(new_expected);
                        }
                    }
                    
                    // CLIENT ROLE: receiving data from server
                    if let Some(remote_port) = unsafe { CLIENT_REMOTE_PORT } {
                        if src_port == remote_port {
                            unsafe {
                                let new_expected = seq.wrapping_add(seq_consumed);
                                CLIENT_EXPECTED_ACK = Some(new_expected);
                            }
                        }
                    }
                }
                
                // Handle FIN packets - they also need ACK tracking
                // But only if there was no data payload (otherwise it was already handled above)
                if fin_flag && ack_flag && !syn_flag && payload_len == 0 {
                    // SERVER ROLE: receiving FIN from client (per-port)
                    if let Some(state) = find_server_patching_mut(dst_port) {
                        // Pure FIN (no data) - needs seq + 1
                        state.expected_ack = Some(seq.wrapping_add(1));
                    }
                    
                    if let Some(remote_port) = unsafe { CLIENT_REMOTE_PORT } {
                        if src_port == remote_port {
                            unsafe {
                                // Pure FIN (no data) - needs seq + 1
                                CLIENT_EXPECTED_ACK = Some(seq.wrapping_add(1));
                            }
                        }
                    }
                }
                
                // =========================================================================
                // PATCH INCOMING ACK - This is done for ALL packets with ACK flag
                // If ACK value is suspiciously low (< 1000), patch it to expected value
                // =========================================================================
                if ack_flag && ack < 1000 && !rst_flag {
                    let mut patched = false;
                    
                    // Try SERVER ROLE patching (we're receiving on our listening port) - PER-PORT
                    if !patched {
                        if let Some(state) = find_server_patching(dst_port) {
                            // Use peer ACK expected (accounts for data we've sent)
                            if let Some(expected) = state.peer_ack_expected {
                                let ack_bytes = expected.to_be_bytes();
                                buffer[tcp_offset + 8] = ack_bytes[0];
                                buffer[tcp_offset + 9] = ack_bytes[1];
                                buffer[tcp_offset + 10] = ack_bytes[2];
                                buffer[tcp_offset + 11] = ack_bytes[3];
                                recalculate_tcp_checksum(&mut buffer, tcp_offset);
                                patched = true;
                            } else if let Some(our_seq) = state.our_seq {
                                // Fallback to initial seq + 1
                                let expected = our_seq.wrapping_add(1);
                                let ack_bytes = expected.to_be_bytes();
                                buffer[tcp_offset + 8] = ack_bytes[0];
                                buffer[tcp_offset + 9] = ack_bytes[1];
                                buffer[tcp_offset + 10] = ack_bytes[2];
                                buffer[tcp_offset + 11] = ack_bytes[3];
                                recalculate_tcp_checksum(&mut buffer, tcp_offset);
                                patched = true;
                            }
                        }
                    }
                    
                    // Try CLIENT ROLE patching (we're receiving from remote server port)
                    if !patched {
                        if let Some(remote_port) = unsafe { CLIENT_REMOTE_PORT } {
                            if src_port == remote_port {
                                // Use peer ACK expected (accounts for data we've sent)
                                if let Some(expected) = unsafe { CLIENT_PEER_ACK_EXPECTED } {
                                    let ack_bytes = expected.to_be_bytes();
                                    buffer[tcp_offset + 8] = ack_bytes[0];
                                    buffer[tcp_offset + 9] = ack_bytes[1];
                                    buffer[tcp_offset + 10] = ack_bytes[2];
                                    buffer[tcp_offset + 11] = ack_bytes[3];
                                    recalculate_tcp_checksum(&mut buffer, tcp_offset);
                                    patched = true;
                                } else if let Some(our_seq) = unsafe { CLIENT_OUR_SEQ } {
                                    // Fallback to initial seq + 1
                                    let expected = our_seq.wrapping_add(1);
                                    let ack_bytes = expected.to_be_bytes();
                                    buffer[tcp_offset + 8] = ack_bytes[0];
                                    buffer[tcp_offset + 9] = ack_bytes[1];
                                    buffer[tcp_offset + 10] = ack_bytes[2];
                                    buffer[tcp_offset + 11] = ack_bytes[3];
                                    recalculate_tcp_checksum(&mut buffer, tcp_offset);
                                    patched = true;
                                }
                            }
                        }
                    }
                    
                    // AGGRESSIVE FALLBACK: If we still haven't patched and ACK is very low,
                    // try any available expected value from per-port state
                    if !patched && ack < 10 {
                        // Try any SERVER port's patching state
                        unsafe {
                            for state in SERVER_PATCHING.iter() {
                                if state.active {
                                    if let Some(expected) = state.peer_ack_expected {
                                        let ack_bytes = expected.to_be_bytes();
                                        buffer[tcp_offset + 8] = ack_bytes[0];
                                        buffer[tcp_offset + 9] = ack_bytes[1];
                                        buffer[tcp_offset + 10] = ack_bytes[2];
                                        buffer[tcp_offset + 11] = ack_bytes[3];
                                        recalculate_tcp_checksum(&mut buffer, tcp_offset);
                                        patched = true;
                                        break;
                                    } else if let Some(our_seq) = state.our_seq {
                                        let expected = our_seq.wrapping_add(1);
                                        let ack_bytes = expected.to_be_bytes();
                                        buffer[tcp_offset + 8] = ack_bytes[0];
                                        buffer[tcp_offset + 9] = ack_bytes[1];
                                        buffer[tcp_offset + 10] = ack_bytes[2];
                                        buffer[tcp_offset + 11] = ack_bytes[3];
                                        recalculate_tcp_checksum(&mut buffer, tcp_offset);
                                        patched = true;
                                        break;
                                    }
                                }
                            }
                        }
                        
                        // Try CLIENT values as last resort
                        if !patched {
                            if let Some(expected) = unsafe { CLIENT_PEER_ACK_EXPECTED } {
                                let ack_bytes = expected.to_be_bytes();
                                buffer[tcp_offset + 8] = ack_bytes[0];
                                buffer[tcp_offset + 9] = ack_bytes[1];
                                buffer[tcp_offset + 10] = ack_bytes[2];
                                buffer[tcp_offset + 11] = ack_bytes[3];
                                recalculate_tcp_checksum(&mut buffer, tcp_offset);
                                patched = true;
                            } else if let Some(our_seq) = unsafe { CLIENT_OUR_SEQ } {
                                let expected = our_seq.wrapping_add(1);
                                let ack_bytes = expected.to_be_bytes();
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
        
        f(&buffer)
    }
}

/// TX token for transmitting packets
pub struct VirtioTxToken<'a> {
    pub device: &'a mut VirtioNet,
}

impl TxToken for VirtioTxToken<'_> {
    fn consume<R, F>(self, len: usize, f: F) -> R
    where
        F: FnOnce(&mut [u8]) -> R,
    {
        let mut buffer = vec![0u8; len];
        let result = f(&mut buffer);

        // Apply TCP patching workaround for smoltcp bug
        if buffer.len() >= 40 && buffer[12] == 0x08 && buffer[13] == 0x00 {
            let ip_ihl = (buffer[14] & 0x0F) as usize * 4;
            let ip_total_len = u16::from_be_bytes([buffer[16], buffer[17]]) as usize;
            let tcp_offset = 14 + ip_ihl;
            
            if buffer.len() >= tcp_offset + 20 && buffer[23] == 6 {
                let tcp_header_len = ((buffer[tcp_offset + 12] >> 4) as usize) * 4;
                let tcp_flags = buffer[tcp_offset + 13];
                let src_port = u16::from_be_bytes([buffer[tcp_offset], buffer[tcp_offset + 1]]);
                let dst_port = u16::from_be_bytes([buffer[tcp_offset + 2], buffer[tcp_offset + 3]]);
                let seq_num = u32::from_be_bytes([buffer[tcp_offset + 4], buffer[tcp_offset + 5], buffer[tcp_offset + 6], buffer[tcp_offset + 7]]);
                let ack_num = u32::from_be_bytes([buffer[tcp_offset + 8], buffer[tcp_offset + 9], buffer[tcp_offset + 10], buffer[tcp_offset + 11]]);
                
                let syn_flag = tcp_flags & 0x02 != 0;
                let ack_flag = tcp_flags & 0x10 != 0;
                let fin_flag = tcp_flags & 0x01 != 0;
                let rst_flag = tcp_flags & 0x04 != 0;
                
                // Calculate TCP payload length (what we're sending)
                let tcp_len = ip_total_len.saturating_sub(ip_ihl);
                let payload_len = tcp_len.saturating_sub(tcp_header_len);
                
                // Calculate sequence space we're consuming (for tracking peer's expected ACK)
                let mut seq_consumed = payload_len as u32;
                if syn_flag { seq_consumed += 1; }  // SYN consumes 1 seq
                if fin_flag { seq_consumed += 1; }  // FIN consumes 1 seq
                
                // Track outgoing SYN (CLIENT ROLE) - pure SYN means we're initiating connection
                if syn_flag && !ack_flag {
                    unsafe {
                        CLIENT_OUR_SEQ = Some(seq_num);
                        // After handshake, peer should ACK our SYN with seq + 1
                        CLIENT_PEER_ACK_EXPECTED = Some(seq_num.wrapping_add(1));
                    }
                }
                
                // SYN-ACK patching (SERVER ROLE) - we're responding to a client's SYN
                // Use per-port patching state
                if syn_flag && ack_flag {
                    // Look up patching state by source port (our listening port)
                    if let Some(state) = find_server_patching_mut(src_port) {
                        state.our_seq = Some(seq_num);
                        state.synack_seq = Some(seq_num);
                        // After handshake, peer should ACK our SYN-ACK with seq + 1
                        state.peer_ack_expected = Some(seq_num.wrapping_add(1));
                        
                        // Save smoltcp's original ack value (before patching)
                        state.smoltcp_ack = Some(ack_num);
                        
                        if let Some(expected_ack) = state.expected_ack {
                            if ack_num != expected_ack {
                                // Calculate seq_offset: correct_seq - smoltcp_seq
                                // This is the offset we need to subtract from incoming SEQ values
                                // so smoltcp sees the seq values it expects
                                state.seq_offset = Some(expected_ack.wrapping_sub(ack_num));
                                
                                // Patch the ack number
                                let ack_bytes = expected_ack.to_be_bytes();
                                buffer[tcp_offset + 8] = ack_bytes[0];
                                buffer[tcp_offset + 9] = ack_bytes[1];
                                buffer[tcp_offset + 10] = ack_bytes[2];
                                buffer[tcp_offset + 11] = ack_bytes[3];
                                recalculate_tcp_checksum(&mut buffer, tcp_offset);
                            }
                        } else if let Some(syn_seq) = state.last_syn_seq {
                            // Fallback to computing from last SYN
                            let correct_ack = syn_seq.wrapping_add(1);
                            if ack_num != correct_ack {
                                // Calculate seq_offset
                                state.seq_offset = Some(correct_ack.wrapping_sub(ack_num));
                                
                                let ack_bytes = correct_ack.to_be_bytes();
                                buffer[tcp_offset + 8] = ack_bytes[0];
                                buffer[tcp_offset + 9] = ack_bytes[1];
                                buffer[tcp_offset + 10] = ack_bytes[2];
                                buffer[tcp_offset + 11] = ack_bytes[3];
                                recalculate_tcp_checksum(&mut buffer, tcp_offset);
                            }
                        }
                    }
                } else if ack_flag && !syn_flag && !rst_flag {
                    // Any packet with ACK flag (includes pure ACK, PSH+ACK, FIN+ACK, data packets)
                    let mut patched = false;
                    
                    // SERVER ROLE: we're sending from our listening port - PER-PORT
                    if let Some(state) = find_server_patching_mut(src_port) {
                        // Update what peer should ACK if we're sending data
                        if seq_consumed > 0 {
                            state.peer_ack_expected = Some(seq_num.wrapping_add(seq_consumed));
                        }
                        
                        // FIX: Patch SEQ value for non-SYN packets
                        // After SYN-ACK (which has seq=Y), pure ACKs should have seq=Y+1
                        // smoltcp incorrectly uses Y instead of Y+1
                        let mut seq_patched = false;
                        if let Some(synack_seq) = state.synack_seq {
                            let expected_seq = synack_seq.wrapping_add(1);
                            // If seq matches SYN-ACK's seq (should be +1), patch it
                            if seq_num == synack_seq {
                                let seq_bytes = expected_seq.to_be_bytes();
                                buffer[tcp_offset + 4] = seq_bytes[0];
                                buffer[tcp_offset + 5] = seq_bytes[1];
                                buffer[tcp_offset + 6] = seq_bytes[2];
                                buffer[tcp_offset + 7] = seq_bytes[3];
                                seq_patched = true;
                            }
                        }
                        
                        // ALWAYS use our tracked expected ACK if it differs from smoltcp's
                        // smoltcp may generate wrong ack values (not just 0/1)
                        if let Some(expected_ack) = state.expected_ack {
                            if ack_num != expected_ack {
                                let ack_bytes = expected_ack.to_be_bytes();
                                buffer[tcp_offset + 8] = ack_bytes[0];
                                buffer[tcp_offset + 9] = ack_bytes[1];
                                buffer[tcp_offset + 10] = ack_bytes[2];
                                buffer[tcp_offset + 11] = ack_bytes[3];
                                patched = true;
                            }
                        } else if let Some(syn_seq) = state.last_syn_seq {
                            // Fallback: use initial SYN seq + 1
                            let correct_ack = syn_seq.wrapping_add(1);
                            if ack_num != correct_ack {
                                let ack_bytes = correct_ack.to_be_bytes();
                                buffer[tcp_offset + 8] = ack_bytes[0];
                                buffer[tcp_offset + 9] = ack_bytes[1];
                                buffer[tcp_offset + 10] = ack_bytes[2];
                                buffer[tcp_offset + 11] = ack_bytes[3];
                                patched = true;
                            }
                        }
                        
                        // Recalculate checksum if we patched either SEQ or ACK
                        if seq_patched || patched {
                            recalculate_tcp_checksum(&mut buffer, tcp_offset);
                        }
                    }
                    
                    // CLIENT ROLE: we're sending TO a remote server port
                    if !patched {
                        if let Some(remote_port) = unsafe { CLIENT_REMOTE_PORT } {
                            if dst_port == remote_port {
                                // Update what peer should ACK if we're sending data
                                if seq_consumed > 0 {
                                    unsafe {
                                        CLIENT_PEER_ACK_EXPECTED = Some(seq_num.wrapping_add(seq_consumed));
                                    }
                                }
                                
                                // ALWAYS use our tracked expected ACK if it differs
                                if let Some(expected_ack) = unsafe { CLIENT_EXPECTED_ACK } {
                                    if ack_num != expected_ack {
                                        let ack_bytes = expected_ack.to_be_bytes();
                                        buffer[tcp_offset + 8] = ack_bytes[0];
                                        buffer[tcp_offset + 9] = ack_bytes[1];
                                        buffer[tcp_offset + 10] = ack_bytes[2];
                                        buffer[tcp_offset + 11] = ack_bytes[3];
                                        recalculate_tcp_checksum(&mut buffer, tcp_offset);
                                    }
                                } else if let Some(server_seq) = unsafe { CLIENT_SERVER_SEQ } {
                                    // Fallback: use initial server seq + 1
                                    let correct_ack = server_seq.wrapping_add(1);
                                    if ack_num != correct_ack {
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
            }
        }

        // Send the packet
        let send_result = self.device.send(&buffer);
        let _ = send_result; // Ignore result to avoid warning


        result
    }
}
