{
  description = "swarm-discovery dev shell";

  inputs = {
    nixpkgs.url      = "github:NixOS/nixpkgs/nixos-unstable";
    rust-overlay.url = "github:oxalica/rust-overlay";
    flake-utils.url  = "github:numtide/flake-utils";
    crate2nix.url    = "github:nix-community/crate2nix";
  };

  outputs = { self, nixpkgs, rust-overlay, flake-utils, crate2nix, ... }:
    let
      linuxSystems = [ "x86_64-linux" "aarch64-linux" ];
    in
    flake-utils.lib.eachDefaultSystem (system:
      let
        overlays = [ (import rust-overlay) ];
        pkgs = import nixpkgs {
          inherit system overlays;
        };

        cargoNix = (import ./Cargo.nix {
          inherit pkgs;
          release = true;
        });

        testNode = pkgs.rustPlatform.buildRustPackage {
          pname = "swarm-discovery-test-node";
          version = "0.1.0";
          src = pkgs.lib.cleanSource ./.;
          cargoRoot = "tests/nixos/test-node";
          cargoLock.lockFile = ./tests/nixos/test-node/Cargo.lock;
          buildAndTestSubdir = "tests/nixos/test-node";
          doCheck = false;
        };
      in
      {
        packages = {
          default = cargoNix.rootCrate.build;
          test-node = testNode;
        };

        devShells.default = pkgs.mkShell {
          packages = with pkgs; [
            rust-bin.nightly.latest.default
            rust-analyzer
            crate2nix.packages.${system}.default
          ];
        };
      } // pkgs.lib.optionalAttrs (builtins.elem system linuxSystems) {
        checks.multicast-discovery = import ./tests/nixos/vm-test.nix {
          inherit pkgs;
          inherit testNode;
        };
        checks.multi-interface = import ./tests/nixos/vlan-subinterface-test.nix {
          inherit pkgs;
          inherit testNode;
        };
        checks.drop-cleanup = import ./tests/nixos/drop-cleanup-test.nix {
          inherit pkgs;
          inherit testNode;
        };
      }
    );
}
