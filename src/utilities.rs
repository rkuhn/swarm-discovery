use std::ffi::CString;
use std::io;

/// Convert a network interface name (e.g. `"eth0"`, `"mesh-vlan"`) to its OS
/// interface index, suitable for passing to
/// [`Discoverer::with_multicast_interfaces_v4`](crate::Discoverer::with_multicast_interfaces_v4),
/// [`DropGuard::add_interface_v4`](crate::DropGuard::add_interface_v4), etc.
///
/// Returns an error if the interface does not exist or the name contains a nul byte.
///
/// # Example
///
/// ```rust,no_run
/// use swarm_discovery::utilities::if_nametoindex;
///
/// let idx = if_nametoindex("eth0").expect("interface not found");
/// println!("eth0 has index {}", idx);
/// ```
pub fn if_nametoindex(name: &str) -> io::Result<u32> {
    let c_name = CString::new(name).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "interface name contains nul byte",
        )
    })?;
    let idx = unsafe { libc::if_nametoindex(c_name.as_ptr()) };
    if idx == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(idx)
    }
}
