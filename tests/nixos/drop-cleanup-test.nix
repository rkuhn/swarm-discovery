{ pkgs, testNode }:

pkgs.testers.nixosTest {
  name = "swarm-discovery-drop-cleanup";

  nodes = {
    node = { ... }: {
      virtualisation.vlans = [ 1 ];

      networking.useDHCP = false;
      networking.interfaces.eth1.ipv4.addresses = [{
        address = "192.168.1.1";
        prefixLength = 24;
      }];

      networking.firewall.enable = false;

      environment.systemPackages = [ testNode ];
    };
  };

  testScript = ''
    start_all()

    # Wait for network interface to be configured
    node.wait_until_succeeds("ip addr show eth1 | grep -q '192.168.1.1'")

    # ============================================================
    # Phase 1: Start the drop-test binary and wait for discovery
    # ============================================================
    with subtest("Start discovery and verify it is running"):
        node.succeed(
            "RUST_LOG=debug drop_test eth1 192.168.1.1 &> /tmp/drop-test.log &"
        )

        # Wait for the discoverer to be fully running
        node.wait_for_file("/tmp/drop-test-running", timeout=30)

    # ============================================================
    # Phase 2: Verify no stray actors after drop
    # ============================================================
    with subtest("Verify all actors stopped and no messages sent to closed mailboxes"):
        # Wait for the done signal (runtime shutdown succeeded)
        node.wait_for_file("/tmp/drop-test-done", timeout=30)

        # Verify the binary reported success
        node.succeed("grep -q PASS /tmp/drop-test-done")

        # The critical check: after dropping the DropGuard, no actor
        # should still be attempting to send messages.  If cleanup is
        # incomplete, acto logs "dropping message due to closed mailbox"
        # because a sender/updater/receiver is still alive and trying to
        # communicate with an already-stopped peer.
        node.fail("grep -q 'dropping message due to closed mailbox' /tmp/drop-test.log")

    # Dump the log for debugging on failure
    print(node.succeed("cat /tmp/drop-test.log"))
  '';
}
