{ pkgs, ... }:
{
  system.stateVersion = "25.05";

  networking.hostName = "wispd-microvm";
  time.timeZone = "UTC";

  users.users.wisp = {
    isNormalUser = true;
    initialPassword = "wisp";
    extraGroups = [ "wheel" "video" "input" ];
  };

  programs.niri.enable = true;

  services.greetd = {
    enable = true;
    settings = {
      initial_session = {
        user = "wisp";
        command = "${pkgs.niri}/bin/niri-session";
      };
      default_session = {
        user = "wisp";
        command = "${pkgs.niri}/bin/niri-session";
      };
    };
  };

  environment.sessionVariables = {
    TERMINAL = "alacritty";
  };

  environment.systemPackages = [
    pkgs.alacritty
    pkgs.libnotify
    pkgs.wayland
    pkgs.libxkbcommon
  ];

  systemd.user.services.wispd = {
    description = "wispd notification daemon";
    partOf = [ "graphical-session.target" ];
    after = [ "graphical-session-pre.target" "dbus.service" ];
    wantedBy = [ "graphical-session.target" ];
    serviceConfig = {
      WorkingDirectory = "/work/wispd";
      ExecStart = "/work/wispd/target/debug/wispd";
      Environment = [
        "LD_LIBRARY_PATH=${pkgs.lib.makeLibraryPath [
          pkgs.wayland
          pkgs.libxkbcommon
        ]}"
      ];
      Restart = "on-failure";
      RestartSec = 1;
    };
  };

  services.dbus.implementation = "broker";
  security.polkit.enable = true;

  services.openssh = {
    enable = true;
    settings = {
      PasswordAuthentication = true;
      KbdInteractiveAuthentication = true;
    };
  };
  networking.firewall.allowedTCPPorts = [ 22 ];

  microvm = {
    hypervisor = "qemu";
    vcpu = 2;
    mem = 1536;
    graphics.enable = true;
    socket = "wispd-microvm.sock";

    shares = [
      {
        proto = "9p";
        tag = "ro-store";
        source = "/nix/store";
        mountPoint = "/nix/.ro-store";
      }
      {
        proto = "9p";
        tag = "wispd-workspace";
        source = ".";
        mountPoint = "/work/wispd";
      }
    ];

    interfaces = [
      {
        type = "user";
        id = "qemu";
        mac = "02:00:00:00:00:01";
      }
    ];

    forwardPorts = [
      {
        host.port = 2222;
        guest.port = 22;
      }
    ];
  };
}
