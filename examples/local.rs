use rand::{thread_rng, Rng};
use std::{
    env::args,
    io::{stderr, stdin},
    net::{IpAddr, UdpSocket},
    str::FromStr,
};
use swarm_discovery::Discoverer;
use tokio::runtime::Builder;
use tracing_subscriber::{fmt, EnvFilter};

fn main() {
    fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .with_writer(stderr)
        .init();

    let rt = Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("build runtime");

    let peer_id = format!("peer_id{}", thread_rng().gen_range(0..100));
    let ifaddr = args().nth(1).expect("ifaddr");
    let ipaddr = IpAddr::from_str(&ifaddr).expect("parse ifaddr");
    let port = UdpSocket::bind((ipaddr, 0))
        .expect("bind")
        .local_addr()
        .expect("local_addr")
        .port();

    let _guard = Discoverer::new("swarm".to_owned(), peer_id)
        .with_addrs(port, vec![ipaddr])
        .with_callback(|peer_id, addrs| {
            eprintln!("discovered {}: {:?}", peer_id, addrs);
        })
        .spawn(rt.handle())
        .expect("discoverer spawn");

    stdin().read_line(&mut String::new()).expect("read_line");
}
