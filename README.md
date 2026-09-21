# xmmrpc

Userspace RPC control for the **Intel XMM7360** LTE modem (found in the
Fibocom L850-GL, e.g. in many ThinkPads and HP EliteBooks), driven by the
in-tree **`iosm`** kernel driver.

This is a Rust rewrite of the original Python `xmmrpc`. It speaks the
reverse-engineered firmware RPC protocol directly over the WWAN control port
exposed by `iosm`; no out-of-tree kernel module is required.

| Driver | Control interface | Data interface |
| --- | --- | --- |
| `iosm` (in-tree) | `/dev/wwan0xmmrpc0` | `wwan0` |

> **Note:** ModemManager is *not* used. It does not support the XMM7360 in RPC
> mode, so this tool talks to the RPC port itself.

## Requirements

- Linux with `CONFIG_WWAN` and `CONFIG_IOSM`.
- Root privileges (it opens the RPC control port and configures the network).

## Usage

Make sure the out-of-tree `xmm7360` module is not loaded and let `iosm` claim
the device:

```sh
sudo modprobe -r xmm7360 2>/dev/null || true
sudo modprobe iosm
```

Then bring the connection up:

```sh
sudo xmmrpc --apn your.apn.here
```

or with a configuration file:

```sh
cp xmmrpc.toml.example xmmrpc.toml   # edit at least the APN
sudo xmmrpc --config xmmrpc.toml
```

### Configuration

Configuration is a TOML file. Without `--config` the tool looks for
`./xmmrpc.toml`, then `/etc/xmmrpc.toml`. Command-line flags override the file,
which overrides the built-in defaults.

| Key | Flag | Default | Description |
| --- | --- | --- | --- |
| `apn` | `-a`, `--apn` | — | Network provider APN (required). |
| `interface` | `-i`, `--interface` | `wwan0` | WWAN interface created by `iosm`. |
| `rpc_port` | `--rpc-port` | `/dev/wwan0xmmrpc0` | RPC control port. |
| `default_route` | `--default-route <bool>` | `true` | Install the modem as the default route. |
| `metric` | `-m`, `--metric` | `1000` | Default route metric (higher is lower priority). |
| `ip_fetch_interval` | `-t`, `--ip-fetch-interval` | `1` | Seconds between address queries. |
| `ip_wait` | `--ip-wait` | `120` | Seconds to wait for an address. |
| `write_resolv` | `--write-resolv <bool>` | `true` | Append modem DNS servers to the resolver config. |
| `resolv_conf` | `--resolv-conf` | `/etc/resolv.conf` | Resolver configuration file. |
| `datachannel_path` | `--datachannel-path` | `/sioscc/PCIE/IOSM/IPS/0` | Firmware data-channel path. |

### Exit codes

| Code | Meaning |
| --- | --- |
| `0` | Success. |
| `1` | Hard failure (bad configuration, missing interface, I/O error, …). |
| `2` | No connectivity: the network refused the attach or never assigned an address. |

Exit code `2` is an ordinary "no data allowance or wrong APN" condition, not a
firmware failure.

### SIM PIN

If the SIM has a PIN enabled, unlock it first, e.g.:

```sh
echo 'AT+CPIN="0000"' | sudo tee /dev/wwan0at0
```

## Nix

```sh
nix build            # build the binary
nix develop          # Rust dev shell (rustc, cargo, clippy, rustfmt, rust-analyzer, nextest, taplo)
```

### NixOS

The flake ships a `services.xmmrpc` module. It loads `iosm`, restarts the
service after resume, and waits for the RPC port before configuring the modem.

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
              writeResolv = true;
            };
          };
        }
      ];
    };
  };
}
```

The module exposes a top-level `enable`/`autoStart`/`package` and a typed
`settings` submodule. The Nix options use camelCase; they map to the
snake_case TOML keys above.

| Option | Type | Default | Description |
| --- | --- | --- | --- |
| `settings.apn` | string | *required* | Network provider APN. |
| `settings.interface` | string | `wwan0` | WWAN interface created by `iosm`. |
| `settings.rpcPort` | string | `/dev/wwan0xmmrpc0` | RPC control port. |
| `settings.defaultRoute` | bool | `true` | Install the modem as the default route. |
| `settings.metric` | unsigned int | `1000` | Default route metric (higher is lower priority). |
| `settings.ipFetchInterval` | positive int | `1` | Seconds between address queries. |
| `settings.ipWait` | positive int | `120` | Seconds to wait for an address. |
| `settings.writeResolv` | bool | `true` | Append modem DNS servers to the resolver config. |
| `settings.resolvConf` | string | `/etc/resolv.conf` | Resolver configuration file. |
| `settings.datachannelPath` | string | `/sioscc/PCIE/IOSM/IPS/0` | Firmware data-channel path. |

| Option | Default | Description |
| --- | --- | --- |
| `enable` | `false` | Enable the module and the systemd service. |
| `autoStart` | `false` | Start the service on boot (`multi-user.target`). |
| `package` | `null` | Override the `xmmrpc` package; `null` builds the one shipped with the flake. |

## License

Dual-licensed under `GPL-2.0 OR BSD-3-Clause`, the same terms as the
`xmm7360-pci` project this RPC implementation is adapted from. See
[LICENSE](LICENSE).
