#[cfg(feature = "can-fd")]
use crate::frame::fd_dlc_to_len;
use crate::{AdapterProfile, CanFrame, CanId, Error, ProtocolError, Result};

pub const MAX_LINE_LEN: usize = 256;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParsedMessage {
    Frame(CanFrame),
    Ack,
    Nack,
    Status(u8),
    UnknownLine,
}

#[derive(Debug, Clone, Copy)]
pub struct SlcanCodec {
    profile: AdapterProfile,
}
impl SlcanCodec {
    pub fn new(profile: AdapterProfile) -> Self {
        Self { profile }
    }
    pub fn profile(&self) -> AdapterProfile {
        self.profile
    }

    /// Writes a complete CR-terminated serial line. The caller provides at least 139 bytes for any frame.
    pub fn encode_frame(&self, frame: &CanFrame, dst: &mut [u8]) -> Result<usize> {
        let id = frame.id();
        if !id.valid() {
            return Err(Error::InvalidFrame);
        }
        if frame.is_fd() && !self.profile.supports_fd() {
            return Err(Error::UnsupportedFeature);
        }
        if frame.esi().is_some() {
            return Err(Error::UnsupportedFeature);
        }
        let prefix = match (
            frame.is_fd(),
            frame.is_remote(),
            id.is_extended(),
            frame.brs(),
        ) {
            (false, false, false, _) => b't',
            (false, false, true, _) => b'T',
            (false, true, false, _) => b'r',
            (false, true, true, _) => b'R',
            (true, false, false, false) => b'd',
            (true, false, true, false) => b'D',
            (true, false, false, true) => b'b',
            (true, false, true, true) => b'B',
            _ => return Err(Error::InvalidFrame),
        };
        let width = if id.is_extended() { 8 } else { 3 };
        let need = 1 + width + 1 + frame.data().len() * 2 + 1;
        if dst.len() < need {
            return Err(Error::InvalidConfiguration);
        }
        dst[0] = prefix;
        let mut n = 1;
        for shift in (0..width).rev() {
            dst[n] = to_hex(((id.value() >> (shift * 4)) & 15) as u8);
            n += 1;
        }
        dst[n] = to_hex(frame.dlc());
        n += 1;
        for &byte in frame.data() {
            dst[n] = to_hex(byte >> 4);
            dst[n + 1] = to_hex(byte & 15);
            n += 2;
        }
        dst[n] = b'\r';
        Ok(n + 1)
    }

    /// Decodes a record without its CR terminator. A bare empty record is an adapter ACK.
    pub fn decode_line(&self, line: &[u8]) -> Result<ParsedMessage> {
        if line.len() > MAX_LINE_LEN {
            return Err(ProtocolError::LineTooLong.into());
        }
        if line == [7] {
            return Ok(ParsedMessage::Nack);
        }
        if line.iter().any(|&b| !(b' '..=b'~').contains(&b)) {
            return Err(ProtocolError::InvalidAscii.into());
        }
        if line.is_empty() {
            return Ok(ParsedMessage::Ack);
        }
        let (extended, remote, fd, _brs) = match line[0] {
            b't' => (false, false, false, false),
            b'T' => (true, false, false, false),
            b'r' => (false, true, false, false),
            b'R' => (true, true, false, false),
            b'd' => (false, false, true, false),
            b'D' => (true, false, true, false),
            b'b' => (false, false, true, true),
            b'B' => (true, false, true, true),
            b'F' if line.len() == 3 => {
                let hi = from_hex(line[1]).ok_or(ProtocolError::InvalidHex)?;
                let lo = from_hex(line[2]).ok_or(ProtocolError::InvalidHex)?;
                return Ok(ParsedMessage::Status((hi << 4) | lo));
            }
            _ => return Ok(ParsedMessage::UnknownLine),
        };
        if fd && !self.profile.supports_fd() {
            return Err(Error::UnsupportedFeature);
        }
        #[cfg(not(feature = "can-fd"))]
        if fd {
            return Err(Error::UnsupportedFeature);
        }
        let width = if extended { 8 } else { 3 };
        if line.len() < 1 + width + 1 {
            return Err(ProtocolError::LengthMismatch.into());
        }
        let mut value = 0u32;
        for &c in &line[1..1 + width] {
            value = (value << 4) | u32::from(from_hex(c).ok_or(ProtocolError::InvalidHex)?);
        }
        let id = if extended {
            CanId::extended(value)
        } else {
            CanId::standard(value as u16)
        }
        .map_err(|_| ProtocolError::InvalidId)?;
        let dlc = from_hex(line[1 + width]).ok_or(ProtocolError::InvalidDlc)?;
        if !fd && dlc > 8 {
            return Err(ProtocolError::InvalidDlc.into());
        }
        let expected = if remote {
            0
        } else if fd {
            #[cfg(feature = "can-fd")]
            {
                fd_dlc_to_len(dlc).ok_or(ProtocolError::InvalidDlc)?
            }
            #[cfg(not(feature = "can-fd"))]
            {
                0
            }
        } else {
            dlc as usize
        };
        let payload = &line[2 + width..];
        if payload.len() != expected * 2 {
            return Err(ProtocolError::LengthMismatch.into());
        }
        if remote {
            return Ok(ParsedMessage::Frame(CanFrame::remote(id, dlc)?));
        }
        let mut data = [0u8; 64];
        for (i, pair) in payload.chunks_exact(2).enumerate() {
            data[i] = (from_hex(pair[0]).ok_or(ProtocolError::InvalidHex)? << 4)
                | from_hex(pair[1]).ok_or(ProtocolError::InvalidHex)?;
        }
        if fd {
            #[cfg(feature = "can-fd")]
            {
                return Ok(ParsedMessage::Frame(CanFrame::fd(
                    id,
                    &data[..expected],
                    _brs,
                )?));
            }
            #[cfg(not(feature = "can-fd"))]
            {
                unreachable!()
            }
        }
        Ok(ParsedMessage::Frame(CanFrame::classic(
            id,
            &data[..expected],
        )?))
    }
}

fn to_hex(n: u8) -> u8 {
    b"0123456789ABCDEF"[n as usize]
}
fn from_hex(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'A'..=b'F' => Some(b - b'A' + 10),
        b'a'..=b'f' => Some(b - b'a' + 10),
        _ => None,
    }
}

/// Incrementally parses fragmented serial input without heap allocation.
pub struct StreamParser {
    codec: SlcanCodec,
    line: [u8; MAX_LINE_LEN],
    len: usize,
    overlong: bool,
}
impl StreamParser {
    pub fn new(codec: SlcanCodec) -> Self {
        Self {
            codec,
            line: [0; MAX_LINE_LEN],
            len: 0,
            overlong: false,
        }
    }
    pub fn push(&mut self, bytes: &[u8], mut emit: impl FnMut(Result<ParsedMessage>)) {
        for &byte in bytes {
            match byte {
                7 => {
                    self.len = 0;
                    self.overlong = false;
                    emit(Ok(ParsedMessage::Nack));
                }
                b'\r' => {
                    if self.overlong {
                        emit(Err(ProtocolError::LineTooLong.into()));
                    } else {
                        emit(self.codec.decode_line(&self.line[..self.len]));
                    }
                    self.len = 0;
                    self.overlong = false;
                }
                _ if self.overlong => (),
                _ if self.len == MAX_LINE_LEN => {
                    self.overlong = true;
                }
                _ => {
                    self.line[self.len] = byte;
                    self.len += 1;
                }
            }
        }
    }
}
