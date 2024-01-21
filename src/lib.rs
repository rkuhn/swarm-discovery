#![doc = include_str!("../README.md")]

mod guardian;
mod receiver;
mod sender;
mod socket;
mod updater;

use acto::{AcTokio, ActoHandle, ActoRuntime, TokioJoinHandle};
use anyhow::Context;
use hickory_proto::rr::Name;
use serde::{Deserialize, Serialize};
use socket::Sockets;
use std::{
    collections::BTreeMap,
    fmt::Display,
    net::IpAddr,
    str::FromStr,
    time::{Duration, Instant},
};
use tokio::runtime::Handle;

type Callback = Box<dyn FnMut(&str, &Peer) + Send + 'static>;

/// Builder for a swarm discovery service.
///
/// # Example
///
/// ```rust
/// use if_addrs::get_if_addrs;
/// use swarm_discovery::Discoverer;
/// use tokio::runtime::Builder;
///
/// // create Tokio runtime
/// let rt = Builder::new_multi_thread()
///     .enable_all()
///     .build()
///     .expect("build runtime");
///
/// // make up some peer ID
/// let peer_id = "peer_id42".to_owned();
///
/// // get local addresses and make up some port
/// let addrs = get_if_addrs().unwrap().into_iter().map(|i| i.addr.ip()).collect::<Vec<_>>();
/// let port = 1234;
///
/// // start announcing and discovering
/// let _guard = Discoverer::new("swarm".to_owned(), peer_id)
///     .with_addrs(port, addrs)
///     .with_callback(|peer_id, peer| {
///         println!("discovered {}: {:?}", peer_id, peer);
///     })
///     .spawn(rt.handle())
///     .expect("discoverer spawn");
/// ```
pub struct Discoverer {
    name: String,
    protocol: Protocol,
    peer_id: String,
    peers: BTreeMap<String, Peer>,
    callback: Callback,
    tau: Duration,
    phi: f32,
    class: IpClass,
}

/// A peer discovered by the swarm discovery service.
///
/// The discovery yields service instances, which are located by a port and a list of IP addresses.
/// Both IPv4 and IPv6 addresses may be present, depending on the configuration via [Discoverer::with_ip_class].
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Peer {
    pub port: u16,
    pub addrs: Vec<IpAddr>,
    pub last_seen: Instant,
}

/// This selects which sockets will be created by the [Discoverer].
///
/// Responses will be sent on that socket which received the query.
/// Queries will prefer v4 when available.
/// Default is [IpClass::V4AndV6].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum IpClass {
    V4Only,
    V6Only,
    #[default]
    V4AndV6,
}

impl IpClass {
    /// Returns `true` if IPv4 is enabled.
    pub fn has_v4(self) -> bool {
        matches!(self, Self::V4Only | Self::V4AndV6)
    }

    /// Returns `true` if IPv6 is enabled.
    pub fn has_v6(self) -> bool {
        matches!(self, Self::V6Only | Self::V4AndV6)
    }
}

/// This selects which protocol suffix to use for the service name.
///
/// Default is [Protocol::Udp].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Protocol {
    #[default]
    Udp,
    Tcp,
}

impl Display for Protocol {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Protocol::Udp => write!(f, "_udp"),
            Protocol::Tcp => write!(f, "_tcp"),
        }
    }
}

impl Discoverer {
    /// Creates a new builder for a swarm discovery service.
    ///
    /// The `name` is the name of the mDNS service, meaning that it will be discoverable under the name `_name._udp.local.`.
    /// The `peer_id` is the unique identifier of this peer, which will be discoverable under the name `peer_id._name._udp.local.`.
    pub fn new(name: String, peer_id: String) -> Self {
        Self {
            name,
            protocol: Protocol::default(),
            peer_id,
            peers: BTreeMap::new(),
            callback: Box::new(|_, _| {}),
            tau: Duration::from_secs(10),
            phi: 1.0,
            class: IpClass::default(),
        }
    }

    /// Set the protocol suffix to use for the service name.
    ///
    /// Note that this does not change the protocol used for discovery, which is always UDP-based mDNS.
    /// Default is [Protocol::Udp].
    pub fn with_protocol(mut self, protocol: Protocol) -> Self {
        self.protocol = protocol;
        self
    }

    /// Register the local peer’s port and IP addresses.
    ///
    /// If this method is not called, the local peer will not advertise itself.
    /// It can still discover others.
    pub fn with_addrs(mut self, port: u16, mut addrs: Vec<IpAddr>) -> Self {
        addrs.sort();
        self.peers.insert(
            self.peer_id.clone(),
            Peer {
                port,
                addrs,
                last_seen: Instant::now(),
            },
        );
        self
    }

    /// Register a callback to be called when a peer is discovered or its addresses change.
    pub fn with_callback(mut self, callback: impl FnMut(&str, &Peer) + Send + 'static) -> Self {
        self.callback = Box::new(callback);
        self
    }

    /// Set the discovery time target.
    ///
    /// After roughly this time a new peer should have discovered some parts of the swarm.
    pub fn with_cadence(mut self, tau: Duration) -> Self {
        self.tau = tau;
        self
    }

    /// Set the response frequency target.
    ///
    /// While query-response cycles follow the configured cadence (see [Discoverer::with_cadence]),
    /// the response rate determines the (soft) maximum of how many responses should be received per second.
    ///
    /// With cadence 10sec, setting this to 1.0Hz means that at most 10 responses will be received per cycle.
    /// Setting it to 0.5Hz means that up to roughly 5 responses will be received per cycle.
    pub fn with_response_rate(mut self, phi: f32) -> Self {
        self.phi = phi;
        self
    }

    /// Set which IP classes to use.
    ///
    /// The default is to use both IPv4 and IPv6, where IPv4 is preferred for sending queries.
    /// Responses will be sent using that class which the query used.
    pub fn with_ip_class(mut self, class: IpClass) -> Self {
        self.class = class;
        self
    }

    /// Start the discovery service.
    ///
    /// This will spawn asynchronous tasks and return a guard which will stop the discovery when dropped.
    /// Changing the configuration is done by stopping the discovery and starting a new one.
    pub fn spawn(self, handle: &Handle) -> anyhow::Result<DropGuard> {
        let _entered = handle.enter();
        let sockets = Sockets::new(self.class)?;

        let service_name = Name::from_str(&format!("_{}.{}.local.", self.name, self.protocol))
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

/// A guard which will keep the discovery running until it is dropped.
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
