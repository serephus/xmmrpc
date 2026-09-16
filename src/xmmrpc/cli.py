#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-2.0 OR BSD-3-Clause
#
# Adapted from xmm7360-pci <https://github.com/xmm7360/xmm7360-pci>.
# The reverse-engineered RPC protocol implementation remains under its
# original upstream dual license.

import logging
import os
import socket
import sys
import time

import configargparse
from pyroute2 import IPRoute

from . import rpc

DEFAULT_CONFIG_FILES = [
    "/etc/xmmrpc.ini",
    os.path.join(os.getcwd(), "xmmrpc.ini"),
]

# Seconds to wait for the network to assign an address.
IP_WAIT = 120


def build_parser():
    parser = configargparse.ArgumentParser(
        prog="xmmrpc",
        description=(
            "Bring up an Intel XMM7360 (Fibocom L850-GL) data connection "
            "through the in-tree iosm driver."
        ),
        default_config_files=DEFAULT_CONFIG_FILES,
    )
    parser.add_argument("-c", "--conf", is_config_file=True, help="configuration file")
    parser.add_argument("-a", "--apn", required=True, help="network provider APN")
    parser.add_argument(
        "-i",
        "--interface",
        default="wwan0",
        help="WWAN network interface created by iosm " "(default: %(default)s)",
    )
    parser.add_argument(
        "--rpc-port",
        default=rpc.DEFAULT_RPC_PORTS[0],
        help="XMM RPC control port (default: %(default)s)",
    )
    parser.add_argument(
        "-n",
        "--nodefaultroute",
        action="store_true",
        help="don't install the modem as the default route",
    )
    parser.add_argument(
        "-m",
        "--metric",
        type=int,
        default=1000,
        help="metric for the default route " "(higher is lower priority)",
    )
    parser.add_argument(
        "-t",
        "--ip-fetch-timeout",
        type=int,
        default=1,
        help="retry interval in seconds when fetching IP " "configuration",
    )
    parser.add_argument(
        "-r",
        "--noresolv",
        action="store_true",
        help="don't add modem-provided DNS servers to " "/etc/resolv.conf",
    )
    return parser


def find_interface(ipr, name):
    idx = ipr.link_lookup(ifname=name)
    if not idx:
        logging.error("network interface %s does not exist", name)
        return None
    return idx[0]


def configure_interface(ipr, idx, ip_addr, cfg):
    ipr.flush_addr(index=idx)
    ipr.link("set", index=idx, state="up")
    ipr.addr("add", index=idx, address=ip_addr)

    if not cfg.nodefaultroute:
        ipr.route(
            "add", dst="default", priority=cfg.metric, oif=idx, family=socket.AF_INET
        )


def write_resolv(dns_values):
    with open("/etc/resolv.conf", "a") as resolv:
        resolv.write("\n# Added by xmmrpc\n")
        for dns in dns_values["v4"] + dns_values["v6"]:
            resolv.write(f"nameserver {dns}\n")


def attach(r):
    """Attach to the packet-switched network, waiting for the modem if needed."""
    r.execute(
        "UtaMsCallPsAttachApnConfigReq",
        rpc.pack_UtaMsCallPsAttachApnConfigReq(r.apn),
        is_async=True,
    )

    response = r.execute(
        "UtaMsNetAttachReq", rpc.pack_UtaMsNetAttachReq(), is_async=True
    )
    _, status = rpc.unpack("nn", response["body"])

    if status == 0xFFFFFFFF:
        logging.info("Attach failed - waiting to see if we were just not ready")
        while not r.attach_allowed:
            r.pump()
        response = r.execute(
            "UtaMsNetAttachReq", rpc.pack_UtaMsNetAttachReq(), is_async=True
        )
        _, status = rpc.unpack("nn", response["body"])

    if status == 0xFFFFFFFF:
        logging.error(
            "The network refused the packet-data attach. This "
            "usually means the SIM has no data allowance (out of "
            "credit) or the APN is wrong."
        )
        return False
    return True


def wait_for_ip(r, timeout):
    deadline = time.monotonic() + timeout
    while True:
        ip_addr, dns_values = rpc.get_ip(r)
        if ip_addr is not None:
            return ip_addr, dns_values
        if time.monotonic() >= deadline:
            logging.error(
                "No IP address was assigned by the network within "
                "%d seconds. This usually means the SIM has no data "
                "allowance (out of credit), the APN is wrong, or the "
                "network refused the data session.",
                timeout,
            )
            return None, None
        logging.info(
            "IP address couldn't be fetched, waiting %d seconds", r.ip_fetch_timeout
        )
        time.sleep(r.ip_fetch_timeout)


def open_data_channel(r):
    pscr = r.execute(
        "UtaMsCallPsConnectReq", rpc.pack_UtaMsCallPsConnectReq(), is_async=True
    )
    dcr = r.execute(
        "UtaRPCPsConnectToDatachannelReq", rpc.pack_UtaRPCPsConnectToDatachannelReq()
    )
    csr_req = pscr["body"][:-6] + dcr["body"] + b"\x02\x04\0\0\0\0"
    r.execute("UtaRPCPSConnectSetupReq", csr_req)


def main(argv=None):
    logging.basicConfig(level=logging.INFO, format="%(levelname)s: %(message)s")
    cfg = build_parser().parse_args(argv)

    try:
        r = rpc.XMMRPC([cfg.rpc_port])
    except OSError as ex:
        logging.error(ex)
        return 1

    # Runtime knobs consumed by the helpers above.
    r.apn = cfg.apn
    r.ip_fetch_timeout = cfg.ip_fetch_timeout

    ipr = IPRoute()

    r.execute("UtaMsSmsInit")
    r.execute("UtaMsCbsInit")
    r.execute("UtaMsNetOpen")
    r.execute("UtaMsCallCsInit")
    r.execute("UtaMsCallPsInitialize")
    r.execute("UtaMsSsInit")
    r.execute("UtaMsSimOpenReq")

    rpc.do_fcc_unlock(r)
    # Disable airplane mode if the modem had been FCC-locked. The first and
    # second arguments are don't-cares.
    rpc.UtaModeSet(r, 1)

    if not attach(r):
        return 2

    ip_addr, dns_values = wait_for_ip(r, IP_WAIT)
    if ip_addr is None:
        return 2

    logging.info("IP address: %s", ip_addr)
    logging.info(
        "DNS server(s): %s", ", ".join(map(str, dns_values["v4"] + dns_values["v6"]))
    )

    idx = find_interface(ipr, cfg.interface)
    if idx is None:
        return 1

    configure_interface(ipr, idx, ip_addr, cfg)

    if not cfg.noresolv:
        write_resolv(dns_values)

    open_data_channel(r)

    return 0


if __name__ == "__main__":
    sys.exit(main())
