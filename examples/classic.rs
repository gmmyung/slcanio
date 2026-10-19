use slcanio::{CanFrame, CanId, CanSocket, Result};

#[tokio::main]
async fn main() -> Result<()> {
    let port = std::env::args()
        .nth(1)
        .expect("serial port argument required");
    let socket = CanSocket::builder(port, 115_200).connect().await?;
    let (tx, mut rx) = socket.split();
    tx.send(CanFrame::classic(CanId::standard(0x123)?, &[1, 2, 3])?)
        .await?;
    while let Some(received) = rx.recv().await? {
        println!("{:?}", received.frame);
    }
    Ok(())
}
