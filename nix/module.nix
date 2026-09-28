{
  config,
  pkgs,
  lib,
  ...
}:

let
  cfg = config.services.xmmrpc;

  inherit (lib)
    mkEnableOption
    mkIf
    mkOption
    types
    ;

  toml = pkgs.formats.toml { };

  xmmrpcPackage =
    if cfg.package != null then
      cfg.package
    else
      pkgs.callPackage ./package.nix { };

  # The Rust tool reads snake_case TOML keys; the Nix options use camelCase.
  xmmrpcConfig = {
    apn = cfg.settings.apn;
    interface = cfg.settings.interface;
    rpc_port = cfg.settings.rpcPort;
    default_route = cfg.settings.defaultRoute;
    metric = cfg.settings.metric;
    ip_fetch_interval = cfg.settings.ipFetchInterval;
    ip_wait = cfg.settings.ipWait;
    write_resolv = cfg.settings.writeResolv;
    resolv_conf = cfg.settings.resolvConf;
    datachannel_path = cfg.settings.datachannelPath;
  };

  xmmrpcConfigFile = toml.generate "xmmrpc.toml" xmmrpcConfig;
in
{
  options.services.xmmrpc = {
    enable = mkEnableOption "xmmrpc control of the Intel XMM7360 (Fibocom L850-GL) modem through the in-tree iosm driver";

    autoStart = mkOption {
      type = types.bool;
      default = false;
      description = "Start the modem configuration service on boot.";
    };

    settings = {
      apn = mkOption {
        type = types.str;
        example = "3gnet";
        description = "Network provider APN.";
      };

      interface = mkOption {
        type = types.str;
        default = "wwan0";
        description = "WWAN network interface created by iosm.";
      };

      rpcPort = mkOption {
        type = types.str;
        default = "/dev/wwan0xmmrpc0";
        description = "XMM RPC control port.";
      };

      defaultRoute = mkOption {
        type = types.bool;
        default = true;
        description = "Install the modem as the default route.";
      };

      metric = mkOption {
        type = types.ints.unsigned;
        default = 1000;
        description = "Default route metric (higher is lower priority).";
      };

      ipFetchInterval = mkOption {
        type = types.ints.positive;
        default = 1;
        description = "Seconds between attempts to fetch the assigned IP address.";
      };

      ipWait = mkOption {
        type = types.ints.positive;
        default = 120;
        description = "Seconds to wait for the network to assign an address.";
      };

      writeResolv = mkOption {
        type = types.bool;
        default = true;
        description = "Append modem-provided DNS servers to the resolver configuration.";
      };

      resolvConf = mkOption {
        type = types.str;
        default = "/etc/resolv.conf";
        description = "Resolver configuration file to append DNS servers to.";
      };

      datachannelPath = mkOption {
        type = types.str;
        default = "/sioscc/PCIE/IOSM/IPS/0";
        description = "Firmware data-channel path.";
      };
    };

    package = mkOption {
      type = types.nullOr types.package;
      default = null;
      description = ''
        xmmrpc package to use. If left as `null`, the package shipped with this
        module is built.
      '';
    };
  };

  config = mkIf cfg.enable {
    # The in-tree `iosm` driver claims the same PCI device (8086:7360).
    boot.kernelModules = [ "iosm" ];

    # The modem has no power management support: it powers off during suspend
    # and has to be reconfigured when the machine resumes.
    powerManagement.resumeCommands = ''
      ${pkgs.systemd}/bin/systemctl --no-block try-restart xmmrpc.service
    '';

    systemd.services.xmmrpc = {
      wantedBy = lib.optionals cfg.autoStart [ "multi-user.target" ];
      description = "xmmrpc - Intel XMM7360 data connection";

      # The RPC control port is created by iosm once the modem is probed.
      preStart = ''
        i=0
        while [ "$i" -lt 60 ]; do
          if [ -e ${lib.escapeShellArg cfg.settings.rpcPort} ]; then
            exit 0
          fi
          sleep 1
          i=$((i + 1))
        done
        echo "timed out waiting for ${cfg.settings.rpcPort}" >&2
        exit 1
      '';

      script = ''
        exec ${xmmrpcPackage}/bin/xmmrpc --config ${xmmrpcConfigFile}
      '';

      serviceConfig = {
        Type = "oneshot";
        RemainAfterExit = true;
        # After a reset the modem can take a while to register and attach.
        TimeoutStartSec = "3min";
        Restart = "on-failure";
        RestartSec = "30s";
        # Exit code 2 means the network never gave us an IP (usually no data
        # allowance/credit). That is not a firmware hang, so don't reset and
        # reload the modem over and over for it.
        RestartPreventExitStatus = [ 2 ];
      };

      unitConfig = {
        # Don't spin forever resetting/reloading a modem that never comes up.
        StartLimitBurst = 3;
        StartLimitIntervalSec = "15min";
      };
    };
  };
}
