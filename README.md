# xmmrpc

Userspace control for the **Intel XMM7360** LTE modem (found in the Fibocom
L850-GL, e.g. in many ThinkPads and HP EliteBooks), driven by the **in-tree
`iosm` kernel driver**.

This is a from-scratch, RPC-only project. It contains:

- a Python implementation of the firmware RPC protocol used to configure the
  modem, and
- a Nix flake with a package and a NixOS module.

Unlike the original [`xmm7360-pci`](https://github.com/xmm7360/xmm7360-pci)
effort, it does **not** ship an out-of-tree kernel driver. The `iosm` driver
already supports `8086:7360` and exposes the modem's RPC channel as a WWAN
control port:

| Driver | Control interface | Data interface |
| --- | --- | --- |
| `iosm` (in-tree) | `/dev/wwan0xmmrpc0` | `wwan0` |

## Requirements

- Linux with `CONFIG_WWAN` and `CONFIG_IOSM` (both are set in the standard
  NixOS kernel as of 6.x; the module is `iosm`).
- Python >= 3.10, `configargparse`, `pyroute2`.

> **Note:** ModemManager is *not* used. As of ModemManager 1.24.2 the XMM7360
> RPC mode is explicitly unsupported (`Intel XMM7360 in RPC mode not
> supported`); support only exists on the unreleased development branch. This
> tool talks to the RPC port directly instead.

## Usage

Make sure `xmm7360` (the out-of-tree module) is not loaded and let `iosm`
claim the device:

```sh
sudo modprobe -r xmm7360 2>/dev/null || true
sudo modprobe iosm
```

Then bring the connection up:

```sh
sudo xmmrpc --apn your.apn.here
```

or use a configuration file:

```sh
cp xmmrpc.ini.sample xmmrpc.ini   # edit at least the APN
sudo xmmrpc -c xmmrpc.ini
```

Options (all of which can be placed in `xmmrpc.ini`):

| Option | Description |
| --- | --- |
| `-a`, `--apn` | Network provider APN (required). |
| `-i`, `--interface` | WWAN interface created by `iosm` (default `wwan0`). |
| `--rpc-port` | RPC control port (default `/dev/wwan0xmmrpc0`). |
| `-n`, `--nodefaultroute` | Do not install the modem as the default route. |
| `-m`, `--metric` | Metric for the default route (default `1000`). |
| `-t`, `--ip-fetch-timeout` | Retry interval while waiting for an address. |
| `-r`, `--noresolv` | Do not append modem-provided DNS servers to `/etc/resolv.conf`. |

The service exits with status `2` when the network refuses the attach or never
assigns an address (usually no data allowance or a wrong APN). This is a normal
"no connectivity" condition, not a firmware failure.

If the SIM has a PIN enabled, unlock it first, e.g.:

```sh
echo 'AT+CPIN="0000"' | sudo tee /dev/wwan0at0
```

## NixOS

```nix
{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    xmmrpc.url = "github:serephus/xmmrpc";
  };

  outputs = { nixpkgs, xmmrpc, ... }: {
    nixosConfigurations.example = nixpkgs.lib.nixosSystem {
      system = "x86_64-linux";
      modules = [
        xmmrpc.nixosModules.default
        {
          services.xmmrpc = {
            enable = true;
            autoStart = true;
            settings = {
              apn = "3gnet";
              nodefaultroute = false;
              noresolv = true;
            };
          };
        }
      ];
    };
  };
}
```

The module loads `iosm` and provides a `xmmrpc.service`. The service is
restarted after suspend/resume, because the modem has no power management
support and must be reconfigured.

### Options

- `services.xmmrpc.enable` — enable the driver/service.
- `services.xmmrpc.autoStart` — start the service at boot (default `false`).
- `services.xmmrpc.settings` — flat attribute set written to `xmmrpc.ini`.
- `services.xmmrpc.package` — override the package.

### Flake outputs

- `packages.<system>.xmmrpc` (also `default`) — the Python application.
- `overlays.default` — adds `xmmrpc` to nixpkgs.
- `nixosModules.default` — the NixOS module.

## Development

```sh
nix develop      # dev shell with python, uv, ruff, black, basedpyright
pip install -e '.[dev]'
pytest
ruff check .
black --check .
```

## License

Dual-licensed under `GPL-2.0 OR BSD-3-Clause`, the same terms as the
`xmm7360-pci` project this RPC implementation is adapted from. See
[LICENSE](LICENSE).
