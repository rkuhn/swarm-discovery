fn main() {
    #[cfg(target_os = "linux")]
    test();
}

#[cfg(target_os = "linux")]
fn test() {
    use if_addrs::get_if_addrs;
    use ipc_channel::ipc::{channel, IpcReceiver, IpcSender};
    use netsim_embed::{declare_machines, machine, run_tests, unshare_user, Ipv4Range, Netsim};
    use std::{
        collections::{BTreeMap, BTreeSet},
        time::Duration,
    };
    use swarm_discovery::Discoverer;
    use tokio::runtime::Builder;
    use tracing_subscriber::{fmt, EnvFilter};

    #[machine]
    fn disco(
        (peer_id, port, rcv, snd): (String, u16, IpcReceiver<()>, IpcSender<(String, String)>),
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
            .with_addrs(port, addrs)
            .with_callback(move |pid, _addrs| {
                snd.send((peer_id.clone(), pid.to_owned())).expect("send");
            })
            .with_cadence(Duration::from_secs(1))
            .spawn(rt.handle())
            .expect("discoverer spawn");

        rcv.recv().expect("recv");
    }

    fn discover() {
        let rt = Builder::new_current_thread()
            .build()
            .expect("build runtime");

        let mut sim = Netsim::<String, String>::new();
        let net = sim.spawn_network(Ipv4Range::random_local_subnet());

        // one channel for the discoveries
        let (tx_f, rx_f) = channel().expect("channel");

        let spawn = move |sim: &mut Netsim<String, String>, num: u8| {
            let (tx_d, rx_d) = channel().expect("channel");
            let tx_f = tx_f.clone();

            rt.block_on(async move {
                let id = sim
                    .spawn(
                        disco,
                        (format!("peer_id{}", num), 1234 + num as u16, rx_d, tx_f),
                    )
                    .await;
                sim.plug(id, net, None).await;
            });

            tx_d
        };

        let tx_d1 = spawn(&mut sim, 1);
        let tx_d2 = spawn(&mut sim, 2);
        let tx_d3 = spawn(&mut sim, 3);

        let mut discovered = BTreeMap::<String, BTreeSet<String>>::new();
        [1, 2, 3].iter().for_each(|n| {
            discovered.insert(format!("peer_id{}", n), BTreeSet::new());
        });
        loop {
            let (host, peer) = rx_f.try_recv_timeout(Duration::from_secs(5)).unwrap();
            tracing::info!("{} discovered {}", host, peer);
            let set = discovered.get_mut(&host).expect("get_mut");
            set.insert(peer.to_owned());
            if set.len() == 3 {
                discovered.remove(&host);
            }
            if discovered.is_empty() {
                break;
            }
        }

        tx_d1.send(()).expect("send");
        tx_d2.send(()).expect("send");
        tx_d3.send(()).expect("send");
    }

    fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .with_test_writer()
        .init();

    unshare_user().expect("unshare_user");

    declare_machines!(disco);
    run_tests!(discover);
}
