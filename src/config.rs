use crate::{Error, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AckMode {
    Required,
    None,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdapterProfile {
    LawicelClassic,
    CanableFd,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Bitrate {
    Kbps10,
    Kbps20,
    Kbps50,
    Kbps100,
    Kbps125,
    Kbps250,
    Kbps500,
    Kbps750,
    Mbps1,
    Mbps2,
    Mbps5,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverflowPolicy {
    DropNewest,
    DropOldest,
}

impl AdapterProfile {
    pub fn ack_mode(self) -> AckMode {
        match self {
            Self::LawicelClassic => AckMode::Required,
            Self::CanableFd => AckMode::None,
        }
    }
    pub fn supports_fd(self) -> bool {
        matches!(self, Self::CanableFd)
    }
    pub(crate) fn nominal_command(self, rate: Bitrate) -> Result<[u8; 3]> {
        let n = match rate {
            Bitrate::Kbps10 => b'0',
            Bitrate::Kbps20 => b'1',
            Bitrate::Kbps50 => b'2',
            Bitrate::Kbps100 => b'3',
            Bitrate::Kbps125 => b'4',
            Bitrate::Kbps250 => b'5',
            Bitrate::Kbps500 => b'6',
            Bitrate::Kbps750 => b'7',
            Bitrate::Mbps1 => b'8',
            _ => return Err(Error::UnsupportedFeature),
        };
        Ok([b'S', n, b'\r'])
    }
    pub(crate) fn data_command(self, rate: Bitrate) -> Result<[u8; 3]> {
        if !self.supports_fd() {
            return Err(Error::UnsupportedFeature);
        }
        let n = match rate {
            Bitrate::Mbps2 => b'2',
            Bitrate::Mbps5 => b'5',
            _ => return Err(Error::UnsupportedFeature),
        };
        Ok([b'Y', n, b'\r'])
    }
}

#[derive(Debug, Clone)]
pub struct Config {
    pub profile: AdapterProfile,
    pub nominal_bitrate: Bitrate,
    pub data_bitrate: Option<Bitrate>,
    pub accept_default_data_bitrate: bool,
    pub fd_enabled: bool,
    pub rx_queue_capacity: usize,
    pub tx_queue_capacity: usize,
    pub overflow_policy: OverflowPolicy,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            profile: AdapterProfile::LawicelClassic,
            nominal_bitrate: Bitrate::Kbps500,
            data_bitrate: None,
            accept_default_data_bitrate: false,
            fd_enabled: false,
            rx_queue_capacity: 1024,
            tx_queue_capacity: 256,
            overflow_policy: OverflowPolicy::DropNewest,
        }
    }
}
impl Config {
    pub fn validate(&self) -> Result<()> {
        self.profile.nominal_command(self.nominal_bitrate)?;
        if self.rx_queue_capacity == 0 || self.tx_queue_capacity == 0 {
            return Err(Error::InvalidConfiguration);
        }
        #[cfg(not(feature = "can-fd"))]
        if self.fd_enabled {
            return Err(Error::UnsupportedFeature);
        }
        if self.fd_enabled && !self.profile.supports_fd() {
            return Err(Error::UnsupportedFeature);
        }
        if let Some(rate) = self.data_bitrate {
            self.profile.data_command(rate)?;
        }
        if self.fd_enabled && self.data_bitrate.is_none() && !self.accept_default_data_bitrate {
            return Err(Error::InvalidConfiguration);
        }
        Ok(())
    }
}
