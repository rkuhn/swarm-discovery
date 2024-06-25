use if_addrs::get_if_addrs;
use rand::{thread_rng, Rng};
use std::{
    io::{stderr, stdin},
    net::UdpSocket,
};
use swarm_discovery::Discoverer;
use tokio::runtime::Builder;
use tracing_subscriber::{fmt, EnvFilter};

fn main() {
    // enable logging: use `RUST_LOG=debug` or similar to see logs on STDERR
    fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .with_writer(stderr)
        .init();

    // create Tokio runtime
    let rt = Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("build runtime");

    // make up some peer ID
    let peer_id = format!("peer_id{}", thread_rng().gen_range(0..100));

    // get local addresses and make up some port
    let addrs = get_if_addrs()
        .expect("get_if_addrs")
        .into_iter()
        .map(|iface| iface.addr.ip())
        .collect::<Vec<_>>();
    let port = UdpSocket::bind((addrs[0], 0))
        .expect("bind")
        .local_addr()
        .expect("local_addr")
        .port();

    println!("peer_id: {}", peer_id);
    println!("addrs: {:?}", addrs);

    // start announcing and discovering
    let _guard = Discoverer::new("swarm".to_owned(), peer_id)
        .with_addrs(port, addrs.iter().take(1).copied())
        .with_addrs(port + 1, addrs)
        .with_callback(|peer_id, addrs| {
            println!("discovered {}: {:?}", peer_id, addrs);
        })
        .spawn(rt.handle())
        .expect("discoverer spawn");

    // end program when user presses Enter
    stdin().read_line(&mut String::new()).expect("read_line");
}
