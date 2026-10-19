# slcanio

[![CI](https://github.com/gmmyung/slcanio/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/gmmyung/slcanio/actions/workflows/ci.yml)
[![Coverage](https://img.shields.io/endpoint?url=https%3A%2F%2Fraw.githubusercontent.com%2Fgmmyung%2Fslcanio%2Fcoverage-badge%2Fcoverage.json)](https://github.com/gmmyung/slcanio/actions/workflows/ci.yml)
[![MSRV](https://img.shields.io/badge/rustc-1.85%2B-orange.svg)](Cargo.toml)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](Cargo.toml)

Cross-platform Rust SLCAN library for Classic CAN and CAN FD.

> **Status:** software tests pass on Linux, macOS and Windows. [Physical CAN FD validation is pending](HARDWARE_VALIDATION.md), so this is not yet a hardware-verified 0.1 release.

## Features

| Feature | What it provides |
|---|---|
| `tokio` (default) | Async serial socket, split sender/receiver and injectable test transport |
| `can-fd` (default) | CAN FD frames and `d/D/b/B` codec |
| `blocking` | Direct `serialport` API without a Tokio runtime |
| `--no-default-features` | Runtime-free Classic CAN codec |

Choose `AdapterProfile::LawicelClassic` or `AdapterProfile::CanableFd` explicitly. CANable's FD commands are a [firmware extension](https://github.com/normaldotcom/canable2-fw), not part of original LAWICEL SLCAN.

## Quick start

```rust,no_run
use slcanio::{AdapterProfile, Bitrate, CanFrame, CanId, CanSocket, Result};

# async fn example() -> Result<()> {
let socket = CanSocket::builder("/dev/ttyUSB0", 921_600)
    .adapter(AdapterProfile::CanableFd)
    .nominal_bitrate(Bitrate::Kbps500)
    .data_bitrate(Bitrate::Mbps2)
    .fd_enabled(true)
    .connect()
    .await?;

let (tx, mut rx) = socket.split();
tx.send(CanFrame::fd(CanId::standard(0x123)?, &[0x55; 12], true)?).await?;
if let Some(received) = rx.recv().await? {
    println!("{:?}", received.frame);
}
# Ok(())
# }
```

Port names use the same API: `/dev/ttyUSB0` on Linux, `/dev/cu.usbmodem...` on macOS or `COM3` on Windows. See the [Classic](examples/classic.rs), [FD](examples/fd.rs) and [blocking](examples/blocking.rs) examples.

## Behavior and limits

- **Bitrates:** UART speed, nominal CAN bitrate and FD data bitrate are separate. Set the FD data bitrate or explicitly accept the firmware's 2 Mbit/s default with `accept_default_data_bitrate(true)`.
- **Send result:** CANable reports a complete serial write; the LAWICEL profile waits for adapter command acceptance. Neither proves CAN bus acknowledgment. A canceled queued send may still be transmitted.
- **Receive:** `recv()` is cancellation safe. RX overflow uses `DropNewest` or `DropOldest`; inspect `Stats::rx_dropped`. Parsed `Fxx` status is available as `Stats::last_status`.
- **Throughput:** a 64-byte extended FD frame uses 139 ASCII bytes. At 921,600 baud, serial framing alone limits it to roughly 663 frames/s before USB and CAN overhead.

## Development

Linux and macOS use the pinned Nix shell; Windows uses native Cargo. [CI](.github/workflows/ci.yml) runs formatting, Clippy, feature builds, tests, docs, Rust 1.85 compatibility and coverage.

```sh
nix develop -c cargo test --all-features
nix develop -c cargo clippy --all-features --all-targets -- -D warnings
```

Read the [specification](SPEC.md) and [hardware validation plan](HARDWARE_VALIDATION.md) for protocol vectors and release criteria.
