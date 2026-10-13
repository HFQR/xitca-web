use super::*;
use assert_matches::assert_matches;
use std::io::Cursor;

#[test]
fn unknown_frame_type() {
    let mut buf = Cursor::new(&[22, 4, 0, 255, 128, 0, 3, 1, 2]);
    assert_matches!(Frame::decode(&mut buf), Err(FrameError::UnknownFrame(22)));
    assert_matches!(Frame::decode(&mut buf), Ok(Frame::CancelPush(PushId(2))));
}

#[test]
fn len_unexpected_end() {
    let mut buf = Cursor::new(&[0, 255]);
    let decoded = Frame::decode(&mut buf);
    assert_matches!(decoded, Err(FrameError::Incomplete(3)));
}

#[test]
fn type_unexpected_end() {
    let mut buf = Cursor::new(&[255]);
    let decoded = Frame::decode(&mut buf);
    assert_matches!(decoded, Err(FrameError::Incomplete(2)));
}

#[test]
fn buffer_too_short() {
    let mut buf = Cursor::new(&[4, 4, 0, 255, 128]);
    let decoded = Frame::decode(&mut buf);
    assert_matches!(decoded, Err(FrameError::Incomplete(6)));
}

fn codec_frame_check(mut frame: Frame<Bytes>, wire: &[u8], check_frame: Frame<Bytes>) {
    let mut buf = Vec::new();
    frame.encode_with_payload(&mut buf);
    assert_eq!(&buf, &wire);

    let mut read = Cursor::new(&buf);
    let decoded = Frame::decode(&mut read).unwrap();
    assert_eq!(check_frame, decoded);
}

#[test]
fn settings_frame() {
    codec_frame_check(
        Frame::Settings(Settings {
            entries: [
                (SettingId::MAX_HEADER_LIST_SIZE, 0xfad1),
                (SettingId::QPACK_MAX_TABLE_CAPACITY, 0xfad2),
                (SettingId::QPACK_MAX_BLOCKED_STREAMS, 0xfad3),
                (SettingId(95), 0),
                (SettingId::NONE, 0),
                (SettingId::NONE, 0),
                (SettingId::NONE, 0),
                (SettingId::NONE, 0),
            ],
            len: 4,
        }),
        &[
            4, 18, 6, 128, 0, 250, 209, 1, 128, 0, 250, 210, 7, 128, 0, 250, 211, 64, 95, 0,
        ],
        Frame::Settings(Settings {
            entries: [
                (SettingId::MAX_HEADER_LIST_SIZE, 0xfad1),
                (SettingId::QPACK_MAX_TABLE_CAPACITY, 0xfad2),
                (SettingId::QPACK_MAX_BLOCKED_STREAMS, 0xfad3),
                // check without the Grease setting because this is ignored
                (SettingId(0), 0),
                (SettingId::NONE, 0),
                (SettingId::NONE, 0),
                (SettingId::NONE, 0),
                (SettingId::NONE, 0),
            ],
            len: 3,
        }),
    );
}

#[test]
fn settings_frame_emtpy() {
    codec_frame_check(
        Frame::Settings(Settings::default()),
        &[4, 0],
        Frame::Settings(Settings::default()),
    );
}

#[test]
fn data_frame() {
    codec_frame_check(
        Frame::Data(Bytes::from("1234567")),
        &[0, 7, 49, 50, 51, 52, 53, 54, 55],
        Frame::Data(Bytes::from("1234567")),
    );
}

#[test]
fn simple_frames() {
    codec_frame_check(Frame::CancelPush(PushId(2)), &[3, 1, 2], Frame::CancelPush(PushId(2)));
    codec_frame_check(Frame::Goaway(VarInt(2)), &[7, 1, 2], Frame::Goaway(VarInt(2)));
    codec_frame_check(Frame::MaxPushId(PushId(2)), &[13, 1, 2], Frame::MaxPushId(PushId(2)));
}

#[test]
fn headers_frames() {
    codec_frame_check(
        Frame::headers("TODO QPACK"),
        &[1, 10, 84, 79, 68, 79, 32, 81, 80, 65, 67, 75],
        Frame::headers("TODO QPACK"),
    );
    codec_frame_check(
        Frame::PushPromise(PushPromise {
            id: 134,
            encoded: Bytes::from("TODO QPACK"),
        }),
        &[5, 12, 64, 134, 84, 79, 68, 79, 32, 81, 80, 65, 67, 75],
        Frame::PushPromise(PushPromise {
            id: 134,
            encoded: Bytes::from("TODO QPACK"),
        }),
    );
}

#[test]
fn reserved_frame() {
    let mut raw = vec![];
    VarInt::from_u32(0x21 + 2 * 0x1f).encode(&mut raw);
    raw.extend(&[6, 0, 255, 128, 0, 250, 218]);
    let mut buf = Cursor::new(&raw);
    let decoded = Frame::decode(&mut buf);
    assert_matches!(decoded, Err(FrameError::UnknownFrame(95)));
}
