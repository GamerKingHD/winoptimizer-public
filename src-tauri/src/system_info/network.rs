//! Network monitor - native adapter info via GetAdaptersAddresses
//! and active TCP connections via GetExtendedTcpTable.
//!
//! Uses native Win32 APIs for high performance and zero subprocess overhead.

use serde::{Deserialize, Serialize};
use sysinfo::System;
use windows::Win32::NetworkManagement::IpHelper::{
    GetAdaptersAddresses, GetExtendedTcpTable, GAA_FLAG_INCLUDE_PREFIX, IP_ADAPTER_ADDRESSES_LH,
    MIB_TCP6ROW_OWNER_PID, MIB_TCP6TABLE_OWNER_PID, MIB_TCPROW_OWNER_PID, MIB_TCPTABLE_OWNER_PID,
    TCP_TABLE_OWNER_PID_ALL,
};
use windows::Win32::Networking::WinSock::{AF_INET, AF_INET6, AF_UNSPEC};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkAdapter {
    pub name: String,
    pub description: String,
    pub ip_addresses: Vec<String>,
    pub mac_address: String,
    pub gateway: String,
    pub dhcp_server: String,
    pub dns_servers: Vec<String>,
    pub operational: bool,
    pub mtu: u32,
    pub link_speed_mbps: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TcpConnection {
    pub local_addr: String,
    pub local_port: u16,
    pub remote_addr: String,
    pub remote_port: u16,
    pub state: String,
    pub pid: u32,
    pub process_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkInfo {
    pub adapters: Vec<NetworkAdapter>,
    pub connections: Vec<TcpConnection>,
}

/// Gather full network info - adapters + active connections (parallelized).
pub fn gather() -> NetworkInfo {
    let (adapters, connections) = rayon::join(list_adapters, list_connections);
    NetworkInfo {
        adapters,
        connections,
    }
}

/// List network adapters by querying Win32 GetAdaptersAddresses.
pub fn list_adapters() -> Vec<NetworkAdapter> {
    let mut adapters = Vec::new();
    let mut len: u32 = 15000; // Start with 15KB guess buffer
    let mut buffer = vec![0u8; len as usize];

    unsafe {
        let mut rc = GetAdaptersAddresses(
            AF_UNSPEC.0 as u32,
            GAA_FLAG_INCLUDE_PREFIX,
            None,
            Some(buffer.as_mut_ptr() as *mut IP_ADAPTER_ADDRESSES_LH),
            &mut len,
        );

        if rc == windows::Win32::Foundation::ERROR_BUFFER_OVERFLOW.0 {
            buffer = vec![0u8; len as usize];
            rc = GetAdaptersAddresses(
                AF_UNSPEC.0 as u32,
                GAA_FLAG_INCLUDE_PREFIX,
                None,
                Some(buffer.as_mut_ptr() as *mut IP_ADAPTER_ADDRESSES_LH),
                &mut len,
            );
        }

        if rc == windows::Win32::Foundation::NO_ERROR.0 {
            let mut curr = buffer.as_ptr() as *const IP_ADAPTER_ADDRESSES_LH;
            while !curr.is_null() {
                let adapter = &*curr;

                let name = if adapter.AdapterName.is_null() {
                    String::new()
                } else {
                    std::ffi::CStr::from_ptr(adapter.AdapterName.0 as *const i8)
                        .to_string_lossy()
                        .into_owned()
                };

                let friendly_name = if adapter.FriendlyName.is_null() {
                    String::new()
                } else {
                    adapter.FriendlyName.to_string().unwrap_or_default()
                };

                let description = if adapter.Description.is_null() {
                    String::new()
                } else {
                    adapter.Description.to_string().unwrap_or_default()
                };

                let mac_len = adapter.PhysicalAddressLength as usize;
                let mac_address = if mac_len > 0 {
                    adapter.PhysicalAddress[..mac_len.min(8)]
                        .iter()
                        .map(|b| format!("{:02x}", b))
                        .collect::<Vec<String>>()
                        .join(":")
                } else {
                    String::new()
                };

                // Traverse unicast addresses
                let ip_addresses = collect_unicast_addresses(adapter.FirstUnicastAddress);

                // Traverse gateways
                let gateway = collect_gateway_addresses(adapter.FirstGatewayAddress);

                // Traverse DNS servers
                let dns_servers = collect_dns_addresses(adapter.FirstDnsServerAddress);

                // Traverse DHCP servers
                let dhcp_server = if adapter.Dhcpv4Server.lpSockaddr.is_null() {
                    String::new()
                } else {
                    sockaddr_to_string(adapter.Dhcpv4Server.lpSockaddr).unwrap_or_default()
                };

                let operational = adapter.OperStatus.0 == 1; // IfOperStatusUp is 1
                let mtu = adapter.Mtu;
                let link_speed_mbps = adapter.TransmitLinkSpeed / 1_000_000;

                // Skip loopback and disconnected adapters without MAC or IP
                if !mac_address.is_empty() || !ip_addresses.is_empty() {
                    let adapter_name = if friendly_name.is_empty() {
                        name
                    } else {
                        friendly_name
                    };
                    adapters.push(NetworkAdapter {
                        name: adapter_name,
                        description,
                        ip_addresses,
                        mac_address,
                        gateway,
                        dhcp_server,
                        dns_servers,
                        operational,
                        mtu,
                        link_speed_mbps,
                    });
                }

                curr = adapter.Next;
            }
        }
    }

    adapters
}

/// List active TCP connections natively.
pub fn list_connections() -> Vec<TcpConnection> {
    let mut connections = Vec::new();

    // Setup process name lookup map
    let mut sys = System::new();
    sys.refresh_processes_specifics(
        sysinfo::ProcessesToUpdate::All,
        true,
        sysinfo::ProcessRefreshKind::nothing().with_exe(sysinfo::UpdateKind::OnlyIfNotSet),
    );

    // Query IPv4 TCP Table
    let mut size_v4 = 0;
    unsafe {
        let _ = GetExtendedTcpTable(
            None,
            &mut size_v4,
            false,
            AF_INET.0 as u32,
            TCP_TABLE_OWNER_PID_ALL,
            0,
        );
    }
    let mut buf_v4 = vec![0u8; size_v4 as usize];
    let res_v4 = unsafe {
        GetExtendedTcpTable(
            Some(buf_v4.as_mut_ptr() as *mut _),
            &mut size_v4,
            false,
            AF_INET.0 as u32,
            TCP_TABLE_OWNER_PID_ALL,
            0,
        )
    };

    if res_v4 == windows::Win32::Foundation::NO_ERROR.0 {
        let table = unsafe { &*(buf_v4.as_ptr() as *const MIB_TCPTABLE_OWNER_PID) };
        let num_entries = table.dwNumEntries as usize;
        let row_ptr = &table.table as *const MIB_TCPROW_OWNER_PID;
        for i in 0..num_entries {
            let row = unsafe { &*row_ptr.add(i) };
            let local_addr = format_ipv4(row.dwLocalAddr);
            let local_port = u16::from_be(row.dwLocalPort as u16);
            let remote_addr = format_ipv4(row.dwRemoteAddr);
            let remote_port = u16::from_be(row.dwRemotePort as u16);
            let state = tcp_state_to_string(row.dwState);
            let pid = row.dwOwningPid;
            let process_name = get_process_name(pid, &sys);

            connections.push(TcpConnection {
                local_addr,
                local_port,
                remote_addr,
                remote_port,
                state,
                pid,
                process_name,
            });
        }
    }

    // Query IPv6 TCP Table
    let mut size_v6 = 0;
    unsafe {
        let _ = GetExtendedTcpTable(
            None,
            &mut size_v6,
            false,
            AF_INET6.0 as u32,
            TCP_TABLE_OWNER_PID_ALL,
            0,
        );
    }
    let mut buf_v6 = vec![0u8; size_v6 as usize];
    let res_v6 = unsafe {
        GetExtendedTcpTable(
            Some(buf_v6.as_mut_ptr() as *mut _),
            &mut size_v6,
            false,
            AF_INET6.0 as u32,
            TCP_TABLE_OWNER_PID_ALL,
            0,
        )
    };

    if res_v6 == windows::Win32::Foundation::NO_ERROR.0 {
        let table = unsafe { &*(buf_v6.as_ptr() as *const MIB_TCP6TABLE_OWNER_PID) };
        let num_entries = table.dwNumEntries as usize;
        let row_ptr = &table.table as *const MIB_TCP6ROW_OWNER_PID;
        for i in 0..num_entries {
            let row = unsafe { &*row_ptr.add(i) };
            let local_addr = format_ipv6(row.ucLocalAddr);
            let local_port = u16::from_be(row.dwLocalPort as u16);
            let remote_addr = format_ipv6(row.ucRemoteAddr);
            let remote_port = u16::from_be(row.dwRemotePort as u16);
            let state = tcp_state_to_string(row.dwState);
            let pid = row.dwOwningPid;
            let process_name = get_process_name(pid, &sys);

            connections.push(TcpConnection {
                local_addr,
                local_port,
                remote_addr,
                remote_port,
                state,
                pid,
                process_name,
            });
        }
    }

    connections
}

// -- Helpers ---------------------------------------------

unsafe fn sockaddr_to_string(
    sa: *mut windows::Win32::Networking::WinSock::SOCKADDR,
) -> Option<String> {
    if sa.is_null() {
        return None;
    }
    use windows::Win32::Networking::WinSock::{SOCKADDR_IN, SOCKADDR_IN6};

    let family = (*sa).sa_family;
    if family == AF_INET {
        let sa_in = sa as *const SOCKADDR_IN;
        let ip_u32 = (*sa_in).sin_addr.S_un.S_addr;
        Some(format_ipv4(ip_u32))
    } else if family == AF_INET6 {
        let sa_in6 = sa as *const SOCKADDR_IN6;
        let bytes = (*sa_in6).sin6_addr.u.Byte;
        Some(format_ipv6(bytes))
    } else {
        None
    }
}

unsafe fn collect_unicast_addresses(
    mut ptr: *mut windows::Win32::NetworkManagement::IpHelper::IP_ADAPTER_UNICAST_ADDRESS_LH,
) -> Vec<String> {
    let mut addrs = Vec::new();
    while !ptr.is_null() {
        if let Some(addr_str) = sockaddr_to_string((*ptr).Address.lpSockaddr) {
            addrs.push(addr_str);
        }
        ptr = (*ptr).Next;
    }
    addrs
}

unsafe fn collect_gateway_addresses(
    mut ptr: *mut windows::Win32::NetworkManagement::IpHelper::IP_ADAPTER_GATEWAY_ADDRESS_LH,
) -> String {
    while !ptr.is_null() {
        if let Some(addr_str) = sockaddr_to_string((*ptr).Address.lpSockaddr) {
            return addr_str; // return first gateway
        }
        ptr = (*ptr).Next;
    }
    String::new()
}

unsafe fn collect_dns_addresses(
    mut ptr: *mut windows::Win32::NetworkManagement::IpHelper::IP_ADAPTER_DNS_SERVER_ADDRESS_XP,
) -> Vec<String> {
    let mut addrs = Vec::new();
    while !ptr.is_null() {
        if let Some(addr_str) = sockaddr_to_string((*ptr).Address.lpSockaddr) {
            addrs.push(addr_str);
        }
        ptr = (*ptr).Next;
    }
    addrs
}

fn format_ipv4(ip: u32) -> String {
    let b = ip.to_ne_bytes();
    format!("{}.{}.{}.{}", b[0], b[1], b[2], b[3])
}

fn format_ipv6(addr: [u8; 16]) -> String {
    let ip = std::net::Ipv6Addr::from(addr);
    ip.to_string()
}

fn tcp_state_to_string(state: u32) -> String {
    match state {
        1 => "CLOSED".to_string(),
        2 => "LISTEN".to_string(),
        3 => "SYN_SENT".to_string(),
        4 => "SYN_RECEIVED".to_string(),
        5 => "ESTABLISHED".to_string(),
        6 => "FIN_WAIT_1".to_string(),
        7 => "FIN_WAIT_2".to_string(),
        8 => "CLOSE_WAIT".to_string(),
        9 => "CLOSING".to_string(),
        10 => "LAST_ACK".to_string(),
        11 => "TIME_WAIT".to_string(),
        12 => "DELETE_TCB".to_string(),
        _ => format!("UNKNOWN ({})", state),
    }
}

fn get_process_name(pid: u32, sys: &System) -> String {
    if pid == 0 {
        return "System Idle".into();
    }
    if pid == 4 {
        return "System".into();
    }

    sys.process(sysinfo::Pid::from(pid as usize))
        .map(|p| p.name().to_string_lossy().to_string())
        .unwrap_or_else(|| format!("PID {}", pid))
}
