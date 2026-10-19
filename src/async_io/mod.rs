use crate::{
    AckMode, AdapterProfile, Bitrate, CanFrame, Config, Error, OverflowPolicy, ParsedMessage,
    Result, SlcanCodec, StreamParser,
};
use std::{
    collections::VecDeque,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU16, AtomicU64, Ordering},
    },
    time::Duration,
};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    sync::{Notify, mpsc, oneshot},
};
use tokio_serial::SerialPortBuilderExt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReceivedFrame {
    pub frame: CanFrame,
    pub timestamp: Option<u32>,
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Stats {
    pub rx_frames: u64,
    pub rx_dropped: u64,
    pub malformed_lines: u64,
    pub tx_writes: u64,
    pub timeouts: u64,
    pub reconnect_count: u64,
    pub last_status: Option<u8>,
}
#[derive(Default)]
struct Counters {
    rx_frames: AtomicU64,
    rx_dropped: AtomicU64,
    malformed_lines: AtomicU64,
    tx_writes: AtomicU64,
    timeouts: AtomicU64,
    last_status: AtomicU16,
}
impl Counters {
    fn snapshot(&self) -> Stats {
        Stats {
            rx_frames: self.rx_frames.load(Ordering::Relaxed),
            rx_dropped: self.rx_dropped.load(Ordering::Relaxed),
            malformed_lines: self.malformed_lines.load(Ordering::Relaxed),
            tx_writes: self.tx_writes.load(Ordering::Relaxed),
            timeouts: self.timeouts.load(Ordering::Relaxed),
            reconnect_count: 0,
            last_status: match self.last_status.load(Ordering::Relaxed) {
                0 => None,
                n => Some((n - 1) as u8),
            },
        }
    }
}
struct RxState {
    frames: VecDeque<ReceivedFrame>,
    capacity: usize,
    policy: OverflowPolicy,
    closed: bool,
    failed: bool,
}
struct RxQueue {
    state: Mutex<RxState>,
    notify: Notify,
    actor_notify: Notify,
    counters: Arc<Counters>,
}
impl RxQueue {
    fn new(capacity: usize, policy: OverflowPolicy, counters: Arc<Counters>) -> Self {
        Self {
            state: Mutex::new(RxState {
                frames: VecDeque::with_capacity(capacity),
                capacity,
                policy,
                closed: false,
                failed: false,
            }),
            notify: Notify::new(),
            actor_notify: Notify::new(),
            counters,
        }
    }
    fn push(&self, frame: CanFrame) {
        self.counters.rx_frames.fetch_add(1, Ordering::Relaxed);
        let mut state = self.state.lock().expect("RX lock poisoned");
        if state.closed {
            return;
        }
        if state.frames.len() == state.capacity {
            self.counters.rx_dropped.fetch_add(1, Ordering::Relaxed);
            match state.policy {
                OverflowPolicy::DropNewest => return,
                OverflowPolicy::DropOldest => {
                    state.frames.pop_front();
                }
            }
        }
        state.frames.push_back(ReceivedFrame {
            frame,
            timestamp: None,
        });
        drop(state);
        self.notify.notify_one();
    }
    fn finish(&self, failed: bool) {
        let mut state = self.state.lock().expect("RX lock poisoned");
        state.closed = true;
        state.failed = failed;
        drop(state);
        self.notify.notify_waiters();
    }
    async fn pop(&self) -> Result<Option<ReceivedFrame>> {
        loop {
            let notified = self.notify.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            {
                let mut state = self.state.lock().expect("RX lock poisoned");
                if let Some(frame) = state.frames.pop_front() {
                    return Ok(Some(frame));
                }
                if state.closed {
                    return if state.failed {
                        Err(Error::Disconnected)
                    } else {
                        Ok(None)
                    };
                }
            }
            notified.await;
        }
    }
}

enum Command {
    Send(CanFrame, oneshot::Sender<Result<()>>),
    Close(oneshot::Sender<Result<()>>),
}
#[derive(Clone)]
pub struct CanSender {
    commands: mpsc::Sender<Command>,
    config: Arc<Config>,
    counters: Arc<Counters>,
}
impl CanSender {
    /// In no-ACK profiles, success means the full serial line was written. A canceled send may still be written.
    /// In ACK-required profiles, success means adapter command acceptance, not CAN bus acknowledgment.
    pub async fn send(&self, frame: CanFrame) -> Result<()> {
        if frame.is_fd() && !self.config.fd_enabled {
            return Err(Error::UnsupportedFeature);
        }
        if frame.brs()
            && self.config.data_bitrate.is_none()
            && !self.config.accept_default_data_bitrate
        {
            return Err(Error::InvalidConfiguration);
        }
        let (reply_tx, reply_rx) = oneshot::channel();
        self.commands
            .send(Command::Send(frame, reply_tx))
            .await
            .map_err(|_| Error::QueueClosed)?;
        reply_rx.await.map_err(|_| Error::Disconnected)?
    }
    pub fn stats(&self) -> Stats {
        self.counters.snapshot()
    }
    pub async fn close(&self) -> Result<()> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.commands
            .send(Command::Close(reply_tx))
            .await
            .map_err(|_| Error::QueueClosed)?;
        reply_rx.await.map_err(|_| Error::Disconnected)?
    }
}
pub struct CanReceiver {
    queue: Arc<RxQueue>,
}
impl CanReceiver {
    /// Cancellation safe: a frame is removed only when this call returns it.
    pub async fn recv(&mut self) -> Result<Option<ReceivedFrame>> {
        self.queue.pop().await
    }
    pub fn stats(&self) -> Stats {
        self.queue.counters.snapshot()
    }
}
impl Drop for CanReceiver {
    fn drop(&mut self) {
        self.queue.finish(false);
        self.queue.actor_notify.notify_one();
    }
}
pub struct CanSocket {
    sender: CanSender,
    receiver: CanReceiver,
}
impl CanSocket {
    pub fn builder(port: impl Into<String>, baud: u32) -> CanSocketBuilder {
        CanSocketBuilder {
            port: port.into(),
            baud,
            config: Config::default(),
            command_timeout: Duration::from_secs(2),
        }
    }
    pub fn split(self) -> (CanSender, CanReceiver) {
        (self.sender, self.receiver)
    }
    pub async fn send(&self, frame: CanFrame) -> Result<()> {
        self.sender.send(frame).await
    }
    pub async fn recv(&mut self) -> Result<Option<ReceivedFrame>> {
        self.receiver.recv().await
    }
    pub fn stats(&self) -> Stats {
        self.sender.stats()
    }
    pub async fn close(self) -> Result<()> {
        self.sender.close().await
    }
}
pub struct CanSocketBuilder {
    port: String,
    baud: u32,
    config: Config,
    command_timeout: Duration,
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
    pub fn rx_queue_capacity(mut self, capacity: usize) -> Self {
        self.config.rx_queue_capacity = capacity;
        self
    }
    pub fn tx_queue_capacity(mut self, capacity: usize) -> Self {
        self.config.tx_queue_capacity = capacity;
        self
    }
    pub fn overflow_policy(mut self, policy: OverflowPolicy) -> Self {
        self.config.overflow_policy = policy;
        self
    }
    pub fn command_timeout(mut self, timeout: Duration) -> Self {
        self.command_timeout = timeout;
        self
    }
    pub async fn connect(self) -> Result<CanSocket> {
        self.config.validate()?;
        if self.baud == 0 {
            return Err(Error::InvalidConfiguration);
        }
        let port = tokio_serial::new(&self.port, self.baud)
            .open_native_async()
            .map_err(|e| Error::Io(std::io::Error::other(e)))?;
        self.connect_with_transport(port).await
    }
    /// Injects an async byte stream, suitable for simulated adapters and tests.
    pub async fn connect_with_transport<T>(self, mut transport: T) -> Result<CanSocket>
    where
        T: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    {
        self.config.validate()?;
        let counters = Arc::new(Counters::default());
        let queue = Arc::new(RxQueue::new(
            self.config.rx_queue_capacity,
            self.config.overflow_policy,
            counters.clone(),
        ));
        let codec = SlcanCodec::new(self.config.profile);
        let mut parser = StreamParser::new(codec);
        if let Err(error) = configure(
            &mut transport,
            &mut parser,
            &self.config,
            self.command_timeout,
            &queue,
            &counters,
        )
        .await
        {
            let _ = tokio::time::timeout(self.command_timeout, transport.write_all(b"C\r")).await;
            return Err(error);
        }
        let (commands, command_rx) = mpsc::channel(self.config.tx_queue_capacity);
        let sender = CanSender {
            commands,
            config: Arc::new(self.config),
            counters: counters.clone(),
        };
        let receiver = CanReceiver {
            queue: queue.clone(),
        };
        tokio::spawn(run_actor(
            transport,
            ActorContext {
                parser,
                codec,
                commands: command_rx,
                queue,
                counters,
                ack: sender.config.profile.ack_mode(),
                timeout: self.command_timeout,
            },
        ));
        Ok(CanSocket { sender, receiver })
    }
}

async fn configure<T: AsyncRead + AsyncWrite + Unpin>(
    stream: &mut T,
    parser: &mut StreamParser,
    config: &Config,
    timeout: Duration,
    queue: &RxQueue,
    counters: &Counters,
) -> Result<()> {
    let ack = config.profile.ack_mode();
    command(stream, parser, b"C\r", ack, timeout, queue, counters).await?;
    let nominal = config.profile.nominal_command(config.nominal_bitrate)?;
    command(stream, parser, &nominal, ack, timeout, queue, counters).await?;
    if let Some(data) = config.data_bitrate {
        let cmd = config.profile.data_command(data)?;
        command(stream, parser, &cmd, ack, timeout, queue, counters).await?;
    }
    command(stream, parser, b"O\r", ack, timeout, queue, counters).await
}
async fn command<T: AsyncRead + AsyncWrite + Unpin>(
    stream: &mut T,
    parser: &mut StreamParser,
    bytes: &[u8],
    ack: AckMode,
    timeout: Duration,
    queue: &RxQueue,
    counters: &Counters,
) -> Result<()> {
    match tokio::time::timeout(timeout, stream.write_all(bytes)).await {
        Ok(result) => result?,
        Err(_) => {
            counters.timeouts.fetch_add(1, Ordering::Relaxed);
            return Err(Error::CommandStateUnknown);
        }
    }
    counters.tx_writes.fetch_add(1, Ordering::Relaxed);
    if ack == AckMode::None {
        return Ok(());
    }
    let wait = async {
        loop {
            let mut byte = [0u8; 1];
            if stream.read(&mut byte).await? == 0 {
                return Err(Error::Disconnected);
            }
            let mut response = None;
            parser.push(&byte, |msg| match msg {
                Ok(ParsedMessage::Frame(frame)) => queue.push(frame),
                Ok(ParsedMessage::Status(status)) => {
                    counters
                        .last_status
                        .store(u16::from(status) + 1, Ordering::Relaxed);
                }
                Ok(ParsedMessage::Ack) => response = Some(Ok(())),
                Ok(ParsedMessage::Nack) => response = Some(Err(Error::AdapterRejected)),
                Ok(_) => (),
                Err(_) => {
                    counters.malformed_lines.fetch_add(1, Ordering::Relaxed);
                }
            });
            if let Some(result) = response {
                return result;
            }
        }
    };
    match tokio::time::timeout(timeout, wait).await {
        Ok(result) => result,
        Err(_) => {
            counters.timeouts.fetch_add(1, Ordering::Relaxed);
            Err(Error::Timeout)
        }
    }
}
struct ActorContext {
    parser: StreamParser,
    codec: SlcanCodec,
    commands: mpsc::Receiver<Command>,
    queue: Arc<RxQueue>,
    counters: Arc<Counters>,
    ack: AckMode,
    timeout: Duration,
}
struct CommandContext<'a> {
    queue: &'a RxQueue,
    counters: &'a Counters,
    ack: AckMode,
    timeout: Duration,
}
async fn actor_command<R: AsyncRead + Unpin, W: AsyncWrite + Unpin>(
    reader: &mut R,
    writer: &mut W,
    parser: &mut StreamParser,
    bytes: &[u8],
    context: CommandContext<'_>,
) -> Result<()> {
    let mut write_done = false;
    let operation = async {
        let mut response = None;
        {
            let write = writer.write_all(bytes);
            tokio::pin!(write);
            loop {
                let mut byte = [0u8; 1];
                tokio::select! {
                    result = &mut write => { result?; write_done = true; break; }
                    read = reader.read(&mut byte) => {
                        if read? == 0 { return Err(Error::Disconnected); }
                        parser.push(&byte, |msg| match msg {
                            Ok(ParsedMessage::Frame(frame)) => context.queue.push(frame),
                            Ok(ParsedMessage::Status(status)) => { context.counters.last_status.store(u16::from(status) + 1, Ordering::Relaxed); },
                            Ok(ParsedMessage::Ack) if context.ack == AckMode::Required => response = Some(Ok(())),
                            Ok(ParsedMessage::Nack) if context.ack == AckMode::Required => response = Some(Err(Error::AdapterRejected)),
                            Ok(_) => (),
                            Err(_) => { context.counters.malformed_lines.fetch_add(1, Ordering::Relaxed); },
                        });
                    }
                }
            }
        }
        context.counters.tx_writes.fetch_add(1, Ordering::Relaxed);
        if context.ack == AckMode::None {
            return Ok(());
        }
        if let Some(response) = response {
            return response;
        }
        loop {
            let mut byte = [0u8; 1];
            if reader.read(&mut byte).await? == 0 {
                return Err(Error::Disconnected);
            }
            parser.push(&byte, |msg| match msg {
                Ok(ParsedMessage::Frame(frame)) => context.queue.push(frame),
                Ok(ParsedMessage::Status(status)) => {
                    context
                        .counters
                        .last_status
                        .store(u16::from(status) + 1, Ordering::Relaxed);
                }
                Ok(ParsedMessage::Ack) => response = Some(Ok(())),
                Ok(ParsedMessage::Nack) => response = Some(Err(Error::AdapterRejected)),
                Ok(_) => (),
                Err(_) => {
                    context
                        .counters
                        .malformed_lines
                        .fetch_add(1, Ordering::Relaxed);
                }
            });
            if let Some(response) = response {
                return response;
            }
        }
    };
    match tokio::time::timeout(context.timeout, operation).await {
        Ok(result) => result,
        Err(_) => {
            context.counters.timeouts.fetch_add(1, Ordering::Relaxed);
            if write_done {
                Err(Error::Timeout)
            } else {
                Err(Error::CommandStateUnknown)
            }
        }
    }
}
async fn run_actor<T: AsyncRead + AsyncWrite + Unpin>(stream: T, context: ActorContext) {
    let (mut reader, mut writer) = tokio::io::split(stream);
    let ActorContext {
        mut parser,
        codec,
        mut commands,
        queue,
        counters,
        ack,
        timeout,
    } = context;
    let mut tx_closed = false;
    let mut clean = false;
    loop {
        if tx_closed && queue.state.lock().expect("RX lock poisoned").closed {
            clean = true;
            break;
        }
        let mut buf = [0u8; 256];
        tokio::select! {
            cmd = commands.recv(), if !tx_closed => match cmd {
                Some(Command::Send(frame, reply)) => {
                    let mut line = [0u8; 139];
                    let result = match codec.encode_frame(&frame, &mut line) {
                        Ok(n) => actor_command(&mut reader, &mut writer, &mut parser, &line[..n], CommandContext { queue: &queue, counters: &counters, ack, timeout }).await,
                        Err(e) => Err(e),
                    };
                    let fatal = matches!(result, Err(Error::Io(_) | Error::Disconnected | Error::Timeout | Error::CommandStateUnknown));
                    let _ = reply.send(result);
                    if fatal { break; }
                }
                Some(Command::Close(reply)) => {
                    let result = actor_command(&mut reader, &mut writer, &mut parser, b"C\r", CommandContext { queue: &queue, counters: &counters, ack, timeout }).await;
                    clean = result.is_ok();
                    let _ = reply.send(result);
                    break;
                }
                None => tx_closed = true,
            },
            read = reader.read(&mut buf) => match read {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    let mut unexpected_ack = false;
                    parser.push(&buf[..n], |msg| match msg {
                        Ok(ParsedMessage::Frame(frame)) => queue.push(frame),
                        Ok(ParsedMessage::Status(status)) => { counters.last_status.store(u16::from(status) + 1, Ordering::Relaxed); },
                        Ok(ParsedMessage::Ack | ParsedMessage::Nack) if ack == AckMode::Required => unexpected_ack = true,
                        Ok(_) => (),
                        Err(_) => { counters.malformed_lines.fetch_add(1, Ordering::Relaxed); },
                    });
                    if unexpected_ack { break; }
                }
            },
            _ = queue.actor_notify.notified() => {},
        }
    }
    queue.finish(!clean);
}
