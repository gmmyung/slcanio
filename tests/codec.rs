use proptest::prelude::*;
use slcanio::{
    AdapterProfile, CanFrame, CanId, Error, ParsedMessage, ProtocolError, SlcanCodec, StreamParser,
};

fn roundtrip(codec: SlcanCodec, frame: CanFrame) {
    let mut buf = [0u8; 139];
    let n = codec.encode_frame(&frame, &mut buf).unwrap();
    assert_eq!(buf[n - 1], b'\r');
    assert_eq!(
        codec.decode_line(&buf[..n - 1]).unwrap(),
        ParsedMessage::Frame(frame)
    );
}
#[test]
fn classic_vectors() {
    let codec = SlcanCodec::new(AdapterProfile::LawicelClassic);
    let cases = [
        (
            CanFrame::classic(CanId::standard(0x123).unwrap(), &[0x11, 0x22, 0x33]).unwrap(),
            "t1233112233",
        ),
        (
            CanFrame::classic(CanId::extended(0x18ff50e5).unwrap(), &[1, 2]).unwrap(),
            "T18FF50E520102",
        ),
        (
            CanFrame::remote(CanId::standard(0x123).unwrap(), 8).unwrap(),
            "r1238",
        ),
        (
            CanFrame::remote(CanId::extended(0x18ff50e5).unwrap(), 0).unwrap(),
            "R18FF50E50",
        ),
    ];
    for (frame, expected) in cases {
        let mut buf = [0u8; 139];
        let n = codec.encode_frame(&frame, &mut buf).unwrap();
        assert_eq!(&buf[..n - 1], expected.as_bytes());
        roundtrip(codec, frame);
    }
}
#[cfg(feature = "can-fd")]
#[test]
fn fd_all_dlc_and_prefixes() {
    let codec = SlcanCodec::new(AdapterProfile::CanableFd);
    let lengths = [0, 1, 2, 3, 4, 5, 6, 7, 8, 12, 16, 20, 24, 32, 48, 64];
    for (dlc, len) in lengths.into_iter().enumerate() {
        for id in [
            CanId::standard(0x123).unwrap(),
            CanId::extended(0x18ff50e5).unwrap(),
        ] {
            for brs in [false, true] {
                let data: Vec<u8> = (0..len).map(|n| n as u8).collect();
                let frame = CanFrame::fd(id, &data, brs).unwrap();
                assert_eq!(frame.dlc(), dlc as u8);
                roundtrip(codec, frame);
            }
        }
    }
    let data: Vec<u8> = (0..12).collect();
    let mut buf = [0; 139];
    let n = codec
        .encode_frame(
            &CanFrame::fd(CanId::standard(0x123).unwrap(), &data, true).unwrap(),
            &mut buf,
        )
        .unwrap();
    assert_eq!(&buf[..n], b"b1239000102030405060708090A0B\r");
    let data: Vec<u8> = (0..16).collect();
    let n = codec
        .encode_frame(
            &CanFrame::fd(CanId::extended(0x18ff50e5).unwrap(), &data, false).unwrap(),
            &mut buf,
        )
        .unwrap();
    assert_eq!(&buf[..n], b"D18FF50E5A000102030405060708090A0B0C0D0E0F\r");
    let full = CanFrame::fd(CanId::extended(0x1fff_ffff).unwrap(), &[0xff; 64], true).unwrap();
    assert_eq!(codec.encode_frame(&full, &mut buf).unwrap(), 139);
}
#[test]
fn rejects_malformed() {
    let codec = SlcanCodec::new(AdapterProfile::CanableFd);
    for line in [
        b"t8000".as_slice(),
        b"t1231GG",
        b"t1232AA",
        b"t1239",
        b"TFFFFFFFF0",
        b"t12",
        b"t1230\n",
    ] {
        assert!(codec.decode_line(line).is_err(), "{line:?}");
    }
    assert!(matches!(
        codec.decode_line(b"t8000"),
        Err(Error::Protocol(ProtocolError::InvalidId))
    ));
    assert_eq!(codec.decode_line(&[7]).unwrap(), ParsedMessage::Nack);
    let classic = SlcanCodec::new(AdapterProfile::LawicelClassic);
    assert!(matches!(
        classic.decode_line(b"d1230"),
        Err(Error::UnsupportedFeature)
    ));
    #[cfg(feature = "can-fd")]
    for len in [9, 10, 11, 13, 15, 17, 63, 65] {
        assert!(CanFrame::fd(CanId::standard(1).unwrap(), &vec![0; len], false).is_err());
    }
}
#[test]
fn streaming_fragmentation_and_recovery() {
    let codec = SlcanCodec::new(AdapterProfile::CanableFd);
    let mut parser = StreamParser::new(codec);
    let mut messages = Vec::new();
    for byte in b"t1231AA\rt1231BB\r" {
        parser.push(&[*byte], |msg| messages.push(msg));
    }
    assert_eq!(messages.len(), 2);
    assert!(
        messages
            .iter()
            .all(|msg| matches!(msg, Ok(ParsedMessage::Frame(_))))
    );
    parser.push(&vec![b'X'; 257], |msg| messages.push(msg));
    parser.push(b"\rt1230\r", |msg| messages.push(msg));
    assert!(matches!(
        messages[2],
        Err(Error::Protocol(ProtocolError::LineTooLong))
    ));
    assert!(matches!(messages[3], Ok(ParsedMessage::Frame(_))));
}
proptest! {
    #[test]
    fn arbitrary_input_never_panics(bytes in prop::collection::vec(any::<u8>(), 0..400)) {
        let codec = SlcanCodec::new(AdapterProfile::CanableFd);
        let _ = codec.decode_line(&bytes);
        let mut parser = StreamParser::new(codec);
        parser.push(&bytes, |_| {});
    }
    #[test]
    fn classic_roundtrip(id in 0u16..=0x7ff, data in prop::collection::vec(any::<u8>(), 0..=8)) {
        roundtrip(SlcanCodec::new(AdapterProfile::LawicelClassic), CanFrame::classic(CanId::standard(id).unwrap(), &data).unwrap());
    }
}

#[cfg(feature = "can-fd")]
#[test]
fn full_length_fd_frame_recovers_at_every_split() {
    let codec = SlcanCodec::new(AdapterProfile::CanableFd);
    let frame = CanFrame::fd(CanId::extended(0x1fff_ffff).unwrap(), &[0xaa; 64], true).unwrap();
    let mut line = [0u8; 139];
    assert_eq!(codec.encode_frame(&frame, &mut line).unwrap(), 139);
    for split in 0..=line.len() {
        let mut parser = StreamParser::new(codec);
        let mut records = Vec::new();
        parser.push(&line[..split], |message| records.push(message));
        parser.push(&line[split..], |message| records.push(message));
        assert_eq!(records.len(), 1);
        assert_eq!(
            records.remove(0).unwrap(),
            ParsedMessage::Frame(frame.clone())
        );
    }
}

#[cfg(feature = "can-fd")]
proptest! {
    #[test]
    fn fd_roundtrip_all_kinds(
        extended in any::<bool>(), brs in any::<bool>(), id in 0u32..=0x7ff,
        bytes in prop::collection::vec(any::<u8>(), 64)
    ) {
        let id = if extended { CanId::extended(id).unwrap() } else { CanId::standard(id as u16).unwrap() };
        for len in [0,1,2,3,4,5,6,7,8,12,16,20,24,32,48,64] {
            roundtrip(SlcanCodec::new(AdapterProfile::CanableFd), CanFrame::fd(id, &bytes[..len], brs).unwrap());
        }
    }
    #[test]
    fn remote_roundtrip(id in 0u32..=0x1fff_ffff, dlc in 0u8..=8) {
        roundtrip(SlcanCodec::new(AdapterProfile::LawicelClassic), CanFrame::remote(CanId::extended(id).unwrap(), dlc).unwrap());
    }
}
