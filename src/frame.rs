use crate::{Error, Result};
use arrayvec::ArrayVec;

/// Identifier type is preserved even when an extended ID fits in 11 bits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CanId {
    Standard(u16),
    Extended(u32),
}

impl CanId {
    pub fn standard(id: u16) -> Result<Self> {
        if id <= 0x7ff {
            Ok(Self::Standard(id))
        } else {
            Err(Error::InvalidFrame)
        }
    }
    pub fn extended(id: u32) -> Result<Self> {
        if id <= 0x1fff_ffff {
            Ok(Self::Extended(id))
        } else {
            Err(Error::InvalidFrame)
        }
    }
    pub fn value(self) -> u32 {
        match self {
            Self::Standard(v) => v.into(),
            Self::Extended(v) => v,
        }
    }
    pub fn is_extended(self) -> bool {
        matches!(self, Self::Extended(_))
    }
    pub(crate) fn valid(self) -> bool {
        match self {
            Self::Standard(v) => v <= 0x7ff,
            Self::Extended(v) => v <= 0x1fff_ffff,
        }
    }
}

/// A validated CAN frame. Variant fields are private through their contained types.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CanFrame {
    ClassicData(ClassicData),
    ClassicRemote(ClassicRemote),
    #[cfg(feature = "can-fd")]
    Fd(FdFrame),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClassicData {
    id: CanId,
    data: ArrayVec<u8, 8>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClassicRemote {
    id: CanId,
    dlc: u8,
}
#[cfg(feature = "can-fd")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FdFrame {
    id: CanId,
    data: ArrayVec<u8, 64>,
    brs: bool,
    esi: Option<bool>,
}

impl CanFrame {
    pub fn classic(id: CanId, data: &[u8]) -> Result<Self> {
        if !id.valid() || data.len() > 8 {
            return Err(Error::InvalidFrame);
        }
        Ok(Self::ClassicData(ClassicData {
            id,
            data: ArrayVec::try_from(data).map_err(|_| Error::InvalidFrame)?,
        }))
    }
    pub fn remote(id: CanId, dlc: u8) -> Result<Self> {
        if !id.valid() || dlc > 8 {
            return Err(Error::InvalidFrame);
        }
        Ok(Self::ClassicRemote(ClassicRemote { id, dlc }))
    }
    #[cfg(feature = "can-fd")]
    pub fn fd(id: CanId, data: &[u8], brs: bool) -> Result<Self> {
        if !id.valid() || fd_len_to_dlc(data.len()).is_none() {
            return Err(Error::InvalidFrame);
        }
        Ok(Self::Fd(FdFrame {
            id,
            data: ArrayVec::try_from(data).map_err(|_| Error::InvalidFrame)?,
            brs,
            esi: None,
        }))
    }
    pub fn id(&self) -> CanId {
        match self {
            Self::ClassicData(v) => v.id,
            Self::ClassicRemote(v) => v.id,
            #[cfg(feature = "can-fd")]
            Self::Fd(v) => v.id,
        }
    }
    pub fn data(&self) -> &[u8] {
        match self {
            Self::ClassicData(v) => &v.data,
            Self::ClassicRemote(_) => &[],
            #[cfg(feature = "can-fd")]
            Self::Fd(v) => &v.data,
        }
    }
    pub fn dlc(&self) -> u8 {
        match self {
            Self::ClassicData(v) => v.data.len() as u8,
            Self::ClassicRemote(v) => v.dlc,
            #[cfg(feature = "can-fd")]
            Self::Fd(v) => fd_len_to_dlc(v.data.len()).expect("validated FD length"),
        }
    }
    pub fn is_remote(&self) -> bool {
        matches!(self, Self::ClassicRemote(_))
    }
    pub fn is_fd(&self) -> bool {
        #[cfg(feature = "can-fd")]
        {
            matches!(self, Self::Fd(_))
        }
        #[cfg(not(feature = "can-fd"))]
        {
            false
        }
    }
    pub fn brs(&self) -> bool {
        #[cfg(feature = "can-fd")]
        {
            match self {
                Self::Fd(v) => v.brs,
                _ => false,
            }
        }
        #[cfg(not(feature = "can-fd"))]
        {
            false
        }
    }
    pub fn esi(&self) -> Option<bool> {
        #[cfg(feature = "can-fd")]
        {
            match self {
                Self::Fd(v) => v.esi,
                _ => None,
            }
        }
        #[cfg(not(feature = "can-fd"))]
        {
            None
        }
    }
}

#[cfg(feature = "can-fd")]
pub(crate) fn fd_len_to_dlc(len: usize) -> Option<u8> {
    match len {
        0..=8 => Some(len as u8),
        12 => Some(9),
        16 => Some(10),
        20 => Some(11),
        24 => Some(12),
        32 => Some(13),
        48 => Some(14),
        64 => Some(15),
        _ => None,
    }
}

#[cfg(feature = "can-fd")]
pub(crate) fn fd_dlc_to_len(dlc: u8) -> Option<usize> {
    const SIZES: [usize; 16] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 12, 16, 20, 24, 32, 48, 64];
    SIZES.get(dlc as usize).copied()
}
