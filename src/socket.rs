use crate::IpClass;
use hickory_proto::op::Message;
use socket2::{Domain, Protocol, SockRef, Socket, Type};
use std::{
    collections::HashMap,
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV4},
    sync::{Arc, RwLock},
};
use thiserror::Error;
use tokio::net::UdpSocket;

pub const MDNS_PORT: u16 = 5353;
pub const MDNS_IPV4: Ipv4Addr = Ipv4Addr::new(224, 0, 0, 251);
pub const MDNS_IPV6: Ipv6Addr = Ipv6Addr::new(0xff02, 0, 0, 0, 0, 0, 0, 0xfb);

#[derive(Debug)]
#[doc(hidden)]
pub enum IP {
    Ipv4,
    Ipv6,
}

impl std::fmt::Display for IP {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            IP::Ipv4 => write!(f, "IPv4"),
            IP::Ipv6 => write!(f, "IPv6"),
        }
    }
}

/// Errors that can occur when creating and configuring an IPv4 or IPv6 socket.
#[derive(Debug, Error)]
pub enum SocketError {
    #[error("{domain}: error creating new socket for")]
    NewSocket {
        domain: IP,
        #[source]
        source: std::io::Error,
    },
    #[error("{domain}: error setting the reuse address")]
    ReuseAddress {
        domain: IP,
        #[source]
        source: std::io::Error,
    },
    #[cfg(unix)]
    #[error("{domain}: error setting the reuse port")]
    ReusePort {
        domain: IP,
        #[source]
        source: std::io::Error,
    },
    #[error("{domain}: error binding the socket for")]
    Bind {
        domain: IP,
        #[source]
        source: std::io::Error,
    },
    #[error("{domain}: error setting multicast loop")]
    SetMulticastLoop {
        domain: IP,
        #[source]
        source: std::io::Error,
    },
    #[error("{domain}: error joining multicast")]
    JoinMulticast {
        domain: IP,
        #[source]
        source: std::io::Error,
    },
    #[error("{domain}: error setting the multicast ttl")]
    MulticastTtl {
        domain: IP,
        #[source]
        source: std::io::Error,
    },
    #[error("{domain}: error setting the socket to non-blocking mode")]
    SetNonBlocking {
        domain: IP,
        #[source]
        source: std::io::Error,
    },
    #[error("{domain}: error creating a UDP socket from a standard socket")]
    UdpSocket {
        domain: IP,
        #[source]
        source: std::io::Error,
    },
    #[error("Cannot bind to IPv4 or IPv6")]
    CannotBind,
}

/// Create a send-only socket for a specific interface.
///
/// This socket is bound to an ephemeral port on the interface for sending only.
pub fn socket_v4_tx(interface_addr: Ipv4Addr) -> Result<UdpSocket, SocketError> {
    // Bind to the specific interface with ephemeral port for sending
    let bind_addr = SocketAddrV4::new(interface_addr, 0).into();

    let socket = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP)).map_err(|source| {
        SocketError::NewSocket {
            domain: IP::Ipv4,
            source,
        }
    })?;
    socket
        .set_reuse_address(true)
        .map_err(|source| SocketError::ReuseAddress {
            domain: IP::Ipv4,
            source,
        })?;
    socket
        .bind(&bind_addr)
        .map_err(|source| SocketError::Bind {
            domain: IP::Ipv4,
            source,
        })?;
    socket
        .set_multicast_loop_v4(true)
        .map_err(|source| SocketError::SetMulticastLoop {
            domain: IP::Ipv4,
            source,
        })?;
    socket
        .set_multicast_ttl_v4(16)
        .map_err(|source| SocketError::MulticastTtl {
            domain: IP::Ipv4,
            source,
        })?;
    socket
        .set_nonblocking(true)
        .map_err(|source| SocketError::SetNonBlocking {
            domain: IP::Ipv4,
            source,
        })?;
    UdpSocket::from_std(std::net::UdpSocket::from(socket)).map_err(|source| {
        SocketError::UdpSocket {
            domain: IP::Ipv4,
            source,
        }
    })
}

/// Create a receive socket that joins multicast on all interfaces.
pub fn socket_v4_rx(interfaces: &[Ipv4Addr]) -> Result<UdpSocket, SocketError> {
    // CRITICAL: Always bind to 0.0.0.0:5353 for receiving multicast packets.
    // This allows the socket to receive multicast from ANY interface.
    let bind_addr = SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, MDNS_PORT).into();

    let socket = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP)).map_err(|source| {
        SocketError::NewSocket {
            domain: IP::Ipv4,
            source,
        }
    })?;
    socket
        .set_reuse_address(true)
        .map_err(|source| SocketError::ReuseAddress {
            domain: IP::Ipv4,
            source,
        })?;
    #[cfg(unix)]
    socket
        .set_reuse_port(true)
        .map_err(|source| SocketError::ReusePort {
            domain: IP::Ipv4,
            source,
        })?;
    socket
        .bind(&bind_addr)
        .map_err(|source| SocketError::Bind {
            domain: IP::Ipv4,
            source,
        })?;
    socket
        .set_multicast_loop_v4(true)
        .map_err(|source| SocketError::SetMulticastLoop {
            domain: IP::Ipv4,
            source,
        })?;
    socket
        .set_multicast_ttl_v4(16)
        .map_err(|source| SocketError::MulticastTtl {
            domain: IP::Ipv4,
            source,
        })?;
    socket
        .set_nonblocking(true)
        .map_err(|source| SocketError::SetNonBlocking {
            domain: IP::Ipv4,
            source,
        })?;

    // Join multicast group on ALL specified interfaces
    if interfaces.is_empty() {
        // No specific interfaces - join on default
        socket
            .join_multicast_v4(&MDNS_IPV4, &Ipv4Addr::UNSPECIFIED)
            .map_err(|source| SocketError::JoinMulticast {
                domain: IP::Ipv4,
                source,
            })?;
        tracing::debug!("Joined multicast on default interface");
    } else {
        // Join on each specified interface
        for interface_addr in interfaces {
            socket
                .join_multicast_v4(&MDNS_IPV4, interface_addr)
                .map_err(|source| SocketError::JoinMulticast {
                    domain: IP::Ipv4,
                    source,
                })?;
            tracing::info!(
                "Joined multicast group 224.0.0.251 on interface {}",
                interface_addr
            );
        }
    }

    UdpSocket::from_std(std::net::UdpSocket::from(socket)).map_err(|source| {
        SocketError::UdpSocket {
            domain: IP::Ipv4,
            source,
        }
    })
}

pub fn socket_v6() -> Result<UdpSocket, SocketError> {
    let socket = Socket::new(Domain::IPV6, Type::DGRAM, Some(Protocol::UDP)).map_err(|source| {
        SocketError::NewSocket {
            domain: IP::Ipv6,
            source,
        }
    })?;
    socket
        .set_reuse_address(true)
        .map_err(|source| SocketError::ReuseAddress {
            domain: IP::Ipv6,
            source,
        })?;
    #[cfg(unix)]
    socket
        .set_reuse_port(true)
        .map_err(|source| SocketError::ReusePort {
            domain: IP::Ipv6,
            source,
        })?;
    socket
        .bind(&SocketAddr::from((Ipv6Addr::UNSPECIFIED, MDNS_PORT)).into())
        .map_err(|source| SocketError::Bind {
            domain: IP::Ipv6,
            source,
        })?;
    socket
        .set_multicast_loop_v6(true)
        .map_err(|source| SocketError::SetMulticastLoop {
            domain: IP::Ipv6,
            source,
        })?;

    // Join multicast on the default interface (interface index 0)
    socket
        .join_multicast_v6(&MDNS_IPV6, 0)
        .map_err(|source| SocketError::JoinMulticast {
            domain: IP::Ipv6,
            source,
        })?;

    socket
        .set_nonblocking(true)
        .map_err(|source| SocketError::SetNonBlocking {
            domain: IP::Ipv6,
            source,
        })?;
    UdpSocket::from_std(std::net::UdpSocket::from(socket)).map_err(|source| {
        SocketError::UdpSocket {
            domain: IP::Ipv6,
            source,
        }
    })
}

#[derive(Clone, Debug)]
pub struct Sockets {
    v4: Option<Arc<UdpSocket>>,
    v6: Option<Arc<UdpSocket>>,
    interface_sockets_v4: Arc<RwLock<HashMap<Ipv4Addr, Arc<UdpSocket>>>>,
}

impl Sockets {
    pub fn new(class: IpClass, multicast_interfaces: Vec<Ipv4Addr>) -> Result<Self, SocketError> {
        // Create one shared receive socket that joins multicast on all interfaces
        let v4_socket = if class.has_v4() || matches!(class, IpClass::Auto) {
            // Create socket that joins multicast on all specified interfaces
            let socket = socket_v4_rx(&multicast_interfaces)?;
            Some(Arc::new(socket))
        } else {
            None
        };

        // Create interface specific sockets for sending only
        let mut interface_sockets_v4 = HashMap::new();
        for addr in &multicast_interfaces {
            match socket_v4_tx(*addr) {
                Ok(socket) => {
                    tracing::debug!("Created send-only socket for interface {}", addr);
                    interface_sockets_v4.insert(*addr, Arc::new(socket));
                }
                Err(e) => {
                    tracing::warn!("Failed to create send socket for interface {}: {}", addr, e);
                }
            }
        }
        let interface_sockets_v4 = Arc::new(RwLock::new(interface_sockets_v4));

        match class {
            IpClass::Auto => {
                let socket = Self {
                    v4: v4_socket,
                    v6: socket_v6().ok().map(Arc::new),
                    interface_sockets_v4: interface_sockets_v4.clone(),
                };
                if socket.v4.is_none() && socket.v6.is_none() {
                    return Err(SocketError::CannotBind);
                }
                Ok(socket)
            }
            _ => Ok(Self {
                v4: v4_socket,
                v6: class
                    .has_v6()
                    .then(|| socket_v6().map(Arc::new))
                    .transpose()?,
                interface_sockets_v4: interface_sockets_v4.clone(),
            }),
        }
    }

    pub fn v4(&self) -> Option<Arc<UdpSocket>> {
        self.v4.as_ref().map(Arc::clone)
    }

    pub fn v6(&self) -> Option<Arc<UdpSocket>> {
        self.v6.as_ref().map(Arc::clone)
    }

    /// Add a new IPv4 interface for multicast operations.
    /// Returns Ok(()) if the socket was successfully created and added.
    pub fn add_interface_v4(&self, addr: Ipv4Addr) -> Result<(), SocketError> {
        let mut interfaces = self.interface_sockets_v4.write().unwrap();
        if interfaces.contains_key(&addr) {
            return Ok(());
        }

        // Create the interface-specific socket for sending
        let socket = socket_v4_tx(addr)?;

        // Join multicast on the shared RX socket so we receive packets on this interface
        self.join_multicast_v4(&addr)?;

        interfaces.insert(addr, Arc::new(socket));
        tracing::info!("Added interface {} for multicast", addr);
        Ok(())
    }

    /// Remove an IPv4 interface from multicast operations.
    /// Returns true if the interface was found and removed.
    pub fn remove_interface_v4(&self, addr: Ipv4Addr) -> bool {
        let mut interfaces = self.interface_sockets_v4.write().unwrap();

        if let Some(socket) = interfaces.remove(&addr) {
            // Leave multicast while still holding the lock to prevent a concurrent
            // add_interface_v4 from re-joining before we finish leaving.
            self.leave_multicast_v4(&addr);

            tracing::info!("Removed interface {} from multicast", addr);

            // Drop the lock before dropping the socket to avoid holding it longer than needed.
            drop(interfaces);
            drop(socket);

            true
        } else {
            false
        }
    }

    pub async fn send_msg(&self, msg: &Message, mode: Mode) {
        let bytes = match msg.to_vec() {
            Ok(b) => b,
            Err(e) => {
                tracing::warn!("error serializing mDNS: {}", e);
                return;
            }
        };

        // Use multi-interface mode only for IPv4 when interface sockets are available
        let use_multi_interface = !self.interface_sockets_v4.read().unwrap().is_empty()
            && matches!(mode, Mode::V4 | Mode::Any);

        if use_multi_interface {
            tracing::debug!(
                "Using multi-interface mode for IPv4 sending, {} interfaces available",
                self.interface_sockets_v4.read().unwrap().len()
            );
            self.send_msg_multi_interface_v4(&bytes, msg).await;

            // If mode is Any, also send on IPv6 if available
            if matches!(mode, Mode::Any) {
                if let Some(v6) = &self.v6 {
                    if let Err(e) = v6.send_to(&bytes, (MDNS_IPV6, MDNS_PORT)).await {
                        tracing::warn!("error sending mDNS on IPv6: {}", e);
                    } else {
                        tracing::debug!(
                            q = msg.queries().len(),
                            an = msg.answers().len(),
                            ad = msg.additionals().len(),
                            "sent {} bytes on IPv6",
                            bytes.len()
                        );
                    }
                }
            }
        } else {
            // Single interface mode or IPv6-only
            let (socket, addr) = match mode {
                Mode::V4 => (self.v4.as_ref().unwrap(), IpAddr::from(MDNS_IPV4)),
                Mode::V6 => (self.v6.as_ref().unwrap(), IpAddr::from(MDNS_IPV6)),
                Mode::Any => {
                    if let Some(v4) = &self.v4 {
                        (v4, IpAddr::from(MDNS_IPV4))
                    } else {
                        (self.v6.as_ref().unwrap(), IpAddr::from(MDNS_IPV6))
                    }
                }
            };
            if let Err(e) = socket.send_to(&bytes, (addr, MDNS_PORT)).await {
                tracing::warn!("error sending mDNS: {}", e);
            } else {
                tracing::debug!(
                    q = msg.queries().len(),
                    an = msg.answers().len(),
                    ad = msg.additionals().len(),
                    "sent {} bytes",
                    bytes.len()
                );
            }
        }
    }

    async fn send_msg_multi_interface_v4(&self, bytes: &[u8], msg: &Message) {
        let mut sent_count = 0;

        // Send on all IPv4 interface-specific sockets
        let interfaces = self.interface_sockets_v4.read().unwrap().clone();
        for (addr, socket) in interfaces.iter() {
            if let Err(e) = socket.send_to(bytes, (MDNS_IPV4, MDNS_PORT)).await {
                tracing::error!("error sending mDNS on interface {}: {}", addr, e);
            } else {
                tracing::debug!(
                    addr = %addr,
                    q = msg.queries().len(),
                    an = msg.answers().len(),
                    ad = msg.additionals().len(),
                    "sent {} bytes on interface",
                    bytes.len()
                );
                sent_count += 1;
            }
        }

        if sent_count == 0 {
            tracing::error!("failed to send mDNS on any IPv4 interface in multi-interface mode");
        }
    }

    fn join_multicast_v4(&self, addr: &Ipv4Addr) -> Result<(), SocketError> {
        // Join multicast on the shared RX socket so we receive packets on this interface
        if let Some(v4) = &self.v4 {
            let sock_ref = SockRef::from(v4.as_ref());
            sock_ref
                .join_multicast_v4(&MDNS_IPV4, addr)
                .map_err(|source| SocketError::JoinMulticast {
                    domain: IP::Ipv4,
                    source,
                })?;
        }

        Ok(())
    }

    fn leave_multicast_v4(&self, addr: &Ipv4Addr) {
        if let Some(v4) = &self.v4 {
            let sock_ref = SockRef::from(v4.as_ref());
            if let Err(e) = sock_ref.leave_multicast_v4(&MDNS_IPV4, addr) {
                tracing::warn!("Failed to leave multicast on interface {}: {}", addr, e);
            }
        }
    }
}

#[derive(Debug)]
pub enum Mode {
    V4,
    V6,
    Any,
}
