#!/usr/bin/env python3
"""Regenerate the MessagePack fixtures consumed by the Rust tracker tests.

This script is a development-only fixture generator. The zeronet-rs runtime
contains no Python dependency and never executes this file.
Requires: python3 -m pip install msgpack
"""

from pathlib import Path

import msgpack

HERE = Path(__file__).resolve().parent

response = {
    "cmd": "response",
    "to": 1,
    "peers": [
        {
            "ip4": [bytes([8, 8, 8, 8, 0x51, 0x3C])],
            "ipv4": [],
            "ipv6": [
                bytes([
                    0x20, 0x01, 0x0D, 0xB8, 0, 0, 0, 0,
                    0, 0, 0, 0, 0, 0, 0, 1, 0x51, 0x3C,
                ])
            ],
            "onion": [],
        }
    ],
}

empty = {"cmd": "response", "to": 1, "peers": []}

for name, value in (
    ("zero_announce_response.msgpack.hex", response),
    ("zero_announce_empty.msgpack.hex", empty),
):
    payload = msgpack.packb(value, use_bin_type=True)
    (HERE / name).write_text(payload.hex() + "\n", encoding="ascii")
