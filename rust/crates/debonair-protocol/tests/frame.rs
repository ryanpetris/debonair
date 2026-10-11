use debonair_protocol::{
    ClientMessage, GetStatus, Input, KeyboardState, MouseInput, Request, SetBattery,
    client_message::Kind,
    frame::{self, Decoder, FrameError},
    input,
    request::Command,
};
use prost::Message;

fn frames(decoder: &mut Decoder, bytes: &[u8]) -> Vec<Result<Vec<u8>, FrameError>> {
    bytes
        .iter()
        .filter_map(|&b| decoder.push(b).map(|r| r.map(<[u8]>::to_vec)))
        .collect()
}

#[test]
fn round_trips_every_block_boundary() {
    let mut decoder = Decoder::new(None);
    for length in [0, 1, 2, 253, 254, 255, 256, 508, 509, 510, 2000] {
        for zeros in [false, true] {
            let bytes: Vec<u8> = (0..length)
                .map(|i| {
                    if zeros && i % 7 == 0 {
                        0
                    } else {
                        (i % 251 + 1) as u8
                    }
                })
                .collect();
            let mut encoded = Vec::new();
            frame::encode_bytes(&bytes, &mut encoded);
            assert_eq!(encoded.last(), Some(&0));
            assert!(!encoded[..encoded.len() - 1].contains(&0));
            let decoded = frames(&mut decoder, &encoded);
            if length == 0 {
                // The encoding of an empty message is a single code byte.
                assert_eq!(decoded, vec![Ok(Vec::new())]);
            } else {
                assert_eq!(decoded, vec![Ok(bytes)]);
            }
        }
    }
}

#[test]
fn skips_empty_frames_and_resynchronizes() {
    let mut decoder = Decoder::new(None);
    let mut stream = vec![9, 9, 9, 0, 0, 0];
    frame::encode(
        &Request {
            id: 7,
            command: Some(Command::GetStatus(GetStatus {})),
        },
        &mut stream,
    );
    let decoded = frames(&mut decoder, &stream);
    // The leftover bytes decode as a malformed frame; empty frames are skipped.
    assert_eq!(decoded.len(), 2);
    assert_eq!(decoded[0], Err(FrameError::Malformed));
    let request = Request::decode(decoded[1].as_ref().unwrap().as_slice()).unwrap();
    assert!(matches!(request.command, Some(Command::GetStatus(_))));
}

#[test]
fn rejects_frames_over_the_limit_until_the_next_delimiter() {
    let mut decoder = Decoder::new(Some(16));
    let mut stream = Vec::new();
    frame::encode_bytes(&[1; 17], &mut stream);
    frame::encode_bytes(&[2; 16], &mut stream);
    frame::encode_bytes(&[3; 600], &mut stream);
    frame::encode_bytes(&[4; 1], &mut stream);
    assert_eq!(
        frames(&mut decoder, &stream),
        vec![
            Err(FrameError::TooLong),
            Ok(vec![2; 16]),
            Err(FrameError::TooLong),
            Ok(vec![4]),
        ]
    );
}

#[test]
fn reset_drops_a_partial_frame() {
    let mut decoder = Decoder::new(None);
    let mut first = Vec::new();
    frame::encode_bytes(b"old", &mut first);
    for &b in &first[..2] {
        assert!(decoder.push(b).is_none());
    }
    decoder.reset();
    let mut second = Vec::new();
    frame::encode_bytes(b"new", &mut second);
    assert_eq!(frames(&mut decoder, &second), vec![Ok(b"new".to_vec())]);
}

#[test]
fn unknown_commands_decode_as_no_command() {
    // Field 99 stands for a command added after this build.
    let mut bytes = Vec::new();
    prost::encoding::uint32::encode(1, &3, &mut bytes);
    prost::encoding::message::encode(99, &SetBattery { level: 5 }, &mut bytes);
    let request = Request::decode(bytes.as_slice()).unwrap();
    assert_eq!(request.id, 3);
    assert!(request.command.is_none());
}

#[test]
fn unknown_fields_are_ignored() {
    let mut bytes = SetBattery { level: 40 }.encode_to_vec();
    prost::encoding::string::encode(15, &"later".to_string(), &mut bytes);
    assert_eq!(SetBattery::decode(bytes.as_slice()).unwrap().level, 40);
}

fn input_frame(kind: input::Kind) -> Vec<u8> {
    let mut out = Vec::new();
    frame::encode(
        &ClientMessage {
            kind: Some(Kind::Input(Input { kind: Some(kind) })),
        },
        &mut out,
    );
    out
}

#[test]
fn input_frames_fit_one_usb_packet() {
    // Every input frame must fit one 64-byte full-speed USB packet, so one transfer carries it.
    let keyboard = input_frame(input::Kind::Keyboard(KeyboardState {
        modifiers: 0xff,
        keys: vec![0xa4; 6],
    }));
    let mouse = input_frame(input::Kind::Mouse(MouseInput {
        buttons: 0x1f,
        x: -32768,
        y: 32767,
        wheel: -32768,
        pan: 32767,
    }));
    assert!(keyboard.len() <= 20, "{}", keyboard.len());
    assert!(mouse.len() <= 24, "{}", mouse.len());
}
