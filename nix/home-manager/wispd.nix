{ self }:
{ config, lib, pkgs, ... }:
let
  cfg = config.services.wispd;
  defaultPackage = lib.attrByPath [ "packages" pkgs.system "wispd" ] null self;
  runtimeLibPath = lib.makeLibraryPath [
    pkgs.wayland
    pkgs.libxkbcommon
  ];
in
{
  options.services.wispd = {
    enable = lib.mkEnableOption "wispd notification daemon";

    package = lib.mkOption {
      type = lib.types.nullOr lib.types.package;
      default = defaultPackage;
      defaultText = lib.literalExpression "inputs.wispd.packages.${pkgs.system}.wispd";
      description = "wispd package to run.";
    };

    rustLog = lib.mkOption {
      type = lib.types.nullOr lib.types.str;
      default = null;
      example = "info,wispd=debug,wisp_source=debug";
      description = "Optional RUST_LOG value for the wispd user service.";
    };

    extraEnvironment = lib.mkOption {
      type = lib.types.attrsOf lib.types.str;
      default = { };
      description = "Extra environment variables passed to the wispd user service.";
    };
  };

  config = lib.mkIf cfg.enable {
    assertions = [
      {
        assertion = cfg.package != null;
        message = ''
          services.wispd.package is null.
          Could not derive a default package from flake output for system `${pkgs.system}`.
          Set it explicitly, for example:
            services.wispd.package = inputs.wispd.packages.${pkgs.system}.wispd;
        '';
      }
    ];

    systemd.user.services.wispd = {
      Unit = {
        Description = "wispd notification daemon";
        Documentation = [ "https://github.com/dmnt/wispd" ];
        After = [ "graphical-session.target" "dbus.service" ];
        PartOf = [ "graphical-session.target" ];
      };

      Service = {
        ExecStart = "${cfg.package}/bin/wispd";
        Restart = "on-failure";
        RestartSec = 1;
        Environment =
          [
            "LD_LIBRARY_PATH=${runtimeLibPath}"
            "PATH=${lib.makeBinPath [ cfg.package pkgs.wayland pkgs.libxkbcommon ]}:$PATH"
          ]
          ++ lib.optional (cfg.rustLog != null) "RUST_LOG=${cfg.rustLog}"
          ++ lib.mapAttrsToList (name: value: "${name}=${value}") cfg.extraEnvironment;
      };

      Install = {
        WantedBy = [ "graphical-session.target" ];
      };
    };
  };
}
