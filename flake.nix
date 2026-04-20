{
  description = "Simple helper for managing docker-stack-deploy containers";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";

    rust-overlay.url = "github:oxalica/rust-overlay";
    rust-overlay.inputs.nixpkgs.follows = "nixpkgs";
  };

  outputs =
    {
      self,
      nixpkgs,
      rust-overlay,
    }:
    let
      systems = [
        "x86_64-linux"
        "aarch64-linux"
        "x86_64-darwin"
        "aarch64-darwin"
      ];
      forAllSystems =
        f:
        nixpkgs.lib.genAttrs systems (
          system:
          f {
            inherit system;
            pkgs = import nixpkgs {
              inherit system;
              overlays = [
                rust-overlay.overlays.default
              ];
            };
          }
        );
    in
    {
      overlays.default = final: prev: {
        dsd-util = self.packages.${final.system}.default;
      };

      formatter = forAllSystems ({ pkgs, ... }: pkgs.nixfmt);

      packages = forAllSystems (
        { pkgs, system, ... }:
        let
          rustMinimalPlatform =
            let
              platform = pkgs.rust-bin.stable.latest.minimal;
            in
            pkgs.makeRustPlatform {
              rustc = platform;
              cargo = platform;
            };
        in
        {
          dsd-util = pkgs.callPackage ./default.nix {
            rustPlatform = rustMinimalPlatform;
          };
          default = self.packages.${system}.dsd-util;
        }
      );

      devShells = forAllSystems (
        { pkgs, system, ... }:
        {
          default =
            let
              rustShellToolchain = pkgs.rust-bin.stable.latest.default.override {
                extensions = [
                  "rust-src"
                  "rust-analyzer"
                ];
              };
              dsd-util = self.packages.${system}.default;
            in
            pkgs.mkShell {
              name = "dsd-util";
              packages = [
                rustShellToolchain
              ]
              ++ dsd-util.nativeBuildInputs
              ++ dsd-util.buildInputs;
            };
        }
      );
    };
}
