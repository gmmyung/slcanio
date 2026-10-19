# slcanio — Rust SLCAN / CAN FD Library

**Status:** Proposal / implementation specification  
**Revision:** 0.2 · 2026-10-19  
**Proposed package:** `slcanio`  
**Rust import:** `slcanio`  
**License:** MIT OR Apache-2.0  
**Language:** English (API names and examples)

> **Name availability:** `slcan`, `slcan_fd` and related names already exist. A web-indexed search found no matching crates.io listing for `slcanio` as of 2026-10-19, but this is **not** a registry reservation or authoritative availability check. Before publication run `cargo search slcanio --limit 10` and check `https://crates.io/api/v1/crates/slcanio` (404 means no crate record at that instant), then publish promptly. If taken, use `lawicel-io` or another newly verified name. A name might be unavailable for reasons other than an existing crate record.

## 1. Scope and non-goals

A modern cross-platform serial-line CAN implementation, with **Classic CAN and CAN FD supported in the initial release**. Provide async-first Tokio APIs, a feature-gated blocking API, and a reusable allocation-free protocol codec. Support Windows, Linux and macOS serial devices.

**V0.1 must support:**
- Classic CAN standard (11-bit) and extended (29-bit) data frames; remote frames (RTR).
- CAN FD standard and extended data frames up to 64 bytes, both BRS and non-BRS.
- Independently configurable nominal/arbitration and FD data-phase bitrates.
- Explicit adapters/dialects: `LawicelClassic` and `CanableFd` (normaldotcom CANable 2.0-compatible); an opt-in `ElmueFd` profile only after fixture and hardware validation.
- Async sender/receiver splitting, bounded queues, cancellation-safe receive, status/error handling.
- Device-independent codec, injectable test transport, and cross-platform CI.

**Non-goals for 0.1:** ISO-TP, UDS, J1939, CAN XL, DBC parsing, hard real-time delivery, kernel SocketCAN integration, automatic firmware flashing, or automatic reconnect. Future extensions may add device discovery, additional dialects and filter commands.

**Crucial:** CAN FD is **not in the original LAWICEL SLCAN standard**. Its commands and supported bitrates depend on firmware. Never turn on FD silently for an unknown adapter. UART speed and CAN wire bitrates are separate.

## 2. Design rules

1. **Async-first, transport-neutral core:** Tokio serial transport in the default feature; codec works without Tokio.
2. **Explicit feature negotiation:** choose an adapter profile; unsupported capabilities return `UnsupportedFeature` rather than guessing.
3. **Safe concurrency:** one I/O actor owns the serial stream and arbitrates CAN frames and command responses.
4. **Bounded memory:** no heap allocation in frame codec hot paths; fixed 64-byte maximum data and bounded I/O queues.
5. **Transparent correctness:** report ambiguous TX state and the distinction between serial write, adapter ACK, and actual CAN bus acknowledgment.
6. **Portability:** identical socket API across Windows, Linux, macOS.
7. **Strict parsing:** reject malformed IDs/DLC/hex/data lengths, recover after corrupt lines, never panic on arbitrary serial input.
8. **Feature isolation:** Classic CAN codecs remain available with FD disabled, but **CAN FD support is a required tested product capability** for the crate release.

## 3. Package and features

Recommended initial manifest (dependency versions are suggested ranges, subject to verification at implementation time):

```toml
[package]
name = "slcanio"
version = "0.1.0"
edition = "2024"
rust-version = "1.85"
license = "MIT OR Apache-2.0"
description = "Cross-platform async SLCAN with Classic CAN and CAN FD"
repository = "https://github.com/<owner>/slcanio"
keywords = ["can", "can-fd", "slcan", "serial", "tokio"]
categories = ["hardware-support", "asynchronous"]

[features]
default = ["tokio", "can-fd"]
tokio = ["dep:tokio", "dep:tokio-serial"]
blocking = ["dep:serialport"]
can-fd = []
embedded-can = ["dep:embedded-can"]

[dependencies]
arrayvec = "0.7"
thiserror = "2"
tokio = { version = "1", optional = true, features = ["io-util", "sync", "time", "rt"] }
tokio-serial = { version = "5", optional = true }
serialport = { version = "4", optional = true }
embedded-can = { version = "0.4", optional = true }

[dev-dependencies]
proptest = "1"
```

The core codec has no reliance on a runtime. `embedded-can` compatibility applies to Classic CAN only; FD remains a native type because `embedded-can`'s Classic frame trait cannot represent FD-specific flags and payload lengths.

## 4. Frame data model

```rust
pub enum CanId {
    Standard(u16), // validated <= 0x7FF
    Extended(u32), // validated <= 0x1FFF_FFFF
}

pub enum CanFrame {
    ClassicData { id: CanId, data: arrayvec::ArrayVec<u8, 8> },
    ClassicRemote { id: CanId, dlc: u8 }, // <= 8, no payload
    Fd {
        id: CanId,
        data: arrayvec::ArrayVec<u8, 64>,
        brs: bool,
        esi: Option<bool>, // observation-only unless dialect can encode ESI
    },
}
```

Use checked constructors (inaccessible fields or invariants checked at construction):

```rust
let classical = CanFrame::classic(CanId::standard(0x123)?, &[1, 2, 3])?;
let fd = CanFrame::fd(CanId::extended(0x18FF_50E5)?, &[0xAB; 64], true)?;
let rtr = CanFrame::remote(CanId::standard(0x123)?, 8)?;
```

- Classic data length 0–8; RTR DLC 0–8.
- FD payload legal *wire lengths*: **0–8, 12, 16, 20, 24, 32, 48, 64**; invalid lengths rejected by default. A later explicit padding policy may pad to the next representable length; never silently pad.
- FD RTR is prohibited; Classic RTR is permitted.
- ESI is not encoded by `d/D/b/B`; reject a TX frame demanding ESI encoding in that dialect, or expose ESI only as optional receive metadata. Do not invent a wire-level ESI field.
- Preserve `is_extended` independently of numeric ID magnitude (standard and extended IDs may have the same numeric value).

### 4.1 CAN FD DLC mapping

| DLC hex | Data bytes | DLC hex | Data bytes |
|---|---:|---|---:|
| 0–8 | 0–8 | 9 | 12 |
| A | 16 | B | 20 |
| C | 24 | D | 32 |
| E | 48 | F | 64 |

The codec always checks that the parsed payload size **equals** the byte count implied by DLC, and that the ID is in range.

## 5. Protocol dialects and commands

### 5.1 LAWICEL Classic profile

| ASCII command | Meaning |
|---|---|
| `S0`..`S8` | Configure nominal/arbitration bitrate (profile mapping) |
| `O` / `C` | Open/close channel |
| `L` | Listen-only open when supported |
| `t` / `T` | Classic data, standard/extended |
| `r` / `R` | Classic RTR, standard/extended |
| `Z0` / `Z1` | Optional timestamp off/on |
| `V` / `N` / `F` | Optional version/serial/status requests |

Strictly distinguish **commands** from **unsolicited frame lines**. An empty carriage-return ACK and BEL NACK are supported where the firmware sends them. Some adapters send no ACK; this is part of the adapter profile.

### 5.2 CANable FD profile (required in 0.1)

Normaldotcom CANable 2.0-compatible firmware extends the above with:

| Prefix / command | Meaning |
|---|---|
| `d` | FD standard ID, no bit-rate switch |
| `D` | FD extended ID, no bit-rate switch |
| `b` | FD standard ID, bit-rate switch enabled |
| `B` | FD extended ID, bit-rate switch enabled |
| `Y2` | FD data phase 2 Mbit/s |
| `Y5` | FD data phase 5 Mbit/s |

Its nominal bitrate commands `S0`..`S8` and selected extra commands (e.g. `S9`, `M0/M1`, `A0/A1`) are firmware-specific; represent them in the profile only after test confirmation. **This firmware does not provide command ACK/NACK**, so `AckMode::None` must be explicitly selected for the profile.

General FD wire format (ASCII uppercase hex IDs and payload):

```text
dIIIL<2*payload bytes hex>\r       # standard FD, no BRS
DIIIIIIIIL<2*payload bytes hex>\r  # extended FD, no BRS
bIIIL<2*payload bytes hex>\r       # standard FD + BRS
BIIIIIIIIL<2*payload bytes hex>\r  # extended FD + BRS
```

`III` is exactly 3 hexadecimal characters, `IIIIIIII` exactly 8, `L` exactly 1 hexadecimal DLC character (0–F). Hex *DLC* is not literal byte count for lengths above 8.

**Reference vectors:**

```text
Classic STD ID 0x123, [11 22 33]: t1233112233\r
Classic EXT ID 0x18FF50E5, [01 02]: T18FF50E520102\r
FD STD ID 0x123, BRS, 12 bytes 00..0B:
b1239000102030405060708090A0B\r
FD EXT ID 0x18FF50E5, no BRS, 16 bytes 00..0F:
D18FF50E5A000102030405060708090A0B0C0D0E0F\r
```

FD frame prefixes `d/D/b/B` are documented by CANable firmware, `python-can`, and other adapter projects, but **not standardized across all adapters**. Include round-trip tests for each prefix and every DLC.

### 5.3 Additional profiles

```rust
pub enum AdapterProfile {
    LawicelClassic,
    CanableFd,
    // Introduce only after tests prove compatible behavior:
    ElmueFd,
    Custom(AdapterCapabilities),
}
```

An Elmue CANable 2.5 firmware family uses similar FD frame prefixes but may have additional `Y` data-rate codes, lowercase custom timing commands and different status/error messages. Its timing and ACK behavior must be modeled as a **separate dialect**; do not assume that configuring 4 or 8 Mbit/s via `Y` will work on a CANable 2.0 device. Custom user profiles require explicit wire syntax and response policy rather than merely a boolean `can_fd`.

```rust
pub struct AdapterCapabilities {
    pub classic: bool,
    pub fd: bool,
    pub brs: bool,
    pub esi_rx: bool,
    pub timestamping: bool,
    pub listen_only: bool,
    pub nominal_bitrates: &'static [u32],
    pub fd_data_bitrates: &'static [u32],
    // additional dialect-level command/response configuration
}
```

## 6. Configuration and initialization

Expose separate serial, nominal CAN, and data CAN rates:

```rust
use slcanio::{AdapterProfile, Bitrate, CanSocket};

let can = CanSocket::builder("/dev/ttyUSB0", 921_600)
    .adapter(AdapterProfile::CanableFd)
    .nominal_bitrate(Bitrate::Kbps500)
    .data_bitrate(Bitrate::Mbps2)
    .fd_enabled(true)
    .rx_queue_capacity(1024)
    .tx_queue_capacity(256)
    .connect()
    .await?;
```

- For an FD adapter: set channel closed → configure nominal rate → configure FD data-phase rate → set mode → open channel.
- For Classic-only: `data_bitrate()` and `fd_enabled(true)` return `UnsupportedFeature` / `InvalidConfiguration` before touching the device.
- Data bitrate setting is optional **only** when the caller explicitly accepts the profile's documented default.
- `fd_enabled(true)` requires a profile supporting FD; FD frames with BRS require a configured FD data phase and `brs` capability.
- `connect()` is either ready-to-send or fails with cleanup; never return a half-configured socket as connected.
- Channel control operations must be serialized with frame writes; bitrate cannot be changed while open.

## 7. Public APIs

### 7.1 Basic TX/RX

```rust
use slcanio::{AdapterProfile, CanFrame, CanId, CanSocket, Bitrate};

#[tokio::main]
async fn main() -> slcanio::Result<()> {
    let can = CanSocket::builder("/dev/ttyUSB0", 921_600)
        .adapter(AdapterProfile::CanableFd)
        .nominal_bitrate(Bitrate::Kbps500)
        .data_bitrate(Bitrate::Mbps2)
        .fd_enabled(true)
        .connect()
        .await?;

    let (tx, mut rx) = can.split();
    tx.send(CanFrame::fd(CanId::standard(0x123)?, &[0x55; 12], true)?).await?;
    tx.send(CanFrame::classic(CanId::standard(0x123)?, &[1, 2, 3])?).await?;

    while let Some(frame) = rx.recv().await? {
        println!("{frame:?}");
    }
    Ok(())
}
```

**API contract:** `rx.recv().await -> Result<Option<ReceivedFrame>>`, `Some` when a frame arrives, `None` on clean channel shutdown. `ReceivedFrame` contains `frame: CanFrame`, `timestamp: Option<Timestamp>` and optional adapter metadata. No claim of accurate hardware timestamps if the adapter does not supply them.

### 7.2 Sender/receiver splitting

```rust
let (tx, mut rx) = can.split();
let tx2 = tx.clone();
tokio::spawn(async move { tx2.send(frame).await });
```

`CanSender: Clone + Send + Sync`; `CanReceiver: Send` but exclusive single-consumer receive. Frame writes are serialized by a single I/O actor; dropped senders do not halt reception. Define how last-handle shutdown terminates the actor and how `close().await` explicitly flushes or discards queued frames.

### 7.3 Blocking API

```rust
use slcanio::blocking::CanSocket;
let mut can = CanSocket::builder("COM3", 921_600)
    .adapter(AdapterProfile::CanableFd)
    .nominal_bitrate(Bitrate::Kbps500)
    .data_bitrate(Bitrate::Mbps2)
    .fd_enabled(true)
    .connect()?;
can.send(frame)?;
let received = can.recv()?;
```

Blocking uses `serialport` directly rather than embedding a hidden Tokio runtime. Codecs and validation are shared.

## 8. Codec and incremental parser

```rust
pub struct SlcanCodec { /* dialect-specific settings */ }
pub enum ParsedMessage {
    Frame(CanFrame),
    Ack,
    Nack,
    Status(Status),
    Version(Version),
    UnknownLine,
}

impl SlcanCodec {
    pub fn encode_frame(&self, frame: &CanFrame, dst: &mut [u8]) -> Result<usize>;
    pub fn decode_line(&self, line: &[u8]) -> Result<ParsedMessage>;
}
```

A streaming parser accepts arbitrarily fragmented input (`push(&[u8])`) and emits complete records on CR or BEL as appropriate. It must:
- Handle partial lines, multiple lines per read, and frame/response interleaving.
- Validate ASCII, uppercase/lowercase hexadecimal, ID width and ID bounds, DLC and exact payload length.
- Bound line length (e.g. 256 bytes for frame paths; dialect-specific larger bounds for metadata), discard overlong lines and recover at the next terminator.
- Avoid allocating on the frame encode/decode path.
- Recognize optional timestamp suffixes **only** when enabled for the selected dialect; no ambiguous parsing.
- Expose recoverable protocol errors separately from transport failures.

**Max FD extended data frame:** 1 prefix + 8 ID hex + 1 DLC hex + 128 payload hex + 1 CR = **139 ASCII bytes**. A single serial `read()` may return any partial subset of these bytes.

## 9. I/O actor, ACK and delivery semantics

```text
Application                   Async driver                     Serial adapter
  send(frame) --- bounded TX ---> | single actor | --- ASCII bytes ---> device
  recv()       <--- bounded RX --- | parser       | <--- frame/ACK/BEL --- device
  command() -- serialized cmd --> | state        |
```

- Single actor has exclusive ownership of read/write and command-response state.
- `AckMode::Required`: at most one unacknowledged command in flight; match the device's documented response grammar.
- `AckMode::None`: do not wait for command feedback; serial write success is the upper bound on confirmed progress.
- A command timeout under ACK mode must not allow a delayed ACK to be mistaken for the next command's response. Fail the session or perform a dialect-proven resynchronization.
- **Never interpret adapter ACK as CAN bus success.** In most supported ASCII firmware it is merely command acceptance. Hardware-specific TX confirmation is out of scope unless a dialect explicitly defines it.
- `send()` completion policy is documented. Default: returns once the actor has written the complete serial line; in ACK-required profiles, optional `send_confirmed()` waits for adapter feedback where supported.
- A canceled `send()` may still be transmitted once queued. Public docs must explain at-most-once is not guaranteed by retries.
- `recv()` cancellation never consumes a completed frame without returning it.
- RX queue overflow policies `DropNewest` and `DropOldest` with an observable counter; TX blocks on a full bounded queue.
- Provide a `Stats` snapshot with RX frames, RX dropped, malformed lines, TX writes, timeouts and reconnect count (zero until supported).

## 10. Errors and lifecycle

```rust
pub enum Error {
    Io(std::io::Error),
    Disconnected,
    Timeout,
    AdapterRejected,
    UnsupportedFeature,
    InvalidConfiguration,
    InvalidFrame,
    Protocol(ProtocolError),
    QueueClosed,
    CommandStateUnknown,
}
```

States: `Disconnected -> Connecting -> Configuring -> Open -> Closing -> Closed` with transition errors as appropriate. An unplug must wake waiting send/receive/control futures, fail queued work and terminate the actor. Auto-reconnect is not part of 0.1; caller may create a new connection.

## 11. Tests and acceptance criteria

**Codec:**
- Golden vectors for `t/T/r/R` and `d/D/b/B`.
- Every FD DLC `0`–`F`, every legal FD payload length, malformed and truncated lines.
- Property-based roundtrip for every supported frame kind; arbitrary input must never panic.
- Reject remote FD, nonrepresentable FD sizes, out-of-range IDs, malformed hex, unsupported FD dialects.
- Verify long-frame handling (139 ASCII bytes) and arbitrary split boundaries.

**Async correctness:**
- Simulated serial via `tokio::io::duplex` with injected read fragmentation, write backpressure, CAN RX interleaving, ACK/BEL, no-ACK, delayed ACK, disconnection, timeout, canceled `send` and `recv`.
- Assert no silent misassociation of command responses; bounded memory and overflow metrics.
- Verify FD/Classic mixed traffic and serial read/write independence.

**Hardware validation (must pass for CAN FD claim):**
- At least one CANable 2.0-compatible adapter and CAN FD peer, nominal 500 kbit/s, data 2 Mbit/s.
- FD BRS on/off, all FD DLC sizes including 64 bytes, standard/extended IDs, mixed Classic/FD traffic.
- Check that requested FD rates unsupported by a profile fail *before* sending commands.
- Verify and document actual tested adapter firmware version and serial-port speed.
- Avoid claiming successful CAN bus transmission from a serial-only ACK.

**CI:** Ubuntu/Windows/macOS; format, clippy, docs, all-features, no-default-features, async and blocking tests. Run optional HIL checks separately.

## 12. Performance notes

An extended-ID 64-byte FD SLCAN line is 139 ASCII bytes; at 8N1 UART this is about 1390 serial bits. Ignoring other overhead, a 921,600-baud line carries **at most ~663 such frames/s** and a 2,000,000-baud line carries **~1,439 frames/s**. This is only serial-wire serialization capacity, not guaranteed end-to-end throughput. Many USB CDC virtual COM implementations have buffering/jitter and may not treat the configured baud rate as an actual bit clock.

Classic extended 8-byte line length is 27 bytes, giving about 3,413 frames/s at 921,600 baud under the same simplifying model. For fast multi-motor control, document the bandwidth limitations and suggest native binary USB-CAN or SocketCAN where available.

## 13. Suggested layout

```text
slcanio/
├── Cargo.toml
├── README.md
├── SPEC.md
├── LICENSE-MIT
├── LICENSE-APACHE
├── src/
│   ├── lib.rs
│   ├── frame.rs
│   ├── error.rs
│   ├── config.rs
│   ├── codec/{mod.rs,parser.rs,encoder.rs,commands.rs}
│   ├── dialect/{mod.rs,lawicel.rs,canable_fd.rs}
│   ├── async_io/{mod.rs,socket.rs,driver.rs,sender.rs,receiver.rs}
│   └── blocking/{mod.rs,socket.rs}
├── examples/{classic.rs,fd.rs,concurrent.rs,blocking.rs}
└── tests/{codec.rs,fd_vectors.rs,async_driver.rs,cancellation.rs}
```

## 14. Delivery milestones

| Milestone | Deliverables | Blocking release? |
|---|---|---|
| M1 | Frame model, strict Classic + FD codec, golden vectors | Yes |
| M2 | Async serial driver with actor, split TX/RX, ACK/no-ACK dialect behaviors | Yes |
| M3 | Bitrate/mode configuration, required CANable FD profile, error handling | Yes |
| M4 | Cancellation, backpressure, parser fuzz/property tests | Yes |
| M5 | Real CAN FD hardware validation, examples, documentation, CI | Yes |
| M6 | Blocking adapter + `embedded-can` trait support | Optional feature; may ship with 0.1 only if complete |
| M7 | Elmue FD dialect, discovery, timestamp/vendor extensions | Future |

## 15. References

- [normaldotcom/canable2-fw — firmware and FD commands](https://github.com/normaldotcom/canable2-fw)
- [python-can SLCAN implementation](https://github.com/hardbyte/python-can/blob/main/can/interfaces/slcan.py)
- [Elmue CANable 2.5 firmware/manual](https://github.com/Elmue/CANable-2.5-firmware-Slcan-and-Candlelight)
- [WireTAP SLCAN protocol notes](https://github.com/Wired-Square/WireTAP/blob/main/docs/slcan-protocol.md)
- [Archived slcan_fd](https://github.com/adom-inc/slcan_fd)
- [embedded-can Rust trait docs](https://docs.rs/embedded-can/latest/embedded_can/)

---

**Acceptance definition:** `slcanio` v0.1 is ready when it can safely and reproducibly transmit and receive mixed Classic CAN and CAN FD (`d/D/b/B`) via a documented supported adapter on a real CAN FD bus, with correct dual-bitrate configuration, validated full-length frames, predictable async cancellation/backpressure and passing multiplatform CI.
