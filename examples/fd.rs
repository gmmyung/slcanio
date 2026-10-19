use slcanio::{AdapterProfile, Bitrate, CanFrame, CanId, CanSocket, Result};

#[tokio::main]
async fn main() -> Result<()> {
    let port = std::env::args()
        .nth(1)
        .expect("serial port argument required");
    let socket = CanSocket::builder(port, 921_600)
        .adapter(AdapterProfile::CanableFd)
        .nominal_bitrate(Bitrate::Kbps500)
        .data_bitrate(Bitrate::Mbps2)
        .fd_enabled(true)
        .connect()
        .await?;
    let (tx, mut rx) = socket.split();
    tx.send(CanFrame::fd(
        CanId::extended(0x18ff50e5)?,
        &[0x55; 64],
        true,
    )?)
    .await?;
    while let Some(received) = rx.recv().await? {
        println!("{:?}", received.frame);
    }
    Ok(())
}
