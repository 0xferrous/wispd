{
  description = "wispd development environment";

  nixConfig = {
    extra-substituters = [ "https://microvm.cachix.org" ];
    extra-trusted-public-keys = [ "microvm.cachix.org-1:oXnBc6hRE3eX5rSYdRyMYXnfzcCxC7yKPTbZXALsqys=" ];
  };

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    crane.url = "github:ipetkov/crane";
    microvm = {
      url = "github:microvm-nix/microvm.nix";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = { self, nixpkgs, flake-utils, crane, microvm }:
    {
      nixosConfigurations.wispd-microvm = nixpkgs.lib.nixosSystem {
        system = "x86_64-linux";
        specialArgs = { inherit self; };
        modules = [
          microvm.nixosModules.microvm
          ./nix/microvm/wispd-microvm.nix
        ];
      };
    }
    // flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = import nixpkgs { inherit system; };
        craneLib = crane.mkLib pkgs;
        src = craneLib.cleanCargoSource ./.;

        commonArgs = {
          pname = "wispd-workspace";
          version = "0.1.0";
          inherit src;
          strictDeps = true;
          nativeBuildInputs = [ pkgs.pkg-config ];
          buildInputs = [
            pkgs.wayland
            pkgs.libxkbcommon
            pkgs.openssl
            pkgs.zlib
          ];
        };

        cargoArtifacts = craneLib.buildDepsOnly commonArgs;
      in {
        packages = {
          wispd = craneLib.buildPackage (commonArgs // {
            pname = "wispd";
            version = "0.1.0";
            inherit cargoArtifacts;
            cargoExtraArgs = "--package wispd";
            nativeBuildInputs = commonArgs.nativeBuildInputs ++ [ pkgs.makeWrapper ];
            postFixup = ''
              wrapProgram $out/bin/wispd \
                --prefix LD_LIBRARY_PATH : ${pkgs.lib.makeLibraryPath [
                  pkgs.wayland
                  pkgs.libxkbcommon
                ]}
            '';
          });

          wispd-monitor = craneLib.buildPackage (commonArgs // {
            pname = "wispd-monitor";
            version = "0.1.0";
            inherit cargoArtifacts;
            cargoExtraArgs = "--package wispd-monitor";
          });

          wispd-forward = craneLib.buildPackage (commonArgs // {
            pname = "wispd-forward";
            version = "0.1.0";
            inherit cargoArtifacts;
            cargoExtraArgs = "--package wispd-forward";
          });

          default = self.packages.${system}.wispd;
        } // pkgs.lib.optionalAttrs (system == "x86_64-linux") {
          wispd-microvm = self.nixosConfigurations.wispd-microvm.config.microvm.declaredRunner;
        };

        apps = {
          wispd = {
            type = "app";
            program = "${self.packages.${system}.wispd}/bin/wispd";
          };

          wispd-monitor = {
            type = "app";
            program = "${self.packages.${system}.wispd-monitor}/bin/wispd-monitor";
          };

          wispd-forward = {
            type = "app";
            program = "${self.packages.${system}.wispd-forward}/bin/wispd-forward";
          };

          default = self.apps.${system}.wispd;
        } // pkgs.lib.optionalAttrs (system == "x86_64-linux") {
          wispd-microvm = {
            type = "app";
            program = "${self.packages.${system}.wispd-microvm}/bin/microvm-run";
          };
        };

        devShells.default = pkgs.mkShell {
          inputsFrom = [ self.packages.${system}.wispd ];

          packages = with pkgs; [
            rustc
            cargo
            rustfmt
            clippy
            rust-analyzer
            pkg-config
          ];
        };
      });
}
