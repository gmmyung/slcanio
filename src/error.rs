use thiserror::Error;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum ProtocolError {
    #[error("invalid ASCII line")]
    InvalidAscii,
    #[error("invalid frame prefix")]
    InvalidPrefix,
    #[error("invalid CAN identifier")]
    InvalidId,
    #[error("invalid DLC")]
    InvalidDlc,
    #[error("invalid hexadecimal data")]
    InvalidHex,
    #[error("payload length does not match DLC")]
    LengthMismatch,
    #[error("line exceeds parser limit")]
    LineTooLong,
}

#[derive(Debug, Error)]
pub enum Error {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("device disconnected")]
    Disconnected,
    #[error("adapter response timed out")]
    Timeout,
    #[error("adapter rejected command")]
    AdapterRejected,
    #[error("adapter does not support this feature")]
    UnsupportedFeature,
    #[error("invalid configuration")]
    InvalidConfiguration,
    #[error("invalid CAN frame")]
    InvalidFrame,
    #[error(transparent)]
    Protocol(#[from] ProtocolError),
    #[error("I/O queue closed")]
    QueueClosed,
    #[error("command outcome is unknown")]
    CommandStateUnknown,
}
