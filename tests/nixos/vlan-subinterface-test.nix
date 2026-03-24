{ pkgs, testNode }:

# Multi-interface discovery test with 3 VMs across 2 subnets.
#
# Topology:
#   Subnet X (VLAN 1, 192.168.1.0/24):  A --- B
#   Subnet Y (VLAN 2, 192.168.2.0/24):        B --- C
#
#   A (192.168.1.1)  — only on subnet X
#   B (192.168.1.2, 192.168.2.2) — on both subnets (multi-homed)
#   C (192.168.2.3)  — only on subnet Y
#
# B runs multicast on both interfaces. This validates that:
#   1. A discovers B (via subnet X)
#   2. C discovers B (via subnet Y)
#   3. B discovers both A and C (via its two interfaces)

pkgs.testers.nixosTest {
  name = "swarm-discovery-multi-interface";

  nodes = {
    a = { ... }: {
      virtualisation.vlans = [ 1 ];
      networking.useDHCP = false;
      networking.interfaces.eth1.ipv4.addresses = [{
        address = "192.168.1.1";
        prefixLength = 24;
      }];
      networking.firewall.enable = false;
      environment.systemPackages = [ testNode ];
    };

    b = { ... }: {
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

    c = { ... }: {
      virtualisation.vlans = [ 2 ];
      networking.useDHCP = false;
      networking.interfaces.eth1.ipv4.addresses = [{
        address = "192.168.2.3";
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


    def send_command(machine, peer_id, command):
        cmd_file = f"/tmp/discovery-cmd-{peer_id}"
        machine.succeed(f"echo '{command}' > {cmd_file}")
        time.sleep(0.5)


    def dump_events(machine, peer_id, label):
        events_file = f"/tmp/discovery-events-{peer_id}.jsonl"
        content = machine.succeed(f"cat {events_file}").strip()
        print(f"=== {label}: {peer_id} events ===")
        for line in content.splitlines():
            print(f"  {line}")
        if not content:
            print("  (empty)")
        print(f"=== end {peer_id} ===")


    # ============================================================
    # Phase 0: Verify network topology
    # ============================================================
    with subtest("Verify network topology"):
        a.wait_until_succeeds("ip addr show eth1 | grep -q '192.168.1.1'")
        b.wait_until_succeeds("ip addr show eth1 | grep -q '192.168.1.2'")
        b.wait_until_succeeds("ip addr show eth2 | grep -q '192.168.2.2'")
        c.wait_until_succeeds("ip addr show eth1 | grep -q '192.168.2.3'")

        # A <-> B on subnet X
        a.succeed("ping -c 1 192.168.1.2")
        b.succeed("ping -c 1 192.168.1.1")

        # B <-> C on subnet Y
        b.succeed("ping -c 1 192.168.2.3")
        c.succeed("ping -c 1 192.168.2.2")

        # Dump topology
        for node_name, node in [("a", a), ("b", b), ("c", c)]:
            print(f"=== {node_name} interfaces ===")
            print(node.succeed("ip -brief addr show"))


    # ============================================================
    # Phase 1: Start all nodes
    #
    #   A: single interface (eth1, subnet X)
    #   B: two interfaces (eth1 subnet X, eth2 subnet Y)
    #   C: single interface (eth1, subnet Y)
    # ============================================================
    with subtest("Start discovery on all nodes"):
        a.succeed(
            "RUST_LOG=debug swarm-discovery-test-node nodea 5000 eth1 &> /tmp/test-a.log &"
        )
        b.succeed(
            "RUST_LOG=debug swarm-discovery-test-node nodeb 5000 eth1 eth2 &> /tmp/test-b.log &"
        )
        c.succeed(
            "RUST_LOG=debug swarm-discovery-test-node nodec 5000 eth1 &> /tmp/test-c.log &"
        )

        a.wait_for_file("/tmp/discovery-ready-nodea")
        b.wait_for_file("/tmp/discovery-ready-nodeb")
        c.wait_for_file("/tmp/discovery-ready-nodec")


    # ============================================================
    # Phase 2: Verify A discovers B on subnet X
    # ============================================================
    with subtest("A discovers B via subnet X"):
        a.wait_until_succeeds(
            "grep '\"event\":\"discovered\"' /tmp/discovery-events-nodea.jsonl"
            " | grep -q '\"peer_id\":\"nodeb\"'",
            timeout=30,
        )

        event_line = a.succeed(
            "grep '\"event\":\"discovered\"' /tmp/discovery-events-nodea.jsonl"
            " | grep '\"peer_id\":\"nodeb\"' | tail -1"
        ).strip()
        event = json.loads(event_line)
        assert any(
            addr[0] == "192.168.1.2" and addr[1] == 5000 for addr in event["addrs"]
        ), f"A should see B's subnet X address 192.168.1.2:5000, got {event['addrs']}"

        dump_events(a, "nodea", "A discovers B")


    # ============================================================
    # Phase 3: Verify C discovers B on subnet Y
    # ============================================================
    with subtest("C discovers B via subnet Y"):
        c.wait_until_succeeds(
            "grep '\"event\":\"discovered\"' /tmp/discovery-events-nodec.jsonl"
            " | grep -q '\"peer_id\":\"nodeb\"'",
            timeout=30,
        )

        event_line = c.succeed(
            "grep '\"event\":\"discovered\"' /tmp/discovery-events-nodec.jsonl"
            " | grep '\"peer_id\":\"nodeb\"' | tail -1"
        ).strip()
        event = json.loads(event_line)
        assert any(
            addr[0] == "192.168.2.2" and addr[1] == 5000 for addr in event["addrs"]
        ), f"C should see B's subnet Y address 192.168.2.2:5000, got {event['addrs']}"

        dump_events(c, "nodec", "C discovers B")


    # ============================================================
    # Phase 4: Verify B discovers both A and C
    # ============================================================
    with subtest("B discovers both A and C"):
        b.wait_until_succeeds(
            "grep '\"event\":\"discovered\"' /tmp/discovery-events-nodeb.jsonl"
            " | grep -q '\"peer_id\":\"nodea\"'",
            timeout=30,
        )
        b.wait_until_succeeds(
            "grep '\"event\":\"discovered\"' /tmp/discovery-events-nodeb.jsonl"
            " | grep -q '\"peer_id\":\"nodec\"'",
            timeout=30,
        )

        # B sees A on subnet X
        event_line = b.succeed(
            "grep '\"event\":\"discovered\"' /tmp/discovery-events-nodeb.jsonl"
            " | grep '\"peer_id\":\"nodea\"' | tail -1"
        ).strip()
        event = json.loads(event_line)
        assert any(
            addr[0] == "192.168.1.1" and addr[1] == 5000 for addr in event["addrs"]
        ), f"B should see A at 192.168.1.1:5000, got {event['addrs']}"

        # B sees C on subnet Y
        event_line = b.succeed(
            "grep '\"event\":\"discovered\"' /tmp/discovery-events-nodeb.jsonl"
            " | grep '\"peer_id\":\"nodec\"' | tail -1"
        ).strip()
        event = json.loads(event_line)
        assert any(
            addr[0] == "192.168.2.3" and addr[1] == 5000 for addr in event["addrs"]
        ), f"B should see C at 192.168.2.3:5000, got {event['addrs']}"

        dump_events(b, "nodeb", "B discovers A and C")


    # ============================================================
    # Phase 5: Clean shutdown
    # ============================================================
    with subtest("Shutdown"):
        send_command(a, "nodea", "shutdown")
        send_command(b, "nodeb", "shutdown")
        send_command(c, "nodec", "shutdown")
  '';
}
