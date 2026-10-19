//! Strict SLCAN and CAN FD encoding with optional async and blocking serial drivers.
//! CANable FD syntax is a firmware extension, not part of the original LAWICEL protocol.
mod codec;
mod config;
mod error;
mod frame;

pub use codec::{MAX_LINE_LEN, ParsedMessage, SlcanCodec, StreamParser};
pub use config::{AckMode, AdapterProfile, Bitrate, Config, OverflowPolicy};
pub use error::{Error, ProtocolError, Result};
pub use frame::{CanFrame, CanId};

#[cfg(feature = "tokio")]
mod async_io;
#[cfg(feature = "tokio")]
pub use async_io::{CanReceiver, CanSender, CanSocket, CanSocketBuilder, ReceivedFrame, Stats};

#[cfg(feature = "blocking")]
pub mod blocking;
