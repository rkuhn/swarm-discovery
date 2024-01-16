use anyhow::Context;
use socket2::{Domain, Protocol, Socket, Type};
use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV4};
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
    Ok(UdpSocket::from_std(std::net::UdpSocket::from(socket)).context("from_std")?)
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
    Ok(UdpSocket::from_std(std::net::UdpSocket::from(socket)).context("from_std")?)
}
