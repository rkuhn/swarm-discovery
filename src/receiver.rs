use crate::{sender::MdnsMsg, Peer};
use acto::{ActoCell, ActoRef, ActoRuntime};
use anyhow::Context;
use hickory_proto::{
    op::Message,
    rr::{DNSClass, Name, RData, RecordType},
};
use std::{
    borrow::Cow, collections::BTreeMap, net::IpAddr, str::FromStr, sync::Arc, time::Instant,
};
use tokio::net::UdpSocket;

pub async fn receiver(
    _ctx: ActoCell<(), impl ActoRuntime>,
    service_name: Name,
    socket: Arc<UdpSocket>,
    target: ActoRef<MdnsMsg>,
) -> anyhow::Result<()> {
    let mut buf = [0; 1472];
    loop {
        let (len, addr) = socket.recv_from(&mut buf).await.context("recv_from")?;
        let msg = &buf[..len];
        tracing::trace!("received {} bytes from {}", len, addr);
        if let Some(msg) = handle_msg(msg, &service_name, addr.ip()) {
            target.send(msg);
        }
    }
}

fn handle_msg(buf: &[u8], service_name: &Name, addr: IpAddr) -> Option<MdnsMsg> {
    let packet = match Message::from_vec(buf) {
        Ok(p) => p,
        Err(e) => {
            tracing::debug!("error parsing mDNS packet: {}", e);
            return None;
        }
    };
    for question in packet.queries() {
        if question.query_class() != DNSClass::IN {
            tracing::trace!(
                "received mDNS query with wrong class {}",
                question.query_class()
            );
            continue;
        }
        if question.query_type() != RecordType::PTR {
            tracing::trace!(
                "received mDNS query with wrong type {}",
                question.query_type()
            );
            continue;
        }
        if question.name() != service_name {
            tracing::trace!("received mDNS query for wrong service {}", question.name());
            continue;
        }
        tracing::debug!("received mDNS query for {}", question.name());
        return Some(match addr {
            IpAddr::V4(_) => MdnsMsg::QueryV4,
            IpAddr::V6(_) => MdnsMsg::QueryV6,
        });
    }

    let local = Name::from_str("local.").unwrap();
    let mut peer_ids: BTreeMap<String, (u16, Vec<IpAddr>)> = BTreeMap::new();
    for response in packet.answers() {
        if response.dns_class() != DNSClass::IN {
            tracing::trace!(
                "received mDNS response with wrong class {:?}",
                response.dns_class()
            );
            continue;
        }
        let name = response.name();
        if name.base_name() != *service_name {
            tracing::trace!("received mDNS response with wrong service {}", name);
            continue;
        }
        tracing::debug!("received mDNS response for {}", name);
        if let Some(RData::SRV(srv)) = response.data() {
            let target = srv.target();
            let target_host = target.iter().next();
            if target_host != name.iter().next() {
                tracing::debug!("received mDNS response for non-matching target {}", target);
                continue;
            }
            let Cow::Borrowed(host) = String::from_utf8_lossy(target_host.unwrap()) else {
                tracing::debug!("received mDNS response with invalid target {:?}", target);
                continue;
            };
            peer_ids.insert(host.to_owned(), (srv.port(), vec![]));
        } else {
            tracing::trace!(
                "received mDNS response with wrong data {:?}",
                response.data()
            );
            continue;
        }
    }
    for additional in packet.additionals() {
        if additional.dns_class() != DNSClass::IN {
            tracing::trace!(
                "received mDNS additional with wrong class {:?}",
                additional.dns_class()
            );
            continue;
        }
        let name = additional.name();
        if name.base_name() != local {
            tracing::trace!("received mDNS additional for wrong service {}", name);
            continue;
        }
        tracing::trace!("received mDNS additional for {}", name);
        let Cow::Borrowed(name) = String::from_utf8_lossy(name.iter().next().unwrap()) else {
            tracing::debug!("received mDNS response with invalid target {:?}", name);
            continue;
        };
        match additional.data() {
            Some(RData::A(a)) => {
                if let Some((_, addrs)) = peer_ids.get_mut(name) {
                    addrs.push(a.0.into());
                }
            }
            Some(RData::AAAA(a)) => {
                if let Some((_, addrs)) = peer_ids.get_mut(name) {
                    addrs.push(a.0.into());
                }
            }
            _ => {
                tracing::debug!(
                    "received mDNS additional with wrong data {:?}",
                    additional.data()
                );
                continue;
            }
        }
    }
    let mut ret = BTreeMap::new();
    for (peer_id, (port, mut addrs)) in peer_ids {
        addrs.sort();
        let last_seen = Instant::now();
        let peer = Peer {
            port,
            addrs,
            last_seen,
        };
        ret.insert(peer_id, peer);
    }
    Some(MdnsMsg::Response(ret))
}
