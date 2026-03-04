{ self }:
{ config, lib, pkgs, ... }:
let
  cfg = config.services.wispd;
  defaultPackage = lib.attrByPath [ "packages" pkgs.system "wispd" ] null self;
  runtimeLibPath = lib.makeLibraryPath [
    pkgs.wayland
    pkgs.libxkbcommon
  ];
  dbusActivationServicePackage = pkgs.writeTextDir "share/dbus-1/services/org.freedesktop.Notifications.service" ''
    [D-BUS Service]
    Name=org.freedesktop.Notifications
    SystemdService=wispd.service
    Exec=${cfg.package}/bin/wispd
  '';
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

    autostart = lib.mkOption {
      type = lib.types.bool;
      default = true;
      description = "Whether to start wispd with graphical-session.target in addition to D-Bus activation.";
    };

    dbusActivation = {
      enable = lib.mkOption {
        type = lib.types.bool;
        default = true;
        description = "Whether to include a D-Bus service definition for org.freedesktop.Notifications activation.";
      };
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

    dbus.packages = lib.optional cfg.dbusActivation.enable dbusActivationServicePackage;

    systemd.user.services.wispd = {
      Unit = {
        Description = "wispd notification daemon";
        Documentation = [ "https://github.com/dmnt/wispd" ];
        After = [ "graphical-session.target" "dbus.service" ];
        PartOf = [ "graphical-session.target" ];
      };

      Service = {
        Type = "dbus";
        BusName = "org.freedesktop.Notifications";
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

      Install = lib.mkIf cfg.autostart {
        WantedBy = [ "graphical-session.target" ];
      };
    };
  };
}
