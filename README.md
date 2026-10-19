# slcanio

A Rust SLCAN codec and serial driver for Classic CAN and CAN FD. The CAN FD `d/D/b/B` commands are specific to normaldotcom CANable 2.0 compatible firmware; they are not part of original LAWICEL SLCAN. Choose the adapter profile explicitly.

**Status:** implementation in progress. Software tests run against simulated serial streams. Real CAN FD hardware validation is still required before claiming a verified 0.1 release.

## Development

Linux and macOS developers can enter the pinned Rust environment with `nix develop`. Windows developers use Rust's native Cargo toolchain. CI is configured for Linux, macOS and Windows. Pass a Linux port such as `/dev/ttyUSB0`, a macOS port such as `/dev/cu.usbmodem...`, or a Windows port such as `COM3` to the same builder. Test locally with:

```sh
nix develop -c cargo test --all-features
nix develop -c cargo clippy --all-features --all-targets -- -D warnings
```

## Features

- `tokio` (default): async serial API, including split sender and receiver and an injectable transport.
- `can-fd` (default): CAN FD frame model and codec.
- `blocking`: direct `serialport` API, with no Tokio runtime.

The runtime-free Classic CAN codec works with `--no-default-features`.

## Async example

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

For Classic LAWICEL devices, select `AdapterProfile::LawicelClassic`. This profile requires serial ACKs. CANable FD firmware sends none. `send()` confirms a complete serial write on no-ACK firmware; on ACK firmware it confirms adapter command acceptance. Neither confirms CAN bus acknowledgment. A canceled `send()` may still be written after it enters the bounded TX queue. Do not retry blindly if the outcome is uncertain.

`rx.recv()` is cancellation safe. RX overflow can drop the newest or oldest frame and increments `Stats::rx_dropped`. A parsed `Fxx` status record is available as `Stats::last_status`. The channel is closed during configuration, then nominal and optional data bitrates are set before it opens. FD data bitrate must be selected, or the caller must explicitly accept the firmware's documented 2 Mbit/s default with `accept_default_data_bitrate(true)`.

A 64-byte extended FD frame uses 139 ASCII bytes. At 921,600 baud, this caps serial wire serialization at roughly 663 such frames/s before overhead. USB CDC buffering and CAN bus timing may reduce throughput further.

See [SPEC.md](SPEC.md) for protocol vectors and acceptance criteria, and [HARDWARE_VALIDATION.md](HARDWARE_VALIDATION.md) for the pending physical test record.
