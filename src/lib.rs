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

mod guardian;
mod receiver;
mod sender;
mod socket;
mod updater;

use acto::{AcTokio, ActoHandle, ActoRuntime, TokioJoinHandle};
use anyhow::Context;
use hickory_proto::rr::Name;
use socket::Sockets;
use std::{collections::BTreeMap, net::IpAddr, str::FromStr, time::Duration};
use tokio::runtime::Handle;

pub struct Discoverer {
    name: String,
    peer_id: String,
    peers: BTreeMap<String, Peer>,
    callback: Box<dyn FnMut(&str, &Peer) + Send + Sync + 'static>,
    tau: Duration,
    phi: f32,
    class: IpClass,
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

/// This selects which sockets will be created by the [Discoverer].
///
/// Responses will be sent on that socket which received the query.
/// Queries will prefer v4 when available.
/// Default is [IpClass::V4AndV6].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum IpClass {
    V4Only,
    V6Only,
    #[default]
    V4AndV6,
}

impl IpClass {
    pub fn has_v4(self) -> bool {
        matches!(self, Self::V4Only | Self::V4AndV6)
    }

    pub fn has_v6(self) -> bool {
        matches!(self, Self::V6Only | Self::V4AndV6)
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
            class: IpClass::default(),
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

    pub fn with_ip_class(mut self, class: IpClass) -> Self {
        self.class = class;
        self
    }

    pub fn spawn(self, handle: &Handle) -> anyhow::Result<DropGuard> {
        let _entered = handle.enter();
        let sockets = Sockets::new(self.class)?;

        let service_name = Name::from_str(&format!("_{}._udp.local.", self.name))
            .context("constructing service name")?;
        // need to test this here so it won’t fail in the actor
        Name::from_str(&self.peer_id)
            .context("constructing name from peer ID")?
            .append_domain(&service_name)
            .context("appending service name to peer ID")?;

        let rt = AcTokio::from_handle("swarm-discovery", handle.clone());
        let task = rt
            .spawn_actor("guardian", move |ctx| {
                guardian::guardian(ctx, self, sockets, service_name)
            })
            .handle;

        Ok(DropGuard {
            task: Some(task),
            _rt: rt,
        })
    }
}

#[must_use = "dropping this value will stop the mDNS discovery"]
pub struct DropGuard {
    task: Option<TokioJoinHandle<()>>,
    _rt: AcTokio,
}

impl Drop for DropGuard {
    fn drop(&mut self) {
        self.task.take().unwrap().abort();
    }
}
