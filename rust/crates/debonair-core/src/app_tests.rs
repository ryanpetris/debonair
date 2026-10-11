use crate::{
    ErrorCode,
    app::{Action, App, Build},
    hid::{Channel, Encoded},
    protocol::{
        self as p, ClientMessage, DongleMessage, client_message::Kind as ClientKind,
        dongle_message::Kind as DongleKind, event::Kind as EventKind, frame,
        input::Kind as InputKind, request::Command, response::Result as Outcome,
    },
    radio::{Config, Event, Radio, Security, Target},
    store::memory::Memory,
};
use alloc::{string::String, vec, vec::Vec};
use prost::Message;

#[derive(Default)]
struct Fake {
    started: Option<(String, u8, usize)>,
    pairing: Vec<bool>,
    sent: Vec<Encoded>,
    blocked: bool,
    targets: Vec<Target>,
    passkeys: Vec<Option<u32>>,
    names: Vec<String>,
    battery: Option<u8>,
}
impl Radio for Fake {
    fn start(&mut self, config: &Config<'_>) -> Result<(), ErrorCode> {
        self.started = Some((
            config.name.into(),
            config.devices.index(),
            config.descriptor.len(),
        ));
        Ok(())
    }
    fn set_name(&mut self, name: &str) -> Result<(), ErrorCode> {
        self.names.push(name.into());
        Ok(())
    }
    fn set_security(&mut self, _: Security) {}
    fn set_pairing(&mut self, enabled: bool) {
        self.pairing.push(enabled);
    }
    fn passkey(&mut self, passkey: Option<u32>) -> Result<(), ErrorCode> {
        self.passkeys.push(passkey);
        Ok(())
    }
    fn disconnect(&mut self) -> Result<(), ErrorCode> {
        Ok(())
    }
    fn targets(&mut self) -> Vec<Target> {
        self.targets.clone()
    }
    fn max_targets(&self) -> u32 {
        4
    }
    fn forget(&mut self, target: Target) -> Result<(), ErrorCode> {
        self.targets.retain(|t| *t != target);
        Ok(())
    }
    fn set_battery(&mut self, level: u8) {
        self.battery = Some(level);
    }
    fn can_send(&mut self) -> bool {
        !self.blocked
    }
    fn send(&mut self, report: &Encoded) -> Result<(), ErrorCode> {
        self.sent.push(*report);
        Ok(())
    }
    fn next_event(&mut self) -> Option<Event> {
        None
    }
}

const TARGET: Target = Target {
    address: [1, 2, 3, 4, 5, 6],
    random: false,
};

fn app(store: &mut Memory) -> App {
    App::new(
        Build {
            version: "0.0.0",
            board: "pico_w",
            model: "Pico W",
            development: false,
            adapter_id: "ABCD".into(),
        },
        store,
    )
}

fn frame_of(kind: ClientKind) -> Vec<u8> {
    let mut out = vec![0];
    frame::encode(&ClientMessage { kind: Some(kind) }, &mut out);
    out
}
fn frame_of_none() -> Vec<u8> {
    let mut out = vec![0];
    frame::encode(&ClientMessage { kind: None }, &mut out);
    out
}
fn request(id: u32, command: Command) -> Vec<u8> {
    frame_of(ClientKind::Request(p::Request {
        id,
        command: Some(command),
    }))
}
fn input(kind: InputKind) -> Vec<u8> {
    frame_of(ClientKind::Input(p::Input { kind: Some(kind) }))
}
fn messages(app: &mut App) -> Vec<DongleKind> {
    let mut bytes = vec![0; 8192];
    let n = app.output(&mut bytes);
    let mut decoder = frame::Decoder::new(None);
    let mut out = Vec::new();
    for &b in &bytes[..n] {
        if let Some(Ok(f)) = decoder.push(b) {
            out.push(DongleMessage::decode(f).unwrap().kind.unwrap());
        }
    }
    out
}
fn responses(app: &mut App) -> Vec<(u32, Option<Outcome>)> {
    messages(app)
        .into_iter()
        .filter_map(|m| match m {
            DongleKind::Response(r) => Some((r.id, r.result)),
            _ => None,
        })
        .collect()
}

fn connected() -> (Memory, App, Fake) {
    let mut store = Memory::default();
    let mut app = app(&mut store);
    let mut radio = Fake {
        targets: vec![TARGET],
        ..Fake::default()
    };
    app.start(&mut radio);
    app.radio_event(Event::Ready, &mut radio);
    app.radio_event(Event::Connected(TARGET), &mut radio);
    app.radio_event(Event::Secured, &mut radio);
    app.session(true);
    (store, app, radio)
}

#[test]
fn starts_with_defaults_and_pairs_when_nothing_is_paired() {
    let mut store = Memory::default();
    let mut app = app(&mut store);
    let mut radio = Fake::default();
    app.start(&mut radio);
    let (name, index, length) = radio.started.clone().unwrap();
    assert_eq!(name, "Debonair");
    assert_eq!(index, 3);
    assert!(length > 100);
    app.radio_event(Event::Ready, &mut radio);
    assert_eq!(radio.pairing, [true]);
    assert_eq!(app.status().link, p::LinkState::Pairing as i32);
}

#[test]
fn input_reaches_the_target_in_order() {
    let (mut store, mut app, mut radio) = connected();
    let mut bytes = input(InputKind::Keyboard(p::KeyboardState {
        modifiers: 1,
        keys: vec![],
    }));
    bytes.extend(input(InputKind::Mouse(p::MouseInput {
        buttons: 1,
        x: 3,
        ..Default::default()
    })));
    app.receive(&bytes, &mut radio, &mut store, 0);
    app.poll(&mut radio, 0);
    let channels: Vec<_> = radio.sent.iter().map(|r| r.channel).collect();
    assert_eq!(channels, [Channel::Input(1), Channel::Input(2)]);
    assert_eq!(radio.sent[1].data()[..3], [1, 3, 0]);
}

#[test]
fn reports_wait_while_the_target_is_busy() {
    let (mut store, mut app, mut radio) = connected();
    radio.blocked = true;
    for x in [1, 2, 3] {
        app.receive(
            &input(InputKind::Mouse(p::MouseInput {
                x,
                ..Default::default()
            })),
            &mut radio,
            &mut store,
            0,
        );
        app.poll(&mut radio, 0);
    }
    assert!(radio.sent.is_empty());
    radio.blocked = false;
    app.radio_event(Event::CanSend, &mut radio);
    app.poll(&mut radio, 0);
    assert_eq!(radio.sent.len(), 1);
    assert_eq!(radio.sent[0].data()[1], 6);
}

#[test]
fn closing_the_session_releases_held_keys() {
    let (mut store, mut app, mut radio) = connected();
    app.receive(
        &input(InputKind::Keyboard(p::KeyboardState {
            modifiers: 0,
            keys: vec![4],
        })),
        &mut radio,
        &mut store,
        0,
    );
    app.poll(&mut radio, 0);
    app.session(false);
    app.poll(&mut radio, 0);
    assert_eq!(radio.sent.len(), 2);
    assert_eq!(radio.sent[1].data(), [0; 8]);
}

#[test]
fn input_is_dropped_without_a_secured_target() {
    let (mut store, mut app, mut radio) = connected();
    app.radio_event(Event::Disconnected, &mut radio);
    app.receive(
        &input(InputKind::Keyboard(p::KeyboardState {
            modifiers: 0,
            keys: vec![4],
        })),
        &mut radio,
        &mut store,
        0,
    );
    app.poll(&mut radio, 0);
    app.radio_event(Event::Connected(TARGET), &mut radio);
    app.radio_event(Event::Secured, &mut radio);
    app.poll(&mut radio, 0);
    assert!(radio.sent.is_empty());
}

#[test]
fn disabled_devices_ignore_their_input() {
    let (mut store, mut app, mut radio) = connected();
    let mut bytes = request(
        1,
        Command::SetConfig(p::SetConfig {
            mouse: Some(false),
            ..Default::default()
        }),
    );
    bytes.extend(input(InputKind::Mouse(p::MouseInput {
        x: 5,
        ..Default::default()
    })));
    app.receive(&bytes, &mut radio, &mut store, 0);
    // The new combination takes effect after the restart, once the Client has the Response.
    assert_eq!(app.poll(&mut radio, 0), None);
    assert!(matches!(
        responses(&mut app).as_slice(),
        [(1, Some(Outcome::Status(_)))]
    ));
    assert_eq!(app.poll(&mut radio, 0), Some(Action::Restart));
    assert!(radio.sent.is_empty());
    let mut restarted = self::app(&mut store);
    assert!(!restarted.settings.devices.mouse);
    let mut radio = Fake::default();
    restarted.start(&mut radio);
    assert_eq!(radio.started.unwrap().1, 1);
}

#[test]
fn rejects_invalid_configuration() {
    let (mut store, mut app, mut radio) = connected();
    for (id, change) in [
        (
            1,
            p::SetConfig {
                keyboard: Some(false),
                mouse: Some(false),
                ..Default::default()
            },
        ),
        (
            2,
            p::SetConfig {
                name: Some("bad\nname".into()),
                ..Default::default()
            },
        ),
    ] {
        app.receive(
            &request(id, Command::SetConfig(change)),
            &mut radio,
            &mut store,
            0,
        );
    }
    let invalid = Some(Outcome::Error(p::Error {
        code: ErrorCode::InvalidValue.into(),
    }));
    assert_eq!(responses(&mut app), [(1, invalid.clone()), (2, invalid)]);
    assert!(store.files.is_empty());
}

#[test]
fn renaming_applies_without_a_restart() {
    let (mut store, mut app, mut radio) = connected();
    app.receive(
        &request(
            4,
            Command::SetConfig(p::SetConfig {
                name: Some("Desk".into()),
                ..Default::default()
            }),
        ),
        &mut radio,
        &mut store,
        0,
    );
    assert_eq!(radio.names, ["Desk"]);
    assert_eq!(app.poll(&mut radio, 0), None);
    assert_eq!(app.status().config.unwrap().name, "Desk");
}

#[test]
fn storage_failure_keeps_the_previous_settings() {
    let (mut store, mut app, mut radio) = connected();
    store.fail = true;
    app.receive(
        &request(
            2,
            Command::SetConfig(p::SetConfig {
                name: Some("Desk".into()),
                ..Default::default()
            }),
        ),
        &mut radio,
        &mut store,
        0,
    );
    assert_eq!(
        responses(&mut app),
        [(
            2,
            Some(Outcome::Error(p::Error {
                code: ErrorCode::StorageFailed.into()
            }))
        )]
    );
    assert_eq!(app.name(), "Debonair");
}

#[test]
fn storage_that_did_not_open_is_not_ready() {
    let (_, mut app, mut radio) = connected();
    for (id, error) in [
        (1, crate::store::Error::Layout),
        (2, crate::store::Error::Unprovisioned),
        (3, crate::store::Error::Unavailable),
    ] {
        let mut store: Result<Memory, _> = Err(error);
        app.receive(
            &request(
                id,
                Command::SetConfig(p::SetConfig {
                    name: Some("Desk".into()),
                    ..Default::default()
                }),
            ),
            &mut radio,
            &mut store,
            0,
        );
    }
    let not_ready = Some(Outcome::Error(p::Error {
        code: ErrorCode::NotReady.into(),
    }));
    assert_eq!(
        responses(&mut app),
        [
            (1, not_ready.clone()),
            (2, not_ready.clone()),
            (3, not_ready)
        ]
    );
    assert_eq!(app.name(), "Debonair");
}

#[test]
fn storage_that_opened_reports_a_failed_save_as_a_storage_failure() {
    let (mut store, mut app, mut radio) = connected();
    store.unavailable = true;
    app.receive(
        &request(
            1,
            Command::SetConfig(p::SetConfig {
                name: Some("Desk".into()),
                ..Default::default()
            }),
        ),
        &mut radio,
        &mut store,
        0,
    );
    assert_eq!(
        responses(&mut app),
        [(
            1,
            Some(Outcome::Error(p::Error {
                code: ErrorCode::StorageFailed.into()
            }))
        )]
    );
}

#[test]
fn passkey_requests_round_trip() {
    let (mut store, mut app, mut radio) = connected();
    let enter = |passkey| request(9, Command::EnterPasskey(p::EnterPasskey { passkey }));
    app.receive(&enter(Some(1)), &mut radio, &mut store, 0);
    app.radio_event(Event::PasskeyRequest, &mut radio);
    app.receive(&enter(Some(123_456)), &mut radio, &mut store, 0);
    let messages = messages(&mut app);
    assert!(matches!(
        &messages[0],
        DongleKind::Response(p::Response {
            id: 9,
            result: Some(Outcome::Error(_))
        })
    ));
    assert!(matches!(
        &messages[1],
        DongleKind::Event(p::Event {
            kind: Some(EventKind::PasskeyRequest(_))
        })
    ));
    assert!(matches!(
        &messages[2],
        DongleKind::Response(p::Response {
            id: 9,
            result: None
        })
    ));
    assert_eq!(radio.passkeys, [Some(123_456)]);
}

#[test]
fn smooth_scrolling_follows_the_feature_report() {
    let (mut store, mut app, mut radio) = connected();
    let scroll = input(InputKind::Mouse(p::MouseInput {
        wheel: 60,
        ..Default::default()
    }));
    app.receive(&scroll, &mut radio, &mut store, 0);
    app.poll(&mut radio, 0);
    assert!(radio.sent.is_empty());
    app.radio_event(
        Event::Feature {
            report_id: 2,
            data: heapless::Vec::from_slice(&[0x05]).unwrap(),
        },
        &mut radio,
    );
    app.receive(&scroll, &mut radio, &mut store, 0);
    app.poll(&mut radio, 0);
    assert_eq!(radio.sent.len(), 1);
    assert_eq!(radio.sent[0].data()[5..7], 60i16.to_le_bytes());
    assert!(app.status().smooth_scrolling);
}

#[test]
fn forget_and_battery_update_status() {
    let (mut store, mut app, mut radio) = connected();
    let mut bytes = request(
        1,
        Command::ForgetTarget(p::ForgetTarget {
            address: TARGET.address.to_vec(),
            address_type: p::AddressType::Public.into(),
        }),
    );
    bytes.extend(request(2, Command::SetBattery(p::SetBattery { level: 42 })));
    bytes.extend(request(
        3,
        Command::SetBattery(p::SetBattery { level: 101 }),
    ));
    app.receive(&bytes, &mut radio, &mut store, 0);
    let results: Vec<_> = responses(&mut app).into_iter().map(|r| r.1).collect();
    assert_eq!(results[0], None);
    assert_eq!(results[1], None);
    assert!(matches!(results[2], Some(Outcome::Error(_))));
    assert_eq!(radio.battery, Some(42));
    let status = app.status();
    assert!(status.targets.is_empty());
    assert_eq!(status.battery, 42);
}

#[test]
fn malformed_frames_get_an_error_and_the_stream_recovers() {
    let (mut store, mut app, mut radio) = connected();
    let mut bytes = vec![5, 0xff, 0];
    bytes.extend(request(7, Command::GetStatus(p::GetStatus {})));
    app.receive(&bytes, &mut radio, &mut store, 0);
    let responses = responses(&mut app);
    assert_eq!(responses.len(), 2);
    assert!(matches!(responses[0], (0, Some(Outcome::Error(_)))));
    assert!(matches!(responses[1], (7, Some(Outcome::Status(_)))));
}

#[test]
fn restart_waits_for_the_response() {
    let (mut store, mut app, mut radio) = connected();
    app.receive(
        &request(1, Command::Restart(p::Restart {})),
        &mut radio,
        &mut store,
        1000,
    );
    assert_eq!(app.poll(&mut radio, 1000), None);
    assert_eq!(app.poll(&mut radio, 1250), Some(Action::Restart));
    app.receive(
        &request(2, Command::EnterBootloader(p::EnterBootloader {})),
        &mut radio,
        &mut store,
        2000,
    );
    assert!(matches!(
        responses(&mut app).last(),
        Some((2, Some(Outcome::Error(_))))
    ));
}

#[test]
fn requests_without_a_known_command_are_refused() {
    let (mut store, mut app, mut radio) = connected();
    let bytes = frame_of(ClientKind::Request(p::Request {
        id: 5,
        command: None,
    }));
    app.receive(&bytes, &mut radio, &mut store, 0);
    assert_eq!(
        responses(&mut app),
        [(
            5,
            Some(Outcome::Error(p::Error {
                code: ErrorCode::UnknownCommand.into()
            }))
        )]
    );
}

#[test]
fn frames_without_a_message_or_over_the_limit_get_an_error() {
    let (mut store, mut app, mut radio) = connected();
    let mut bytes = frame_of_none();
    bytes.extend([0x55; crate::protocol::MAX_CLIENT_MESSAGE_BYTES + 10]);
    bytes.push(0);
    bytes.extend(request(3, Command::GetStatus(p::GetStatus {})));
    app.receive(&bytes, &mut radio, &mut store, 0);
    let invalid = Some(Outcome::Error(p::Error {
        code: ErrorCode::InvalidRequest.into(),
    }));
    let responses = responses(&mut app);
    assert_eq!(responses[..2], [(0, invalid.clone()), (0, invalid)]);
    assert!(matches!(responses[2], (3, Some(Outcome::Status(_)))));
}

#[test]
fn reading_pauses_instead_of_dropping_responses() {
    let (mut store, mut app, mut radio) = connected();
    let count = 300;
    let bytes: Vec<u8> = (1..=count)
        .flat_map(|id| request(id, Command::GetStatus(p::GetStatus {})))
        .collect();
    let mut offset = app.receive(&bytes, &mut radio, &mut store, 0);
    assert!(
        offset < bytes.len(),
        "the Client's bytes wait while output is full"
    );
    let mut ids = Vec::new();
    loop {
        ids.extend(responses(&mut app).into_iter().map(|(id, _)| id));
        if offset == bytes.len() {
            break;
        }
        offset += app.receive(&bytes[offset..], &mut radio, &mut store, 0);
    }
    assert_eq!(ids, (1..=count).collect::<Vec<_>>());
}

#[test]
fn status_events_wait_for_earlier_output_and_carry_the_latest_status() {
    let (mut store, mut app, mut radio) = connected();
    for (id, level) in [(1, 42), (2, 43)] {
        app.receive(
            &request(id, Command::SetBattery(p::SetBattery { level })),
            &mut radio,
            &mut store,
            0,
        );
        app.poll(&mut radio, 0);
    }
    assert_eq!(responses(&mut app), [(1, None), (2, None)]);
    app.poll(&mut radio, 0);
    let events = messages(&mut app);
    assert!(matches!(
        events.as_slice(),
        [DongleKind::Event(p::Event {
            kind: Some(EventKind::Status(p::Status { battery: 43, .. }))
        })]
    ));
    app.poll(&mut radio, 0);
    assert!(messages(&mut app).is_empty());
}

fn events(app: &mut App) -> Vec<EventKind> {
    messages(app)
        .into_iter()
        .filter_map(|m| match m {
            DongleKind::Event(p::Event { kind }) => kind,
            _ => None,
        })
        .collect()
}

/// Fills the output with responses until reading pauses.
fn fill(app: &mut App, radio: &mut Fake, store: &mut Memory) {
    let bytes: Vec<u8> = (1..=300)
        .flat_map(|id| request(id, Command::GetStatus(p::GetStatus {})))
        .collect();
    assert!(app.receive(&bytes, radio, store, 0) < bytes.len());
}

#[test]
fn one_shot_events_wait_for_room_one_of_each_kind() {
    let (mut store, mut app, mut radio) = connected();
    fill(&mut app, &mut radio, &mut store);
    // Only the latest prompt can be answered.
    app.radio_event(Event::PasskeyRequest, &mut radio);
    app.radio_event(Event::PasskeyRequest, &mut radio);
    let written = events(&mut app);
    assert!(
        written.is_empty(),
        "nothing is written while output is full"
    );
    app.poll(&mut radio, 0);
    assert!(matches!(
        events(&mut app).as_slice(),
        [EventKind::PasskeyRequest(_)]
    ));

    // A result closes the prompt and replaces an earlier result; a new prompt after it stays
    // behind it.
    fill(&mut app, &mut radio, &mut store);
    app.radio_event(Event::PasskeyRequest, &mut radio);
    app.radio_event(Event::PairingComplete { success: false }, &mut radio);
    app.radio_event(Event::PairingComplete { success: true }, &mut radio);
    app.radio_event(Event::PasskeyRequest, &mut radio);
    events(&mut app);
    app.poll(&mut radio, 0);
    assert!(matches!(
        events(&mut app).as_slice(),
        [
            EventKind::PairingResult(p::PairingResult { success: true }),
            EventKind::PasskeyRequest(_)
        ]
    ));

    // A disconnect closes a prompt the Client has not seen.
    fill(&mut app, &mut radio, &mut store);
    app.radio_event(Event::PasskeyRequest, &mut radio);
    app.radio_event(Event::Disconnected, &mut radio);
    events(&mut app);
    app.poll(&mut radio, 0);
    assert!(
        !events(&mut app)
            .iter()
            .any(|e| matches!(e, EventKind::PasskeyRequest(_)))
    );
}

#[test]
fn a_waiting_event_goes_out_before_later_responses() {
    let (mut store, mut app, mut radio) = connected();
    fill(&mut app, &mut radio, &mut store);
    app.radio_event(Event::PairingComplete { success: false }, &mut radio);
    messages(&mut app);
    // The Client's next request is read once output has room, after the result it follows.
    let bytes = request(500, Command::GetStatus(p::GetStatus {}));
    assert_eq!(app.receive(&bytes, &mut radio, &mut store, 0), bytes.len());
    let messages = messages(&mut app);
    assert!(matches!(
        messages.as_slice(),
        [
            DongleKind::Event(p::Event {
                kind: Some(EventKind::PairingResult(_))
            }),
            DongleKind::Response(p::Response { id: 500, .. }),
        ]
    ));
}

#[test]
fn reading_waits_for_an_event_that_cannot_be_queued() {
    let (mut store, mut app, mut radio) = connected();
    app.no_memory = true;
    app.radio_event(Event::PairingComplete { success: true }, &mut radio);
    let bytes = request(7, Command::GetStatus(p::GetStatus {}));
    assert_eq!(app.receive(&bytes, &mut radio, &mut store, 0), 0);
    app.no_memory = false;
    assert_eq!(app.receive(&bytes, &mut radio, &mut store, 0), bytes.len());
    assert!(matches!(
        messages(&mut app).as_slice(),
        [
            DongleKind::Event(p::Event {
                kind: Some(EventKind::PairingResult(_))
            }),
            DongleKind::Response(p::Response { id: 7, .. }),
        ]
    ));
}

#[test]
fn one_shot_events_are_written_at_once_with_room() {
    let (mut store, mut app, mut radio) = connected();
    app.receive(
        &request(1, Command::GetStatus(p::GetStatus {})),
        &mut radio,
        &mut store,
        0,
    );
    app.radio_event(Event::PasskeyRequest, &mut radio);
    app.radio_event(Event::PairingComplete { success: true }, &mut radio);
    let messages = messages(&mut app);
    assert!(matches!(
        messages.as_slice(),
        [
            DongleKind::Response(p::Response { id: 1, .. }),
            DongleKind::Event(p::Event {
                kind: Some(EventKind::PasskeyRequest(_))
            }),
            DongleKind::Event(p::Event {
                kind: Some(EventKind::PairingResult(_))
            }),
        ]
    ));
}

#[test]
fn a_response_without_memory_is_held_and_reading_waits() {
    let (mut store, mut app, mut radio) = connected();
    let mut bytes = request(1, Command::GetStatus(p::GetStatus {}));
    let first = bytes.len();
    bytes.extend(request(2, Command::SetBattery(p::SetBattery { level: 42 })));
    app.no_memory = true;
    assert_eq!(app.receive(&bytes, &mut radio, &mut store, 0), first);
    assert_eq!(app.receive(&bytes[first..], &mut radio, &mut store, 0), 0);
    assert!(!app.has_output());
    app.no_memory = false;
    app.poll(&mut radio, 0);
    assert_eq!(
        app.receive(&bytes[first..], &mut radio, &mut store, 0),
        bytes.len() - first
    );
    let ids: Vec<u32> = responses(&mut app).into_iter().map(|(id, _)| id).collect();
    assert_eq!(ids, [1, 2]);
}

#[test]
fn events_without_memory_stay_waiting() {
    let (mut store, mut app, mut radio) = connected();
    app.receive(
        &request(1, Command::SetBattery(p::SetBattery { level: 42 })),
        &mut radio,
        &mut store,
        0,
    );
    responses(&mut app);
    app.no_memory = true;
    app.radio_event(Event::PairingComplete { success: false }, &mut radio);
    app.poll(&mut radio, 0);
    assert!(!app.has_output());
    app.no_memory = false;
    app.poll(&mut radio, 0);
    assert!(matches!(
        events(&mut app).as_slice(),
        [EventKind::PairingResult(p::PairingResult {
            success: false
        })]
    ));
    app.poll(&mut radio, 0);
    assert!(matches!(
        events(&mut app).as_slice(),
        [EventKind::Status(p::Status { battery: 42, .. })]
    ));
}

#[test]
fn a_restart_waits_for_a_held_response() {
    let (mut store, mut app, mut radio) = connected();
    app.no_memory = true;
    app.receive(
        &request(1, Command::Restart(p::Restart {})),
        &mut radio,
        &mut store,
        1000,
    );
    assert_eq!(app.poll(&mut radio, 1000), None);
    app.no_memory = false;
    assert_eq!(app.poll(&mut radio, 1000), None);
    assert_eq!(responses(&mut app), [(1, None)]);
    assert_eq!(app.poll(&mut radio, 1000), Some(Action::Restart));
}

#[test]
fn a_new_session_starts_with_nothing_waiting() {
    let (mut store, mut app, mut radio) = connected();
    app.receive(
        &request(1, Command::SetBattery(p::SetBattery { level: 42 })),
        &mut radio,
        &mut store,
        0,
    );
    app.radio_event(Event::PasskeyRequest, &mut radio);
    app.session(true);
    app.poll(&mut radio, 0);
    assert!(messages(&mut app).is_empty());
}
