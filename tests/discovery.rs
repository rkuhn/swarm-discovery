fn main() {
    #[cfg(target_os = "linux")]
    test();
}

#[cfg(target_os = "linux")]
fn test() {
    use if_addrs::get_if_addrs;
    use ipc_channel::ipc::{channel, IpcReceiver, IpcSender};
    use netsim_embed::{declare_machines, machine, run_tests, DelayBuffer, Ipv4Range, Netsim};
    use std::{
        collections::{BTreeMap, BTreeSet},
        thread,
        time::{Duration, Instant},
    };
    use swarm_discovery::{Discoverer, Protocol};
    use tokio::runtime::Builder;
    use tracing_subscriber::{fmt, EnvFilter};

    #[machine]
    fn disco(
        (protocol, tau, phi, peer_id, port, rcv, snd): (
            Protocol,
            Duration,
            f32,
            String,
            u16,
            IpcReceiver<()>,
            IpcSender<(String, String)>,
        ),
    ) {
        let rt = Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("build runtime");

        let addrs = get_if_addrs()
            .expect("get_if_addrs")
            .into_iter()
            .map(|iface| iface.addr.ip())
            .collect::<Vec<_>>();

        let _guard = Discoverer::new("swarm".to_owned(), peer_id.clone())
            .with_protocol(protocol)
            .with_addrs(port, addrs)
            .with_callback(move |pid, _addrs| {
                snd.send((peer_id.clone(), pid.to_owned())).expect("send");
            })
            .with_cadence(tau)
            .with_response_rate(phi)
            .spawn(rt.handle())
            .expect("discoverer spawn");

        rcv.recv().expect("recv");
    }

    fn discover(protocol: Protocol) {
        let rt = Builder::new_current_thread()
            .build()
            .expect("build runtime");

        let mut sim = Netsim::<String, String>::new();
        let net = sim.spawn_network(Ipv4Range::random_local_subnet());
        sim.network(net)
            .set_count_filter(Some(Box::new(|bytes: &[u8]| {
                bytes.len() >= 60 && bytes[16..24] == [224, 0, 0, 251, 20, 233, 20, 233]
            })));

        // one channel for the discoveries
        let (tx_f, rx_f) = channel().expect("channel");

        const N: usize = 100;
        let tau = Duration::from_millis(2000);
        let phi = 5f32;

        let spawn = move |sim: &mut Netsim<String, String>, num: usize| {
            let (tx_d, rx_d) = channel().expect("channel");
            let tx_f = tx_f.clone();

            rt.block_on(async move {
                let mut delay = DelayBuffer::new();
                delay.set_delay(Duration::from_millis(10));
                let id = sim
                    .spawn(
                        disco,
                        (
                            protocol,
                            tau,
                            phi,
                            format!("peer_id{}", num),
                            1234 + num as u16,
                            rx_d,
                            tx_f,
                        ),
                        Some(delay),
                    )
                    .await;
                sim.plug(id, net, None).await;
            });

            tx_d
        };

        let mut discovered = BTreeMap::<String, BTreeSet<String>>::new();
        let mut channels = Vec::with_capacity(N);
        for i in 0..N {
            let tx_d = spawn(&mut sim, i);
            discovered.insert(format!("peer_id{}", i), BTreeSet::new());
            channels.push(tx_d);
            thread::sleep(Duration::from_millis(10));
        }
        tracing::info!("waiting for discovery");

        let start = Instant::now();
        let mut discoveries = 0;
        loop {
            let (host, peer) = rx_f.try_recv_timeout(Duration::from_secs(5)).unwrap();
            discoveries += 1;
            tracing::info!(
                "{} discovered {} ({} remaining)",
                host,
                peer,
                discovered.len()
            );
            if discovered.len() < 5 {
                tracing::info!("discovered: {:?}", discovered.keys().collect::<Vec<_>>());
            }
            if let Some(set) = discovered.get_mut(&host) {
                set.insert(peer.to_owned());
                if set.len() == N {
                    discovered.remove(&host);
                }
                if discovered.is_empty() {
                    break;
                }
            }
        }
        let elapsed = start.elapsed();

        let forwarded = sim.network(net).num_forwarded();
        tracing::info!(%forwarded, ?elapsed, %discoveries, "discovery complete");

        // phase 1 was getting a swarm of N nodes going - during mass startup
        // there are lots of duplicates, so we expect a lot of forwarded packets
        // the real test is now adding a node and observing its discoveries while
        // seeing basically no duplicates

        let tx_d = spawn(&mut sim, N);
        let peer_id = format!("peer_id{}", N);
        let mut discovered = BTreeSet::new();
        let start = Instant::now();
        discoveries = 0;
        while discovered.len() < (N + 1) {
            discoveries += 1;
            let (host, peer) = rx_f.recv().expect("recv");
            if host != peer_id {
                continue;
            }
            tracing::info!("{} discovered {}", host, peer);
            discovered.insert(peer);
        }
        let elapsed = start.elapsed();

        for tx_d in channels {
            tx_d.send(()).expect("send");
        }
        tx_d.send(()).expect("send");

        let net = sim.network(net);
        let max =
            // number of responses
            (elapsed.as_secs_f32() * phi).ceil() as usize * (N + 1)
            // number of queries
            + (elapsed.as_secs_f32() / tau.as_secs_f32()).ceil() as usize * (N + 1);
        let forwarded2 = net.num_forwarded() - forwarded;
        tracing::info!(%forwarded2, ?elapsed, %discoveries, "additional discovery complete");
        assert!(forwarded2 < max, "forwarded {} >= {}", forwarded2, max);
        assert_eq!(net.num_disabled(), 0);
        assert_eq!(net.num_invalid(), 0);
        assert_eq!(net.num_unroutable(), 0);
    }

    fn discover_udp() {
        discover(Protocol::Udp);
    }

    fn discover_tcp() {
        discover(Protocol::Tcp);
    }

    fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .with_test_writer()
        .init();

    declare_machines!(disco);
    run_tests!(discover_udp, discover_tcp);
}
