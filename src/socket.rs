use crate::IpClass;
use anyhow::{bail, Context};
use hickory_proto::op::Message;
use socket2::{Domain, Protocol, Socket, Type};
use std::{
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV4},
    sync::Arc,
};
use tokio::net::UdpSocket;

pub const MDNS_PORT: u16 = 5353;
pub const MDNS_IPV4: Ipv4Addr = Ipv4Addr::new(224, 0, 0, 251);
pub const MDNS_IPV6: Ipv6Addr = Ipv6Addr::new(0xff02, 0, 0, 0, 0, 0, 0, 0xfb);

pub fn socket_v4() -> anyhow::Result<UdpSocket> {
    let socket =
        Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP)).context("Socket::new")?;
    socket
        .set_reuse_address(true)
        .context("set_reuse_address")?;
    #[cfg(unix)]
    socket.set_reuse_port(true).context("set_reuse_port")?;
    socket
        .bind(&SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, MDNS_PORT).into())
        .context("bind")?;
    socket
        .set_multicast_loop_v4(true)
        .context("set_multicast_loop_v4")?;
    socket
        .join_multicast_v4(&MDNS_IPV4, &Ipv4Addr::UNSPECIFIED)
        .context("join_multicast_v4")?;
    socket
        .set_multicast_ttl_v4(16)
        .context("set_multicast_ttl_v4")?;
    socket.set_nonblocking(true).context("set_nonblocking")?;
    UdpSocket::from_std(std::net::UdpSocket::from(socket)).context("from_std")
}

pub fn socket_v6() -> anyhow::Result<UdpSocket> {
    let socket =
        Socket::new(Domain::IPV6, Type::DGRAM, Some(Protocol::UDP)).context("Socket::new")?;
    socket
        .set_reuse_address(true)
        .context("set_reuse_address")?;
    #[cfg(unix)]
    socket.set_reuse_port(true).context("set_reuse_port")?;
    socket
        .bind(&SocketAddr::from((Ipv6Addr::UNSPECIFIED, MDNS_PORT)).into())
        .context("bind")?;
    socket
        .set_multicast_loop_v6(true)
        .context("set_multicast_loop_v6")?;
    socket
        .join_multicast_v6(&MDNS_IPV6, 0)
        .context("join_multicast_v6")?;
    socket.set_nonblocking(true).context("set_nonblocking")?;
    UdpSocket::from_std(std::net::UdpSocket::from(socket)).context("from_std")
}

#[derive(Clone, Debug)]
pub struct Sockets {
    v4: Option<Arc<UdpSocket>>,
    v6: Option<Arc<UdpSocket>>,
}

impl Sockets {
    pub fn new(class: IpClass) -> anyhow::Result<Self> {
        match class {
            IpClass::Auto => {
                let socket = Self {
                    v4: socket_v4().ok().map(Arc::new),
                    v6: socket_v6().ok().map(Arc::new),
                };
                if socket.v4.is_none() && socket.v6.is_none() {
                    bail!("Socket cannot bind to ipv4 or ipv6");
                }
                Ok(socket)
            }
            _ => Ok(Self {
                v4: class
                    .has_v4()
                    .then(|| socket_v4().context("socket_v4").map(Arc::new))
                    .transpose()?,
                v6: class
                    .has_v6()
                    .then(|| socket_v6().context("socket_v6").map(Arc::new))
                    .transpose()?,
            }),
        }
    }

    pub fn v4(&self) -> Option<Arc<UdpSocket>> {
        self.v4.as_ref().map(Arc::clone)
    }

    pub fn v6(&self) -> Option<Arc<UdpSocket>> {
        self.v6.as_ref().map(Arc::clone)
    }

    pub async fn send_msg(&self, msg: &Message, mode: Mode) {
        let bytes = match msg.to_vec() {
            Ok(b) => b,
            Err(e) => {
                tracing::warn!("error serializing mDNS: {}", e);
                return;
            }
        };
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

pub enum Mode {
    V4,
    V6,
    Any,
}
