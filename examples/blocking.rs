use slcanio::{AdapterProfile, Bitrate, CanFrame, CanId, Result, blocking::CanSocket};

fn main() -> Result<()> {
    let port = std::env::args()
        .nth(1)
        .expect("serial port argument required");
    let mut socket = CanSocket::builder(port, 921_600)
        .adapter(AdapterProfile::CanableFd)
        .nominal_bitrate(Bitrate::Kbps500)
        .data_bitrate(Bitrate::Mbps2)
        .fd_enabled(true)
        .connect()?;
    socket.send(CanFrame::fd(CanId::standard(0x123)?, &[0x55; 12], true)?)?;
    loop {
        println!("{:?}", socket.recv()?);
    }
}
