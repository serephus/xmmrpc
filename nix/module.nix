{ config, pkgs, lib, ... }:

let
  cfg = config.services.xmmrpc;

  inherit (lib) mkEnableOption mkIf mkOption types;

  xmmrpcPackage =
    if cfg.package != null then
      cfg.package
    else
      pkgs.callPackage ./package.nix { };

  xmmrpcConfigFile =
    pkgs.writeText "xmmrpc.ini" (lib.generators.toKeyValue { } cfg.config);
in
{
  options.services.xmmrpc = {
    enable = mkEnableOption "xmmrpc control of the Intel XMM7360 (Fibocom L850-GL) modem through the in-tree iosm driver";

    autoStart = mkOption {
      type = types.bool;
      default = false;
      description = "Start the modem configuration service on boot.";
    };

    config = mkOption {
      type = with types; attrsOf (oneOf [ bool int str ]);
      default = { };
      example = {
        apn = "3gnet";
        nodefaultroute = false;
        noresolv = true;
      };
      description = ''
        xmmrpc.ini configuration written as a flat attribute set. Supported
        keys are the arguments of `xmmrpc` (e.g. `apn`, `nodefaultroute`,
        `metric`, `ip-fetch-timeout`, `noresolv`). `apn` is required.
      '';
    };

    package = mkOption {
      type = types.nullOr types.package;
      default = null;
      description = ''
        xmmrpc package to use. If left as `null`, the package is built from
        this flake.
      '';
    };
  };

  config = mkIf cfg.enable {
    assertions = [{
      assertion = cfg.config ? apn;
      message = ''
        services.xmmrpc.config must contain an `apn` attribute, e.g.
        `services.xmmrpc.config.apn = "your.apn.here";`.
      '';
    }];

    # The in-tree `iosm` driver claims the same PCI device (8086:7360).
    boot.kernelModules = [ "iosm" ];
    boot.blacklistedKernelModules = [ "xmm7360" ];

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
          if [ -e /dev/wwan0xmmrpc0 ]; then
            exit 0
          fi
          sleep 1
          i=$((i + 1))
        done
        echo "timed out waiting for /dev/wwan0xmmrpc0" >&2
        exit 1
      '';

      script = ''
        exec ${xmmrpcPackage}/bin/xmmrpc -c ${xmmrpcConfigFile}
      '';

      serviceConfig = {
        Type = "oneshot";
        RemainAfterExit = true;
        # After a reset the modem can take a while to register and attach;
        # kill it only once it is clearly not coming back.
        TimeoutStartSec = "3min";
        Restart = "on-failure";
        RestartSec = "30s";
        # Exit code 2 means "the network never gave us an IP" (usually no data
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
