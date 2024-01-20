fn main() {
    #[cfg(target_os = "linux")]
    test();
}

#[cfg(target_os = "linux")]
fn test() {
    netsim_embed::NetSim::new();
}
