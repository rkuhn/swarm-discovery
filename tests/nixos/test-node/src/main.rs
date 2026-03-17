use std::env;
use std::fs;
use std::io::Write;
use std::net::{IpAddr, Ipv4Addr};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use swarm_discovery::{Discoverer, IpClass};
use tokio::time::sleep;

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let args: Vec<String> = env::args().collect();
    if args.len() < 4 {
        eprintln!("Usage: {} <peer-id> <port> <interface-ip> [<interface-ip>...]", args[0]);
        std::process::exit(1);
    }

    let peer_id = args[1].clone();
    let port: u16 = args[2].parse().expect("valid port");
    let interfaces: Vec<Ipv4Addr> = args[3..]
        .iter()
        .map(|s| s.parse().expect("valid IPv4 address"))
        .collect();
    let addrs: Vec<IpAddr> = interfaces.iter().map(|ip| IpAddr::V4(*ip)).collect();

    let events_path = PathBuf::from(format!("/tmp/discovery-events-{}.jsonl", peer_id));
    let cmd_path = PathBuf::from(format!("/tmp/discovery-cmd-{}", peer_id));
    let ready_path = PathBuf::from(format!("/tmp/discovery-ready-{}", peer_id));

    // Truncate/create events file
    fs::write(&events_path, "").unwrap();

    let events_file = Arc::new(Mutex::new(
        fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&events_path)
            .unwrap(),
    ));

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("build tokio runtime");

    let events = events_file.clone();
    let guard = Discoverer::new("nixtest".to_owned(), peer_id.clone())
        .with_addrs(port, addrs)
        .with_ip_class(IpClass::V4Only)
        .with_multicast_interfaces_v4(interfaces)
        .with_cadence(Duration::from_millis(500))
        .with_response_rate(5.0)
        .with_callback(move |pid, peer| {
            let line = if peer.is_expiry() {
                format!(r#"{{"event":"lost","peer_id":"{}"}}"#, pid)
            } else {
                let addrs_json: Vec<String> = peer
                    .addrs()
                    .iter()
                    .map(|(ip, p)| format!(r#"["{}",{}]"#, ip, p))
                    .collect();
                format!(
                    r#"{{"event":"discovered","peer_id":"{}","addrs":[{}]}}"#,
                    pid,
                    addrs_json.join(",")
                )
            };
            let mut f = events.lock().unwrap();
            writeln!(f, "{}", line).unwrap();
            f.flush().unwrap();
        })
        .spawn(rt.handle())
        .expect("spawn discoverer");

    // Signal readiness
    fs::write(&ready_path, "ready").unwrap();
    eprintln!("Test node {} ready", peer_id);

    // Command polling loop
    rt.block_on(async {
        loop {
            sleep(Duration::from_millis(100)).await;

            let contents = match fs::read_to_string(&cmd_path) {
                Ok(c) => c,
                Err(_) => continue,
            };
            let contents = contents.trim();
            if contents.is_empty() {
                continue;
            }

            let _ = fs::remove_file(&cmd_path);

            let parts: Vec<&str> = contents.split_whitespace().collect();
            match parts.first().copied() {
                Some("add_interface") => {
                    let addr: Ipv4Addr = parts[1].parse().unwrap();
                    guard.add_interface_v4(addr);
                    eprintln!("Added interface {}", addr);
                }
                Some("remove_interface") => {
                    let addr: Ipv4Addr = parts[1].parse().unwrap();
                    guard.remove_interface_v4(addr);
                    eprintln!("Removed interface {}", addr);
                }
                Some("add_addr") => {
                    let p: u16 = parts[1].parse().unwrap();
                    let addr: IpAddr = parts[2].parse().unwrap();
                    guard.add(p, vec![addr]);
                    eprintln!("Added addr {}:{}", addr, p);
                }
                Some("remove_addr") => {
                    let addr: IpAddr = parts[1].parse().unwrap();
                    guard.remove_addr(addr);
                    eprintln!("Removed addr {}", addr);
                }
                Some("shutdown") => {
                    eprintln!("Shutting down");
                    break;
                }
                other => {
                    eprintln!("Unknown command: {:?}", other);
                }
            }
        }
    });

    drop(guard);
}
