//! # Algorithm
//!
//! Configuration parameters: τ (discovery time target), φ (response frequency target)
//!
//! Each node sends mDNS queries according to the following algorithm:
//!
//! 1. set timeout to a duration pulled at random from the interval [τ, 2τ].
//! 2. upon reception of another node’s mDNS query go back to 1.
//! 3. upon timeout, send mDNS query and go back to 1.
//!
//! Each node responds to a mDNS query according to the following algorithm:
//!
//! 1. at reception of mDNS query, set timeout to a duration pulled at random from the interval [0, 100ms].
//! 2. set counter to zero.
//! 3. upon reception of another node’s mDNS response to this query, increment the counter.
//! 4. when counter is greater than τ×φ end procedure.
//! 5. upon timeout send response.
//!
//! # mDNS usage
//!
//! - configurable service name NAME
//! - queries sent for PTR records of the form _NAME._tcp.local
//! - responses give SRV records of the form PEER_ID._NAME._tcp.local -> PEER_ID.local (and associated A/AAAA records)

mod socket;

use anyhow::Context;
use hickory_proto::{
    op::{Message, MessageType, Query},
    rr::{rdata, DNSClass, Name, RData, Record, RecordType},
};
use rand::{thread_rng, Rng};
use socket::{MDNS_IPV4, MDNS_PORT};
use std::{borrow::Cow, collections::BTreeMap, net::IpAddr, str::FromStr, time::Duration};
use tokio::{net::UdpSocket, pin, runtime::Handle, task::JoinHandle};

pub struct Discoverer {
    name: String,
    peer_id: String,
    peers: BTreeMap<String, Peer>,
    callback: Box<dyn FnMut(&str, &Peer) + Send + Sync + 'static>,
    tau: Duration,
    phi: f32,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Default)]
pub struct Peer {
    port: u16,
    addrs: Vec<IpAddr>,
}

impl Peer {
    pub fn new(port: u16) -> Self {
        Self {
            port,
            addrs: vec![],
        }
    }
}

impl Discoverer {
    pub fn new(name: String, peer_id: String) -> Self {
        Self {
            name,
            peer_id,
            peers: BTreeMap::new(),
            callback: Box::new(|_, _| {}),
            tau: Duration::from_secs(10),
            phi: 1.0,
        }
    }

    pub fn with_addrs(mut self, port: u16, mut addrs: Vec<IpAddr>) -> Self {
        addrs.sort();
        self.peers
            .insert(self.peer_id.clone(), Peer { port, addrs });
        self
    }

    pub fn with_callback(
        mut self,
        callback: impl FnMut(&str, &Peer) + Send + Sync + 'static,
    ) -> Self {
        self.callback = Box::new(callback);
        self
    }

    pub fn with_cadence(mut self, tau: Duration) -> Self {
        self.tau = tau;
        self
    }

    pub fn with_response_rate(mut self, phi: f32) -> Self {
        self.phi = phi;
        self
    }

    pub fn spawn(self, handle: &Handle) -> anyhow::Result<DropGuard> {
        let mut discoverer = self;
        let tau = discoverer.tau;
        let phi = discoverer.phi;

        let _entered = handle.enter();
        let socket = socket::socket_v4().context("socket_v4")?;

        let service_name = Name::from_str(&format!("_{}._udp.local.", discoverer.name))
            .context("constructing service name")?;
        let query = make_query(&service_name)?;
        let response = make_response(&discoverer, &service_name)?;

        let task = handle.spawn(async move {
            let service_name = service_name;
            let mut buf = [0; 1472];
            loop {
                // phase 1: waiting to query
                // phase 2: waiting to respond
                // always dealing with responses

                // phase 1
                let timeout = tokio::time::sleep(
                    tau + tau / 1_000_000 * thread_rng().gen_range(0..1_000_000),
                );
                pin!(timeout);
                loop {
                    tokio::select! {
                        _ = &mut timeout => {
                            send_msg(&query, &socket).await;
                            break;
                        }
                        res = socket.recv_from(&mut buf) => {
                            match res {
                                Ok((len, addr)) => {
                                    tracing::trace!("received {} bytes from {}", len, addr);
                                    let p1 = handle_recv_phase_1(&mut discoverer, &buf[..len], &service_name);
                                    if p1 == Phase1::Query {
                                        break;
                                    }
                                }
                                Err(e) => {
                                    tracing::warn!("error receiving from mDNS socket: {}", e);
                                }
                            }
                        }
                    }
                }

                // phase 2
                let timeout = tokio::time::sleep(Duration::from_millis(100) / 1_000_000 * thread_rng().gen_range(0..1_000_000));
                pin!(timeout);
                let mut counter = 0;
                loop {
                    tokio::select! {
                        _ = &mut timeout => {
                            if let Some(response) = response.as_ref() {
                                send_msg(response, &socket).await;
                            }
                            break;
                        }
                        res = socket.recv_from(&mut buf) => {
                            match res {
                                Ok((len, addr)) => {
                                    tracing::trace!("received {} bytes from {}", len, addr);
                                    counter += handle_recv_phase_2(&mut discoverer, &buf[..len], &service_name);
                                    if counter > (tau.as_secs_f32() * phi) as usize {
                                        break;
                                    }
                                }
                                Err(e) => {
                                    tracing::warn!("error receiving from mDNS socket: {}", e);
                                }
                            }
                        }
                    }
                }
            }
        });
        Ok(DropGuard { task })
    }
}

fn make_query(service_name: &Name) -> anyhow::Result<Message> {
    let mut msg = Message::new();
    msg.set_message_type(MessageType::Query);
    let mut query = Query::new();
    query.set_query_class(DNSClass::IN);
    query.set_query_type(RecordType::PTR);
    query.set_name(service_name.clone());
    msg.add_query(query);
    Ok(msg)
}

fn make_response(discoverer: &Discoverer, service_name: &Name) -> anyhow::Result<Option<Message>> {
    if let Some(peer) = discoverer.peers.get(&discoverer.peer_id) {
        let mut msg = Message::new();
        msg.set_message_type(MessageType::Response);
        msg.set_authoritative(true);

        let target = Name::from_str(&format!("{}.local.", discoverer.peer_id))
            .context("constructing name from PeerId")?;
        let my_srv_name = Name::from_str(&discoverer.peer_id)
            .context("DNS name from peer_id")?
            .append_domain(service_name)
            .context("appending domain to peer_id")?;
        msg.add_answer(Record::from_rdata(
            my_srv_name,
            0,
            RData::SRV(rdata::SRV::new(0, 0, peer.port, target.clone())),
        ));
        for addr in &peer.addrs {
            match addr {
                IpAddr::V4(addr) => {
                    msg.add_additional(Record::from_rdata(
                        target.clone(),
                        0,
                        RData::A(rdata::A::from(*addr)),
                    ));
                }
                IpAddr::V6(addr) => {
                    msg.add_additional(Record::from_rdata(
                        target.clone(),
                        0,
                        RData::AAAA(rdata::AAAA::from(*addr)),
                    ));
                }
            }
        }
        Ok(Some(msg))
    } else {
        tracing::info!("no addresses for peer, not announcing");
        Ok(None)
    }
}

#[must_use = "dropping this value will stop the mDNS discovery"]
pub struct DropGuard {
    task: JoinHandle<()>,
}

impl Drop for DropGuard {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn send_msg(msg: &Message, socket: &UdpSocket) {
    let bytes = match msg.to_vec() {
        Ok(b) => b,
        Err(e) => {
            tracing::warn!("error serializing mDNS: {}", e);
            return;
        }
    };
    if let Err(e) = socket.send_to(&bytes, (MDNS_IPV4, MDNS_PORT)).await {
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

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Clone, Copy)]
enum Phase1 {
    Error,
    Query,
    Response,
}

fn handle_recv_phase_1(peers: &mut Discoverer, buf: &[u8], service_name: &Name) -> Phase1 {
    let packet = match Message::from_vec(buf) {
        Ok(p) => p,
        Err(e) => {
            tracing::debug!("error parsing mDNS packet: {}", e);
            return Phase1::Error;
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
        return Phase1::Query;
    }
    handle_response(packet, peers, service_name);
    Phase1::Response
}

fn handle_response(packet: Message, peers: &mut Discoverer, service_name: &Name) -> usize {
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
    let ret = peer_ids.len();
    for (peer_id, (port, mut addrs)) in peer_ids {
        addrs.sort();
        let current = peers
            .peers
            .get(&peer_id)
            .map(Cow::Borrowed)
            .unwrap_or_else(|| Cow::Owned(Peer::new(port)));
        let peer = Peer { port, addrs };
        if current.as_ref() != &peer {
            tracing::debug!("peer {} changed from {:?} to {:?}", peer_id, current, peer);
            (peers.callback)(&peer_id, &peer);
            peers.peers.insert(peer_id.clone(), peer);
        }
    }
    ret
}

fn handle_recv_phase_2(peers: &mut Discoverer, buf: &[u8], service_name: &Name) -> usize {
    let packet = match Message::from_vec(buf) {
        Ok(p) => p,
        Err(e) => {
            tracing::debug!("error parsing mDNS packet: {}", e);
            return 0;
        }
    };
    handle_response(packet, peers, service_name)
}
