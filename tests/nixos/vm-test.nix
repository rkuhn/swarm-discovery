{ pkgs, testNode }:

pkgs.testers.nixosTest {
  name = "swarm-discovery-multicast";

  nodes = {
    node1 = { ... }: {
      virtualisation.vlans = [ 1 2 ];

      networking.useDHCP = false;
      networking.interfaces.eth1.ipv4.addresses = [{
        address = "192.168.1.1";
        prefixLength = 24;
      }];
      networking.interfaces.eth2.ipv4.addresses = [{
        address = "192.168.2.1";
        prefixLength = 24;
      }];

      networking.firewall.enable = false;

      environment.systemPackages = [ testNode ];
    };

    node2 = { ... }: {
      virtualisation.vlans = [ 1 2 ];

      networking.useDHCP = false;
      networking.interfaces.eth1.ipv4.addresses = [{
        address = "192.168.1.2";
        prefixLength = 24;
      }];
      networking.interfaces.eth2.ipv4.addresses = [{
        address = "192.168.2.2";
        prefixLength = 24;
      }];

      networking.firewall.enable = false;

      environment.systemPackages = [ testNode ];
    };
  };

  testScript = ''
    import json
    import time

    start_all()

    # Wait for network interfaces to be configured
    node1.wait_until_succeeds("ip addr show eth1 | grep -q '192.168.1.1'")
    node1.wait_until_succeeds("ip addr show eth2 | grep -q '192.168.2.1'")
    node2.wait_until_succeeds("ip addr show eth1 | grep -q '192.168.1.2'")
    node2.wait_until_succeeds("ip addr show eth2 | grep -q '192.168.2.2'")

    # Verify basic connectivity on both VLANs
    node1.succeed("ping -c 1 192.168.1.2")
    node2.succeed("ping -c 1 192.168.1.1")
    node1.succeed("ping -c 1 192.168.2.2")
    node2.succeed("ping -c 1 192.168.2.1")


    def send_command(machine, peer_id, command):
        """Send a command to a test node via its command file."""
        cmd_file = f"/tmp/discovery-cmd-{peer_id}"
        machine.succeed(f"echo '{command}' > {cmd_file}")
        time.sleep(0.5)


    def dump_events(machine, peer_id, label):
        """Dump the events file for debugging."""
        events_file = f"/tmp/discovery-events-{peer_id}.jsonl"
        content = machine.succeed(f"cat {events_file}").strip()
        print(f"=== {label}: {peer_id} events ===")
        for line in content.splitlines():
            print(f"  {line}")
        if not content:
            print("  (empty)")
        print(f"=== end {peer_id} ===")


    def get_line_count(machine, peer_id):
        """Get the current line count of the events file."""
        events_file = f"/tmp/discovery-events-{peer_id}.jsonl"
        return int(machine.succeed(f"wc -l < {events_file}").strip())


    # ============================================================
    # Phase 1: Basic discovery on VLAN 1
    # ============================================================
    with subtest("Basic discovery on VLAN 1"):
        node1.succeed(
            "RUST_LOG=debug swarm-discovery-test-node node1 5000 192.168.1.1 &> /tmp/test-node1.log &"
        )
        node2.succeed(
            "RUST_LOG=debug swarm-discovery-test-node node2 5000 192.168.1.2 &> /tmp/test-node2.log &"
        )

        node1.wait_for_file("/tmp/discovery-ready-node1")
        node2.wait_for_file("/tmp/discovery-ready-node2")

        # Wait for mutual discovery
        node1.wait_until_succeeds(
            "grep '\"event\":\"discovered\"' /tmp/discovery-events-node1.jsonl | grep -q '\"peer_id\":\"node2\"'",
            timeout=30,
        )
        node2.wait_until_succeeds(
            "grep '\"event\":\"discovered\"' /tmp/discovery-events-node2.jsonl | grep -q '\"peer_id\":\"node1\"'",
            timeout=30,
        )

        # Verify discovered addresses are correct (VLAN 1 IPs)
        event_line = node1.succeed(
            "grep '\"event\":\"discovered\"' /tmp/discovery-events-node1.jsonl | grep '\"peer_id\":\"node2\"' | tail -1"
        ).strip()
        event = json.loads(event_line)
        assert any(
            a[0] == "192.168.1.2" and a[1] == 5000 for a in event["addrs"]
        ), f"Expected 192.168.1.2:5000 in {event['addrs']}"

        dump_events(node1, "node1", "After Phase 1")
        dump_events(node2, "node2", "After Phase 1")


    # ============================================================
    # Phase 2: Add VLAN 2 interface dynamically
    # ============================================================
    with subtest("Dynamic interface addition on VLAN 2"):
        send_command(node1, "node1", "add_interface 192.168.2.1")
        send_command(node2, "node2", "add_interface 192.168.2.2")

        send_command(node1, "node1", "add_addr 5000 192.168.2.1")
        send_command(node2, "node2", "add_addr 5000 192.168.2.2")

        # Wait for VLAN 2 addresses to appear in discovery events
        node1.wait_until_succeeds(
            "grep '192.168.2.2' /tmp/discovery-events-node1.jsonl",
            timeout=30,
        )
        node2.wait_until_succeeds(
            "grep '192.168.2.1' /tmp/discovery-events-node2.jsonl",
            timeout=30,
        )

        dump_events(node1, "node1", "After Phase 2")
        dump_events(node2, "node2", "After Phase 2")


    # ============================================================
    # Phase 3: Remove VLAN 1, verify discovery continues on VLAN 2
    # ============================================================
    with subtest("Interface removal - discovery continues on VLAN 2"):
        send_command(node1, "node1", "remove_interface 192.168.1.1")
        send_command(node2, "node2", "remove_interface 192.168.1.2")

        send_command(node1, "node1", "remove_addr 192.168.1.1")
        send_command(node2, "node2", "remove_addr 192.168.1.2")

        # Record line count so we can check only new events (avoids truncating the file)
        baseline_node1 = get_line_count(node1, "node1")
        baseline_node2 = get_line_count(node2, "node2")

        # Verify fresh discovery on VLAN 2 (only lines after baseline)
        node1.wait_until_succeeds(
            f"tail -n +{baseline_node1 + 1} /tmp/discovery-events-node1.jsonl"
            " | grep '\"event\":\"discovered\"' | grep -q '192.168.2.2'",
            timeout=30,
        )
        node2.wait_until_succeeds(
            f"tail -n +{baseline_node2 + 1} /tmp/discovery-events-node2.jsonl"
            " | grep '\"event\":\"discovered\"' | grep -q '192.168.2.1'",
            timeout=30,
        )

        # Verify VLAN 1 addresses are no longer advertised in new events
        latest_line = node1.succeed(
            f"tail -n +{baseline_node1 + 1} /tmp/discovery-events-node1.jsonl"
            " | grep '\"event\":\"discovered\"' | grep '\"peer_id\":\"node2\"' | tail -1"
        ).strip()
        latest = json.loads(latest_line)
        for addr_pair in latest["addrs"]:
            assert addr_pair[0] != "192.168.1.2", \
                f"VLAN 1 address should not appear after removal: {latest['addrs']}"

        dump_events(node1, "node1", "After Phase 3")
        dump_events(node2, "node2", "After Phase 3")


    # ============================================================
    # Phase 4: Shutdown and verify peer loss detection
    # ============================================================
    with subtest("Peer loss detection on shutdown"):
        send_command(node2, "node2", "shutdown")

        node1.wait_until_succeeds(
            "grep '\"event\":\"lost\"' /tmp/discovery-events-node1.jsonl | grep -q '\"peer_id\":\"node2\"'",
            timeout=30,
        )

        dump_events(node1, "node1", "After Phase 4")

        send_command(node1, "node1", "shutdown")
  '';
}
