#![cfg(all(feature = "tokio", feature = "can-fd"))]
use slcanio::{AdapterProfile, Bitrate, CanFrame, CanId, CanSocket, Error, OverflowPolicy};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

#[tokio::test]
async fn canable_setup_send_receive_and_cancel() {
    let (host, peer) = tokio::io::duplex(4096);
    let device = tokio::spawn(async move {
        let mut peer = BufReader::new(peer);
        for expected in [b"C\r".as_slice(), b"S6\r", b"Y2\r", b"O\r"] {
            let mut line = Vec::new();
            peer.read_until(b'\r', &mut line).await.unwrap();
            assert_eq!(line, expected);
        }
        let mut line = Vec::new();
        peer.read_until(b'\r', &mut line).await.unwrap();
        assert_eq!(&line, b"b1239000102030405060708090A0B\r");
        peer.get_mut().write_all(b"t123101\r").await.unwrap();
        tokio::time::sleep(Duration::from_millis(30)).await;
    });
    let can = CanSocket::builder("injected", 921_600)
        .adapter(AdapterProfile::CanableFd)
        .nominal_bitrate(Bitrate::Kbps500)
        .data_bitrate(Bitrate::Mbps2)
        .fd_enabled(true)
        .connect_with_transport(host)
        .await
        .unwrap();
    let (tx, mut rx) = can.split();
    assert!(
        tokio::time::timeout(Duration::from_millis(5), rx.recv())
            .await
            .is_err()
    );
    let payload: Vec<u8> = (0..12).collect();
    tx.send(CanFrame::fd(CanId::standard(0x123).unwrap(), &payload, true).unwrap())
        .await
        .unwrap();
    let got = tokio::time::timeout(Duration::from_secs(1), rx.recv())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(got.frame.data(), &[1]);
    assert_eq!(got.timestamp, None);
    device.await.unwrap();
}

#[tokio::test]
async fn lawicel_ack_interleaving_and_rejection() {
    let (host, peer) = tokio::io::duplex(1024);
    let device = tokio::spawn(async move {
        let mut peer = BufReader::new(peer);
        for expected in [b"C\r".as_slice(), b"S6\r", b"O\r"] {
            let mut line = Vec::new();
            peer.read_until(b'\r', &mut line).await.unwrap();
            assert_eq!(line, expected);
            peer.get_mut().write_all(b"\r").await.unwrap();
        }
        let mut line = Vec::new();
        peer.read_until(b'\r', &mut line).await.unwrap();
        assert_eq!(&line, b"t1230\r");
        peer.get_mut().write_all(b"t3211AA\r\x07").await.unwrap();
        tokio::time::sleep(Duration::from_millis(30)).await;
    });
    let can = CanSocket::builder("injected", 115_200)
        .connect_with_transport(host)
        .await
        .unwrap();
    let (tx, mut rx) = can.split();
    let result = tx
        .send(CanFrame::classic(CanId::standard(0x123).unwrap(), &[]).unwrap())
        .await;
    assert!(matches!(result, Err(Error::AdapterRejected)));
    let frame = rx.recv().await.unwrap().unwrap().frame;
    assert_eq!(frame.id(), CanId::standard(0x321).unwrap());
    assert_eq!(frame.data(), &[0xaa]);
    device.await.unwrap();
}

#[tokio::test]
async fn timeout_during_configuration_fails_session() {
    let (host, peer) = tokio::io::duplex(64);
    let device = tokio::spawn(async move {
        let mut peer = BufReader::new(peer);
        let mut line = Vec::new();
        peer.read_until(b'\r', &mut line).await.unwrap();
        assert_eq!(&line, b"C\r");
        peer.get_mut().write_all(b"\r").await.unwrap();
        line.clear();
        peer.read_until(b'\r', &mut line).await.unwrap();
        assert_eq!(&line, b"S6\r");
        tokio::time::sleep(Duration::from_millis(100)).await;
    });
    let result = CanSocket::builder("injected", 115_200)
        .command_timeout(Duration::from_millis(20))
        .connect_with_transport(host)
        .await;
    assert!(matches!(result, Err(Error::Timeout)));
    device.await.unwrap();
}

#[tokio::test]
async fn bounded_rx_drop_oldest_is_observable() {
    let (host, peer) = tokio::io::duplex(1024);
    let device = tokio::spawn(async move {
        let mut peer = BufReader::new(peer);
        for _ in 0..3 {
            let mut line = Vec::new();
            peer.read_until(b'\r', &mut line).await.unwrap();
        }
        peer.get_mut()
            .write_all(b"t123101\rt123102\rt123103\rF80\r")
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;
    });
    let mut can = CanSocket::builder("injected", 115_200)
        .adapter(AdapterProfile::CanableFd)
        .rx_queue_capacity(1)
        .overflow_policy(OverflowPolicy::DropOldest)
        .connect_with_transport(host)
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(1), async {
        while can.stats().rx_frames < 3 || can.stats().last_status != Some(0x80) {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(can.stats().rx_dropped, 2);
    assert_eq!(can.stats().last_status, Some(0x80));
    let frame = can.recv().await.unwrap().unwrap().frame;
    assert_eq!(frame.data(), &[3]);
    device.await.unwrap();
}

#[tokio::test]
async fn incoming_frame_is_read_while_serial_write_is_backpressured() {
    use tokio::{io::AsyncReadExt, sync::oneshot};
    let (host, peer) = tokio::io::duplex(32);
    let (allow_drain_tx, allow_drain_rx) = oneshot::channel();
    let device = tokio::spawn(async move {
        let mut peer = BufReader::new(peer);
        for _ in 0..4 {
            let mut line = Vec::new();
            peer.read_until(b'\r', &mut line).await.unwrap();
        }
        let mut prefix = [0u8; 16];
        peer.read_exact(&mut prefix).await.unwrap();
        assert_eq!(prefix[0], b'B');
        peer.get_mut().write_all(b"t123101\r").await.unwrap();
        allow_drain_rx.await.unwrap();
        let mut rest = Vec::new();
        peer.read_until(b'\r', &mut rest).await.unwrap();
        assert_eq!(prefix.len() + rest.len(), 139);
    });
    let can = CanSocket::builder("injected", 921_600)
        .adapter(AdapterProfile::CanableFd)
        .data_bitrate(Bitrate::Mbps2)
        .fd_enabled(true)
        .connect_with_transport(host)
        .await
        .unwrap();
    let (tx, mut rx) = can.split();
    let send = tokio::spawn(async move {
        tx.send(CanFrame::fd(CanId::extended(0x18ff50e5).unwrap(), &[0xaa; 64], true).unwrap())
            .await
    });
    let received = tokio::time::timeout(Duration::from_secs(1), rx.recv())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(received.frame.data(), &[1]);
    allow_drain_tx.send(()).unwrap();
    send.await.unwrap().unwrap();
    device.await.unwrap();
}

#[tokio::test]
async fn unsupported_fd_configuration_sends_no_commands() {
    use tokio::io::AsyncReadExt;
    let (host, mut peer) = tokio::io::duplex(64);
    let result = CanSocket::builder("injected", 115_200)
        .adapter(AdapterProfile::LawicelClassic)
        .fd_enabled(true)
        .data_bitrate(Bitrate::Mbps2)
        .connect_with_transport(host)
        .await;
    assert!(matches!(result, Err(Error::UnsupportedFeature)));
    let mut byte = [0u8; 1];
    assert_eq!(peer.read(&mut byte).await.unwrap(), 0);
}

#[tokio::test]
async fn explicit_close_ends_receiver_cleanly() {
    let (host, peer) = tokio::io::duplex(64);
    let device = tokio::spawn(async move {
        let mut peer = BufReader::new(peer);
        for expected in [b"C\r".as_slice(), b"S6\r", b"O\r", b"C\r"] {
            let mut line = Vec::new();
            peer.read_until(b'\r', &mut line).await.unwrap();
            assert_eq!(line, expected);
        }
    });
    let can = CanSocket::builder("injected", 115_200)
        .adapter(AdapterProfile::CanableFd)
        .connect_with_transport(host)
        .await
        .unwrap();
    let (tx, mut rx) = can.split();
    tx.close().await.unwrap();
    assert!(rx.recv().await.unwrap().is_none());
    device.await.unwrap();
}

#[tokio::test]
async fn ack_timeout_terminates_session_before_late_ack() {
    let (host, peer) = tokio::io::duplex(128);
    let device = tokio::spawn(async move {
        let mut peer = BufReader::new(peer);
        for _ in 0..3 {
            let mut line = Vec::new();
            peer.read_until(b'\r', &mut line).await.unwrap();
            peer.get_mut().write_all(b"\r").await.unwrap();
        }
        let mut line = Vec::new();
        peer.read_until(b'\r', &mut line).await.unwrap();
        assert_eq!(&line, b"t1230\r");
        tokio::time::sleep(Duration::from_millis(40)).await;
        let _ = peer.get_mut().write_all(b"\r").await;
    });
    let can = CanSocket::builder("injected", 115_200)
        .command_timeout(Duration::from_millis(15))
        .connect_with_transport(host)
        .await
        .unwrap();
    let (tx, mut rx) = can.split();
    let frame = CanFrame::classic(CanId::standard(0x123).unwrap(), &[]).unwrap();
    assert!(matches!(tx.send(frame.clone()).await, Err(Error::Timeout)));
    assert!(tx.send(frame).await.is_err());
    assert!(matches!(rx.recv().await, Err(Error::Disconnected)));
    device.await.unwrap();
}
