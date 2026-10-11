//! The application: answers the Client's requests, applies its input and reports what the Bluetooth
//! backend does. One owner task calls every method, so nothing here is shared.
//!
//! Output to the Client is bounded and never silently dropped while its session lasts:
//!
//! - Every request gets its Response. Reading from the Client pauses while [`OUTPUT_BYTES`] wait
//!   for it, and while a Response or one-shot event waits to be queued, so a Client that stops
//!   reading holds back its own requests and input rather than losing answers, and no Response
//!   passes an earlier event.
//! - At most one PasskeyRequest and one PairingResult wait, in the order they happened, and they
//!   are written once fewer than [`OUTPUT_BYTES`] wait and no Response is held. A newer
//!   PasskeyRequest replaces an unwritten one, since only the latest prompt can be answered. A
//!   PairingResult replaces an unwritten one, since the Client acts on the latest outcome, and
//!   discards an unwritten PasskeyRequest, whose prompt it closes; a disconnect discards it too.
//! - A Status event carries the latest status and is written once nothing else is waiting, so
//!   changes made in the meantime go out together.
use crate::{
    ErrorCode,
    hid::{self, Devices},
    protocol::{
        self as p, ClientMessage, DongleMessage, MAX_CLIENT_MESSAGE_BYTES,
        client_message::Kind as ClientKind, dongle_message::Kind as DongleKind,
        event::Kind as EventKind, frame, input::Kind as InputKind, request::Command,
        response::Result as Outcome,
    },
    radio::{self, Event, Radio, Target},
    reports::Reports,
    settings::{Settings, valid_name},
    store::Store,
};
use alloc::{collections::VecDeque, string::String, vec::Vec};
use heapless::Vec as Bounded;
use prost::Message;

/// Reading from the Client pauses while at least this many bytes wait for it.
pub const OUTPUT_BYTES: usize = 4096;
/// How long a restart waits for its Response to reach the Client.
const ACTION_GRACE_MS: u64 = 250;

/// Facts about this firmware build and board.
pub struct Build {
    pub version: &'static str,
    pub board: &'static str,
    /// The board's product name, shown as the model in Device Information.
    pub model: &'static str,
    pub development: bool,
    /// The adapter ID: the USB serial number.
    pub adapter_id: String,
}

/// The name Targets see until the user sets one.
pub const DEFAULT_NAME: &str = "Debonair";

/// A one-shot event waiting for room in the output.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Notice {
    PasskeyRequest,
    PairingResult(bool),
}

/// Work the platform does after the Response has been sent.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Action {
    Restart,
    Bootloader,
}

pub struct App {
    build: Build,
    pub settings: Settings,
    descriptor: Vec<u8>,
    ready: bool,
    connected: Option<Target>,
    secured: bool,
    pairing: bool,
    passkey_pending: bool,
    leds: u8,
    smooth: (bool, bool),
    boot: bool,
    battery: u8,
    targets: Vec<Target>,
    max_targets: u32,
    reports: Reports,
    decoder: frame::Decoder,
    output: VecDeque<u8>,
    /// A Response that memory could not be reserved for in `output`. Reading waits until it is
    /// queued.
    held: Option<Vec<u8>>,
    /// One-shot events not yet in `output`, oldest first: at most one of each kind.
    notices: Bounded<Notice, 2>,
    session: bool,
    status_changed: bool,
    action: Option<(Action, u64)>,
    /// Makes every reservation in `output` fail, as when memory runs out.
    #[cfg(test)]
    pub(crate) no_memory: bool,
}

fn error(code: ErrorCode) -> Outcome {
    Outcome::Error(p::Error { code: code.into() })
}

/// The frame of an Event.
fn event(kind: EventKind) -> Vec<u8> {
    let mut bytes = Vec::new();
    frame::encode(
        &DongleMessage {
            kind: Some(DongleKind::Event(p::Event { kind: Some(kind) })),
        },
        &mut bytes,
    );
    bytes
}

impl App {
    /// Loads saved settings. A storage failure leaves the defaults in effect.
    pub fn new(build: Build, store: &mut impl Store) -> Self {
        let settings = Settings::load(store).unwrap_or_default();
        Self {
            build,
            descriptor: settings.devices.descriptor(),
            settings,
            ready: false,
            connected: None,
            secured: false,
            pairing: false,
            passkey_pending: false,
            leds: 0,
            smooth: (false, false),
            boot: false,
            battery: 100,
            targets: Vec::new(),
            max_targets: 0,
            reports: Reports::new(),
            decoder: frame::Decoder::new(Some(MAX_CLIENT_MESSAGE_BYTES)),
            output: VecDeque::new(),
            held: None,
            notices: Bounded::new(),
            session: false,
            status_changed: false,
            action: None,
            #[cfg(test)]
            no_memory: false,
        }
    }

    pub fn name(&self) -> &str {
        self.settings.effective_name(DEFAULT_NAME)
    }

    /// Starts the Bluetooth backend with the saved settings.
    pub fn start(&mut self, radio: &mut impl Radio) {
        let config = radio::Config {
            name: self.settings.effective_name(DEFAULT_NAME),
            devices: self.settings.devices,
            security: self.settings.security,
            battery: self.battery,
            descriptor: &self.descriptor,
            manufacturer: p::USB_MANUFACTURER,
            model: self.build.model,
            firmware: self.build.version,
        };
        if let Err(code) = radio.start(&config) {
            self.radio_event(Event::Failed(code), radio);
        }
    }

    /// The Client opened or closed the serial port. Input from a closed session is released, so a
    /// Client that quits cannot leave keys held on the Target. Unsent output and events are
    /// discarded, so a new session starts with nothing waiting.
    pub fn session(&mut self, open: bool) {
        self.decoder.reset();
        self.output.clear();
        self.held = None;
        self.notices.clear();
        self.session = open;
        self.reports.release_all();
        self.status_changed = false;
    }

    /// Releases every held key and button, as when the Client's USB bus suspends.
    pub fn release(&mut self) {
        self.reports.release_all();
    }

    /// Bytes received from the Client. Returns how many were taken: taking stops while
    /// [`OUTPUT_BYTES`] wait for the Client, or a Response or one-shot event waits to be queued, so
    /// no Response passes an earlier event. The caller offers the rest again once output has been
    /// sent.
    pub fn receive(
        &mut self,
        bytes: &[u8],
        radio: &mut impl Radio,
        store: &mut impl Store,
        now: u64,
    ) -> usize {
        self.queue_held();
        self.write_notices();
        for (taken, &byte) in bytes.iter().enumerate() {
            if self.output.len() >= OUTPUT_BYTES || self.held.is_some() || !self.notices.is_empty()
            {
                return taken;
            }
            let frame = match self.decoder.push(byte) {
                None => continue,
                Some(Ok(frame)) => Some(Vec::from(frame)),
                Some(Err(_)) => None,
            };
            match frame.and_then(|f| ClientMessage::decode(f.as_slice()).ok()) {
                Some(ClientMessage {
                    kind: Some(ClientKind::Input(input)),
                }) => self.input(input),
                Some(ClientMessage {
                    kind: Some(ClientKind::Request(request)),
                }) => {
                    let result = self.dispatch(request.command, radio, store, now);
                    self.respond(p::Response {
                        id: request.id,
                        result,
                    });
                }
                _ => self.respond(p::Response {
                    id: 0,
                    result: Some(error(ErrorCode::InvalidRequest)),
                }),
            }
        }
        bytes.len()
    }

    fn input(&mut self, input: p::Input) {
        let devices = self.settings.devices;
        match input.kind {
            Some(InputKind::Keyboard(state)) if devices.keyboard => {
                self.reports.keyboard(state.modifiers as u8, &state.keys);
            }
            Some(InputKind::Consumer(state)) if devices.keyboard => {
                self.reports.consumer(&state.usages);
            }
            Some(InputKind::Mouse(m)) if devices.mouse => {
                self.reports
                    .mouse(m.buttons as u8, m.x, m.y, m.wheel, m.pan);
            }
            _ => {}
        }
    }

    fn dispatch(
        &mut self,
        command: Option<Command>,
        radio: &mut impl Radio,
        store: &mut impl Store,
        now: u64,
    ) -> Option<Outcome> {
        // A command newer than this firmware decodes as none.
        let Some(command) = command else {
            return Some(error(ErrorCode::UnknownCommand));
        };
        let needs_radio = matches!(
            command,
            Command::StartPairing(_)
                | Command::StopPairing(_)
                | Command::EnterPasskey(_)
                | Command::Disconnect(_)
                | Command::ForgetTarget(_)
        );
        if needs_radio && !self.ready {
            return Some(error(ErrorCode::NotReady));
        }
        match command {
            Command::GetStatus(_) => Some(Outcome::Status(self.status())),
            Command::SetConfig(change) => Some(self.configure(change, radio, store, now)),
            Command::StartPairing(_) => {
                self.set_pairing(true, radio);
                None
            }
            Command::StopPairing(_) => {
                self.set_pairing(false, radio);
                None
            }
            Command::EnterPasskey(answer) => {
                if !self.passkey_pending {
                    return Some(error(ErrorCode::NotFound));
                }
                if answer.passkey.is_some_and(|v| v > 999_999) {
                    return Some(error(ErrorCode::InvalidValue));
                }
                self.passkey_pending = false;
                radio.passkey(answer.passkey).err().map(error)
            }
            Command::Disconnect(_) => {
                if self.connected.is_none() {
                    return Some(error(ErrorCode::NotFound));
                }
                radio.disconnect().err().map(error)
            }
            Command::ForgetTarget(forget) => {
                let Ok(address) = <[u8; 6]>::try_from(forget.address.as_slice()) else {
                    return Some(error(ErrorCode::InvalidValue));
                };
                let target = Target {
                    address,
                    random: forget.address_type == p::AddressType::Random as i32,
                };
                if !self.targets.contains(&target) {
                    return Some(error(ErrorCode::NotFound));
                }
                let result = radio.forget(target);
                self.refresh_targets(radio);
                result.err().map(error)
            }
            Command::SetBattery(battery) => {
                let Ok(level) = u8::try_from(battery.level) else {
                    return Some(error(ErrorCode::InvalidValue));
                };
                if level > 100 {
                    return Some(error(ErrorCode::InvalidValue));
                }
                if level != self.battery {
                    self.battery = level;
                    radio.set_battery(level);
                    self.status_changed = true;
                }
                None
            }
            Command::Restart(_) => {
                self.action = Some((Action::Restart, now + ACTION_GRACE_MS));
                None
            }
            Command::EnterBootloader(_) => {
                if !self.build.development {
                    return Some(error(ErrorCode::Unsupported));
                }
                self.action = Some((Action::Bootloader, now + ACTION_GRACE_MS));
                None
            }
        }
    }

    fn configure(
        &mut self,
        change: p::SetConfig,
        radio: &mut impl Radio,
        store: &mut impl Store,
        now: u64,
    ) -> Outcome {
        let mut next = self.settings.clone();
        if let Some(name) = change.name {
            if !name.is_empty() && !valid_name(&name) {
                return error(ErrorCode::InvalidValue);
            }
            next.name = name;
        }
        next.devices = Devices {
            keyboard: change.keyboard.unwrap_or(next.devices.keyboard),
            mouse: change.mouse.unwrap_or(next.devices.mouse),
        };
        if !next.devices.keyboard && !next.devices.mouse {
            return error(ErrorCode::InvalidValue);
        }
        if let Some(security) = change.security {
            match p::Security::try_from(security) {
                Ok(security) => next.security = security,
                Err(_) => return error(ErrorCode::InvalidValue),
            }
        }
        if next == self.settings {
            return Outcome::Status(self.status());
        }
        if next.save(store).is_err() {
            // Storage that did not open, such as flash holding another layout, is not ready.
            return error(if store.opening().is_err() {
                ErrorCode::NotReady
            } else {
                ErrorCode::StorageFailed
            });
        }
        let previous = core::mem::replace(&mut self.settings, next);
        if previous.devices != self.settings.devices {
            // A different combination is a different Bluetooth identity with its own report map.
            // Restart so that it starts cleanly.
            self.action = Some((Action::Restart, now + ACTION_GRACE_MS));
        } else {
            if previous.name != self.settings.name {
                let _ = radio.set_name(self.settings.effective_name(DEFAULT_NAME));
            }
            if previous.security != self.settings.security {
                radio.set_security(self.settings.security);
            }
        }
        self.status_changed = true;
        Outcome::Status(self.status())
    }

    fn set_pairing(&mut self, enabled: bool, radio: &mut impl Radio) {
        if self.pairing != enabled {
            self.pairing = enabled;
            radio.set_pairing(enabled);
            self.status_changed = true;
        }
    }

    fn refresh_targets(&mut self, radio: &mut impl Radio) {
        let targets = radio.targets();
        if targets != self.targets {
            self.targets = targets;
            self.status_changed = true;
        }
    }

    /// Applies one event from the Bluetooth backend.
    pub fn radio_event(&mut self, event: Event, radio: &mut impl Radio) {
        match event {
            Event::Ready => {
                self.ready = true;
                self.max_targets = radio.max_targets();
                self.refresh_targets(radio);
                // With nothing paired, the adapter waits to be paired.
                let pairing = self.targets.is_empty();
                self.pairing = !pairing;
                self.set_pairing(pairing, radio);
            }
            Event::Failed(_) => {
                self.ready = false;
                self.connected = None;
                self.secured = false;
                self.reports.reset();
            }
            Event::Connected(target) => {
                self.connected = Some(target);
                self.secured = false;
                self.leds = 0;
                self.smooth = (false, false);
                self.boot = false;
                self.reports.reset();
            }
            Event::Identified(target) => {
                if self.connected.is_some() {
                    self.connected = Some(target);
                }
            }
            Event::Disconnected => {
                self.connected = None;
                self.secured = false;
                self.passkey_pending = false;
                // The prompt closed before the Client saw it.
                self.notices.retain(|n| *n != Notice::PasskeyRequest);
                self.leds = 0;
                self.smooth = (false, false);
                self.reports.reset();
            }
            Event::Secured => {
                self.secured = true;
                self.refresh_targets(radio);
            }
            Event::PasskeyRequest => {
                self.passkey_pending = true;
                self.notify(Notice::PasskeyRequest);
            }
            Event::PairingComplete { success } => {
                self.passkey_pending = false;
                self.notify(Notice::PairingResult(success));
                if success {
                    self.set_pairing(false, radio);
                }
                self.refresh_targets(radio);
            }
            Event::Output { report_id, data } => {
                if matches!(report_id, 0 | hid::KEYBOARD_ID)
                    && let Some(&leds) = data.first()
                {
                    self.leds = leds & 0x1f;
                }
            }
            Event::Feature { report_id, data } => {
                if report_id == hid::MOUSE_ID {
                    self.smooth = hid::multipliers(&data);
                    self.reports
                        .set_high_resolution(self.smooth.0, self.smooth.1);
                }
            }
            Event::ProtocolMode { boot } => {
                self.boot = boot;
                return;
            }
            Event::TargetsChanged => {
                self.refresh_targets(radio);
                return;
            }
            Event::CanSend => return,
        }
        self.status_changed = true;
    }

    /// Sends queued reports while the backend accepts them and, once earlier output has been taken,
    /// queues a Status event after any change. Returns work for the platform once its Response has
    /// had time to reach the Client.
    pub fn poll(&mut self, radio: &mut impl Radio, now: u64) -> Option<Action> {
        if self.connected.is_none() || !self.secured {
            while self.reports.pop().is_some() {}
        }
        while let Some(report) = self.reports.front() {
            let Some(encoded) = report.encode(self.boot) else {
                self.reports.pop();
                continue;
            };
            if !radio.can_send() {
                break;
            }
            // A report the backend refuses is dropped: later reports carry the current state.
            let _ = radio.send(&encoded);
            self.reports.pop();
        }
        self.queue_held();
        if let Some((action, deadline)) = self.action
            && ((self.output.is_empty() && self.held.is_none()) || now >= deadline)
        {
            self.action = None;
            return Some(action);
        }
        self.write_notices();
        if self.status_changed
            && self.output.is_empty()
            && self.held.is_none()
            && self.notices.is_empty()
        {
            let status = self.status();
            if self.queue(event(EventKind::Status(status))).is_ok() {
                self.status_changed = false;
            }
        }
        None
    }

    /// Moves up to `out.len()` bytes of pending output into `out`.
    pub fn output(&mut self, out: &mut [u8]) -> usize {
        let count = out.len().min(self.output.len());
        for (slot, byte) in out.iter_mut().zip(self.output.drain(..count)) {
            *slot = byte;
        }
        count
    }

    pub fn has_output(&self) -> bool {
        !self.output.is_empty()
    }

    /// Appends `bytes` to the output, or gives them back when memory for them cannot be reserved.
    fn queue(&mut self, bytes: Vec<u8>) -> Result<(), Vec<u8>> {
        #[cfg(test)]
        if self.no_memory {
            return Err(bytes);
        }
        match self.output.try_reserve(bytes.len()) {
            Ok(()) => {
                self.output.extend(bytes);
                Ok(())
            }
            Err(_) => Err(bytes),
        }
    }

    /// Queues the Response to the request just read. One that cannot be queued yet is held, and
    /// reading waits until it is.
    fn respond(&mut self, response: p::Response) {
        if !self.session {
            return;
        }
        let mut bytes = Vec::new();
        frame::encode(
            &DongleMessage {
                kind: Some(DongleKind::Response(response)),
            },
            &mut bytes,
        );
        self.held = self.queue(bytes).err();
    }

    fn queue_held(&mut self) {
        if let Some(bytes) = self.held.take() {
            self.held = self.queue(bytes).err();
        }
    }

    /// Keeps a one-shot event for the Client and writes it once the output has room.
    fn notify(&mut self, notice: Notice) {
        if !self.session {
            return;
        }
        match notice {
            Notice::PasskeyRequest => self.notices.retain(|n| *n != Notice::PasskeyRequest),
            // A result closes the prompt and is the latest outcome.
            Notice::PairingResult(_) => self.notices.clear(),
        }
        // At most one of each kind waits, so there is room.
        let _ = self.notices.push(notice);
        self.write_notices();
    }

    /// Writes waiting one-shot events, oldest first, while fewer than [`OUTPUT_BYTES`] wait and no
    /// Response is held. One that cannot be queued stays waiting.
    fn write_notices(&mut self) {
        while let Some(&notice) = self.notices.first()
            && self.output.len() < OUTPUT_BYTES
            && self.held.is_none()
        {
            let kind = match notice {
                Notice::PasskeyRequest => EventKind::PasskeyRequest(p::PasskeyRequest {}),
                Notice::PairingResult(success) => {
                    EventKind::PairingResult(p::PairingResult { success })
                }
            };
            if self.queue(event(kind)).is_err() {
                return;
            }
            self.notices.remove(0);
        }
    }

    pub fn status(&self) -> p::Status {
        let target = |t: &Target| p::Target {
            address: t.address.to_vec(),
            address_type: if t.random {
                p::AddressType::Random
            } else {
                p::AddressType::Public
            }
            .into(),
        };
        let link = if !self.ready {
            p::LinkState::Off
        } else if self.connected.is_some() {
            p::LinkState::Connected
        } else if self.pairing {
            p::LinkState::Pairing
        } else {
            p::LinkState::Waiting
        };
        p::Status {
            id: self.build.adapter_id.clone(),
            firmware_version: self.build.version.into(),
            board: self.build.board.into(),
            development: self.build.development,
            config: Some(p::Config {
                name: self.name().into(),
                keyboard: self.settings.devices.keyboard,
                mouse: self.settings.devices.mouse,
                security: self.settings.security.into(),
            }),
            ready: self.ready,
            link: link.into(),
            connected: self.connected.as_ref().map(target),
            targets: self.targets.iter().map(target).collect(),
            leds: self.leds.into(),
            smooth_scrolling: self.smooth.0,
            battery: self.battery.into(),
            max_targets: self.max_targets,
        }
    }

    /// Whether the connected Target is ready for input.
    pub fn connected(&self) -> bool {
        self.connected.is_some() && self.secured
    }
    pub fn pairing(&self) -> bool {
        self.pairing
    }
    pub fn ready(&self) -> bool {
        self.ready
    }
}
