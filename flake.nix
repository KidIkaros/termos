{
  description = "TermOS - Terminal multiplexer and window manager (Rust port of TUIOS)";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = { self, nixpkgs, flake-utils, rust-overlay, ... }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        overlays = [ (import rust-overlay) ];
        pkgs = import nixpkgs {
          inherit system overlays;
        };

        rustToolchain = pkgs.rust-bin.stable.latest.default.override {
          extensions = [ "rust-src" "rust-analyzer" "clippy" ];
        };

        termos = pkgs.callPackage ./nix/termos.nix { };

      in
      {
        packages = {
          default = termos;
          termos = termos;
        };

        apps = {
          default = {
            type = "app";
            program = "${termos}/bin/termos";
          };
          termos = {
            type = "app";
            program = "${termos}/bin/termos";
          };
        };

        devShells.default = pkgs.mkShell {
          buildInputs = [
            rustToolchain
            pkgs.cargo
            pkgs.rustfmt
            pkgs.clippy
          ] ++ pkgs.lib.optionals pkgs.stdenv.isLinux [
            pkgs.pkg-config
          ];

          RUST_SRC_PATH = "${rustToolchain}/lib/rustlib/src/rust/library";
        };
      });
}
