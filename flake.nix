{
  description = "wispd development environment";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, flake-utils }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = import nixpkgs { inherit system; };
      in {
        packages.wispd = pkgs.rustPlatform.buildRustPackage {
          pname = "wispd";
          version = "0.1.0";
          src = ./.;
          cargoLock.lockFile = ./Cargo.lock;

          nativeBuildInputs = [ pkgs.pkg-config ];
          buildInputs = [
            pkgs.wayland
            pkgs.libxkbcommon
          ];
        };

        packages.default = self.packages.${system}.wispd;

        apps.wispd = {
          type = "app";
          program = "${self.packages.${system}.wispd}/bin/wispd";
        };

        apps.default = self.apps.${system}.wispd;

        devShells.default = pkgs.mkShell {
          packages = with pkgs; [
            rustc
            cargo
            rustfmt
            clippy
            rust-analyzer
            pkg-config
            wayland
            libxkbcommon
          ];

          LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath [
            pkgs.wayland
            pkgs.libxkbcommon
          ];
        };
      });
}
