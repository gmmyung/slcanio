//! Blocking serial API using `serialport` directly, without a runtime.
use crate::{
    AckMode, AdapterProfile, Bitrate, CanFrame, Config, Error, ParsedMessage, Result, SlcanCodec,
    StreamParser,
};
use std::{
    collections::VecDeque,
    io::{Read, Write},
    time::Duration,
};

pub struct CanSocket {
    port: Box<dyn serialport::SerialPort>,
    parser: StreamParser,
    codec: SlcanCodec,
    config: Config,
    open: bool,
    pending: VecDeque<CanFrame>,
}
pub struct CanSocketBuilder {
    port: String,
    baud: u32,
    config: Config,
    timeout: Duration,
}
impl CanSocket {
    pub fn builder(port: impl Into<String>, baud: u32) -> CanSocketBuilder {
        CanSocketBuilder {
            port: port.into(),
            baud,
            config: Config::default(),
            timeout: Duration::from_secs(2),
        }
    }
    /// For no-ACK firmware, this confirms only a serial write. For ACK firmware, it confirms adapter acceptance.
    pub fn send(&mut self, frame: CanFrame) -> Result<()> {
        if !self.open {
            return Err(Error::Disconnected);
        }
        if frame.is_fd() && !self.config.fd_enabled {
            return Err(Error::UnsupportedFeature);
        }
        if frame.brs()
            && self.config.data_bitrate.is_none()
            && !self.config.accept_default_data_bitrate
        {
            return Err(Error::InvalidConfiguration);
        }
        let mut line = [0u8; 139];
        let n = self.codec.encode_frame(&frame, &mut line)?;
        let result = self.command(&line[..n]);
        if matches!(
            result,
            Err(Error::Timeout | Error::Disconnected | Error::Io(_) | Error::CommandStateUnknown)
        ) {
            self.open = false;
        }
        result
    }
    pub fn recv(&mut self) -> Result<CanFrame> {
        if !self.open {
            return Err(Error::Disconnected);
        }
        if let Some(frame) = self.pending.pop_front() {
            return Ok(frame);
        }
        loop {
            let mut byte = [0u8; 1];
            match self.port.read(&mut byte) {
                Ok(0) => return Err(Error::Disconnected),
                Ok(_) => {
                    let mut result = None;
                    self.parser.push(&byte, |msg| match msg {
                        Ok(ParsedMessage::Frame(frame)) => result = Some(Ok(frame)),
                        Ok(_) => (),
                        Err(e) => result = Some(Err(e)),
                    });
                    if let Some(result) = result {
                        return result;
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::TimedOut => return Err(Error::Timeout),
                Err(e) => return Err(e.into()),
            }
        }
    }
    pub fn close(mut self) -> Result<()> {
        if self.open {
            self.command(b"C\r")?;
            self.open = false;
        }
        Ok(())
    }
    fn command(&mut self, bytes: &[u8]) -> Result<()> {
        self.port.write_all(bytes)?;
        if self.config.profile.ack_mode() == AckMode::None {
            return Ok(());
        }
        loop {
            let mut byte = [0u8; 1];
            match self.port.read(&mut byte) {
                Ok(0) => return Err(Error::Disconnected),
                Ok(_) => {
                    let mut result = None;
                    self.parser.push(&byte, |msg| match msg {
                        Ok(ParsedMessage::Ack) => result = Some(Ok(())),
                        Ok(ParsedMessage::Nack) => result = Some(Err(Error::AdapterRejected)),
                        Ok(ParsedMessage::Frame(frame)) => {
                            if self.pending.len() == self.config.rx_queue_capacity
                                && self.config.overflow_policy == crate::OverflowPolicy::DropOldest
                            {
                                self.pending.pop_front();
                            }
                            if self.pending.len() < self.config.rx_queue_capacity {
                                self.pending.push_back(frame);
                            }
                        }
                        _ => (),
                    });
                    if let Some(result) = result {
                        return result;
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::TimedOut => return Err(Error::Timeout),
                Err(e) => return Err(e.into()),
            }
        }
    }
}
impl CanSocketBuilder {
    pub fn adapter(mut self, profile: AdapterProfile) -> Self {
        self.config.profile = profile;
        self
    }
    pub fn nominal_bitrate(mut self, rate: Bitrate) -> Self {
        self.config.nominal_bitrate = rate;
        self
    }
    pub fn data_bitrate(mut self, rate: Bitrate) -> Self {
        self.config.data_bitrate = Some(rate);
        self
    }
    pub fn accept_default_data_bitrate(mut self, yes: bool) -> Self {
        self.config.accept_default_data_bitrate = yes;
        self
    }
    pub fn fd_enabled(mut self, yes: bool) -> Self {
        self.config.fd_enabled = yes;
        self
    }
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }
    pub fn connect(self) -> Result<CanSocket> {
        self.config.validate()?;
        if self.baud == 0 {
            return Err(Error::InvalidConfiguration);
        }
        let port = serialport::new(self.port, self.baud)
            .timeout(self.timeout)
            .open()
            .map_err(|e| Error::Io(std::io::Error::other(e)))?;
        let codec = SlcanCodec::new(self.config.profile);
        let mut socket = CanSocket {
            port,
            parser: StreamParser::new(codec),
            codec,
            config: self.config,
            open: false,
            pending: VecDeque::new(),
        };
        socket.command(b"C\r")?;
        let nominal = socket
            .config
            .profile
            .nominal_command(socket.config.nominal_bitrate)?;
        socket.command(&nominal)?;
        if let Some(data) = socket.config.data_bitrate {
            let cmd = socket.config.profile.data_command(data)?;
            socket.command(&cmd)?;
        }
        socket.command(b"O\r")?;
        socket.open = true;
        Ok(socket)
    }
}
