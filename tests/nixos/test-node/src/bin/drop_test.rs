use std::ffi::CString;
use std::fs;
use std::net::{IpAddr, Ipv4Addr};
use std::time::{Duration, Instant};
use swarm_discovery::{Discoverer, IpClass};

/// Convert an interface name to its OS interface index.
fn if_nametoindex(name: &str) -> u32 {
    let c_name = CString::new(name).expect("invalid interface name");
    let idx = unsafe { libc::if_nametoindex(c_name.as_ptr()) };
    if idx == 0 {
        panic!(
            "interface '{}' not found: {}",
            name,
            std::io::Error::last_os_error()
        );
    }
    idx
}

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3 {
        eprintln!("Usage: {} <interface-name> <ip-address>", args[0]);
        std::process::exit(1);
    }

    let iface = &args[1];
    let addr: Ipv4Addr = args[2].parse().expect("valid IPv4 address");
    let ifindex = if_nametoindex(iface);

    eprintln!(
        "=== Drop cleanup test on interface {} (index {}), addr {} ===",
        iface, ifindex, addr
    );

    // Build a multi-threaded tokio runtime
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("build tokio runtime");

    // Spawn the discoverer — this creates the guardian actor, sender, receiver,
    // and updater actors, plus mDNS sockets bound to port 5353.
    let guard = Discoverer::new("droptest".to_owned(), "testpeer".to_owned())
        .with_addrs(5000, vec![IpAddr::V4(addr)])
        .with_ip_class(IpClass::V4Only)
        .with_multicast_interfaces_v4(vec![ifindex])
        .with_cadence(Duration::from_millis(500))
        .with_response_rate(5.0)
        .with_callback(|peer_id, peer| {
            eprintln!("  callback: {} {:?}", peer_id, peer.addrs());
        })
        .spawn(rt.handle())
        .expect("spawn discoverer");

    // Let discovery run long enough for all actors to start and send packets
    std::thread::sleep(Duration::from_secs(3));
    fs::write("/tmp/drop-test-running", "").unwrap();
    eprintln!("Discovery is running, signaled /tmp/drop-test-running");

    // Drop the guard — this should abort the guardian task and drop the acto
    // runtime, which in turn cancels all child actors and closes all sockets.
    eprintln!("Dropping DropGuard...");
    drop(guard);

    // Give async cleanup a moment to propagate
    std::thread::sleep(Duration::from_secs(2));
    fs::write("/tmp/drop-test-dropped", "").unwrap();
    eprintln!("Guard dropped, signaled /tmp/drop-test-dropped");

    // Shut down the tokio runtime.  If the DropGuard properly cleaned up all
    // spawned tasks, this should complete almost instantly.  If tasks leaked,
    // the runtime will have to forcefully cancel them and may take up to the
    // full timeout.
    eprintln!("Shutting down tokio runtime...");
    let start = Instant::now();
    rt.shutdown_timeout(Duration::from_secs(10));
    let elapsed = start.elapsed();
    eprintln!("Runtime shutdown took {:?}", elapsed);

    if elapsed > Duration::from_secs(5) {
        eprintln!(
            "FAIL: Runtime shutdown took too long ({:?}), tasks were not properly cleaned up",
            elapsed
        );
        fs::write("/tmp/drop-test-done", "FAIL").unwrap();
        std::process::exit(1);
    }

    eprintln!(
        "PASS: All threads cleaned up, runtime shut down in {:?}",
        elapsed
    );
    fs::write("/tmp/drop-test-done", "PASS").unwrap();
}
