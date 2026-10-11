//! Runs the C adapter and BTstack against a minimal fake controller: startup, a Target connecting,
//! encryption, and an input report leaving as an ATT notification; and a paired Target's saved bond,
//! which BTstack reads from RAM and whose changes are saved as records.
use core::task::Poll;
use debonair_btstack::{
    backend::{Backend, State},
    transport::{Io, Packet},
};
use debonair_core::{
    ErrorCode,
    bonds::{Bond, Identity, Keys},
    hid::{Devices, Report},
    protocol::storage as saved,
    radio::{Config, Event, Radio, Security, Target},
    store::{self, Error, Shared, Store},
};
use embassy_futures::{block_on, poll_once};
use std::{
    collections::BTreeMap,
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
};

/// Makes every save and removal fail.
static FAIL: AtomicBool = AtomicBool::new(false);
fn failing() -> Result<(), Error> {
    if FAIL.load(Ordering::Relaxed) {
        Err(Error::Io)
    } else {
        Ok(())
    }
}

#[derive(Default)]
struct Memory(BTreeMap<String, Vec<u8>>);
impl Store for Memory {
    fn load(&mut self, path: &str) -> Result<Option<Vec<u8>>, Error> {
        Ok(self.0.get(path).filter(|v| !v.is_empty()).cloned())
    }
    fn save(&mut self, path: &str, bytes: &[u8]) -> Result<(), Error> {
        failing()?;
        self.0.insert(path.into(), bytes.into());
        Ok(())
    }
    fn remove(&mut self, path: &str) -> Result<(), Error> {
        failing()?;
        self.0.remove(path);
        Ok(())
    }
    fn list(&mut self, directory: &str) -> Result<Vec<String>, Error> {
        let prefix = format!("{directory}/");
        let mut names: Vec<String> = self
            .0
            .keys()
            .filter_map(|path| path.strip_prefix(&prefix))
            .map(|rest| rest.split('/').next().unwrap().into())
            .collect();
        names.dedup();
        Ok(names)
    }
}

const BOND: &str = "/combinations/keyboard-mouse/bonds/4/bond.pb";

/// The bond saved for the paired Target at `BOND`.
fn saved_bond(store: &Shared<Memory>) -> Option<Bond> {
    let bytes = store.borrow().0.get(BOND)?.clone();
    let saved: saved::Bond = store::decode(&bytes).unwrap();
    Some(Bond::from_saved(&saved).unwrap())
}

static NOW: AtomicU64 = AtomicU64::new(0);
fn now() -> u64 {
    NOW.load(Ordering::Relaxed)
}
fn fatal() -> ! {
    panic!("BTstack assertion")
}

/// Return parameters for the commands whose results BTstack's startup depends on.
fn command_complete(opcode: u16) -> Vec<u8> {
    let mut parameters = vec![0u8];
    match opcode {
        // Read Local Supported Features: LE supported.
        0x1003 => {
            let mut features = [0u8; 8];
            features[4] = 1 << 6;
            parameters.extend(features);
        }
        // Read Buffer Size: 1021-byte ACL packets, 8 buffers.
        0x1005 => parameters.extend([0xfd, 0x03, 0x40, 8, 0, 8, 0]),
        // Read BD_ADDR.
        0x1009 => parameters.extend([1, 2, 3, 4, 5, 6]),
        // Read Encryption Key Size of connection 0x40: 16 bytes.
        0x1408 => parameters.extend([0x40, 0x00, 16]),
        // LE Read Buffer Size: 251-byte LE ACL packets, 8 buffers.
        0x2002 => parameters.extend([0xfb, 0x00, 8]),
        // LE Rand: key generation rejects all-zero randomness.
        0x2018 => {
            let seed = NOW
                .load(Ordering::Relaxed)
                .wrapping_mul(0x9e37_79b9_7f4a_7c15);
            parameters.extend(seed.to_le_bytes());
        }
        _ => parameters.extend([0u8; 64]),
    }
    let mut event = vec![0x0e, 0, 1];
    event.extend(opcode.to_le_bytes());
    event.extend(parameters);
    event[1] = (event.len() - 2) as u8;
    event
}

fn packet(kind: u8, bytes: &[u8]) -> Packet {
    let mut packet = Packet::empty();
    packet.kind = kind;
    packet.len = bytes.len() as u16;
    packet.data_mut().copy_from_slice(bytes);
    packet
}

struct Harness {
    io: &'static Io,
    backend: Backend<Memory>,
    events: Vec<Event>,
    acl: Vec<Vec<u8>>,
    commands: Vec<u16>,
}
impl Harness {
    /// Runs the stack until it is idle, answering every command.
    fn run(&mut self) {
        for _ in 0..2000 {
            NOW.fetch_add(1, Ordering::Relaxed);
            self.backend.poll();
            while let Some(event) = self.backend.next_event() {
                self.events.push(event);
            }
            let Poll::Ready(outbound) = poll_once(self.io.next_outbound()) else {
                continue;
            };
            self.io.finish_outbound(true);
            match outbound.kind {
                1 => {
                    let opcode = u16::from_le_bytes([outbound.data()[0], outbound.data()[1]]);
                    self.commands.push(opcode);
                    block_on(self.io.received(packet(4, &command_complete(opcode))));
                }
                2 => {
                    let data = outbound.data();
                    // Number Of Completed Packets returns the buffer credit.
                    let completed = [0x13, 5, 1, data[0], data[1] & 0x0f, 1, 0];
                    self.acl.push(data.to_vec());
                    block_on(self.io.received(packet(4, &completed)));
                }
                _ => unreachable!(),
            }
        }
    }
    fn inject(&mut self, event: &[u8]) {
        block_on(self.io.received(packet(4, event)));
        self.run();
    }
    /// Sends an ATT PDU from the Target on connection 0x40 and returns the ATT PDUs sent back.
    fn att(&mut self, pdu: &[u8]) -> Vec<Vec<u8>> {
        let mut acl = vec![0x40, 0x20];
        acl.extend(((pdu.len() + 4) as u16).to_le_bytes());
        acl.extend((pdu.len() as u16).to_le_bytes());
        acl.extend([4, 0]);
        acl.extend(pdu);
        self.acl.clear();
        block_on(self.io.received(packet(2, &acl)));
        self.run();
        self.acl
            .iter()
            .filter(|p| p.len() > 8 && p[6..8] == [4, 0])
            .map(|p| p[8..].to_vec())
            .collect()
    }
}

#[test]
fn starts_connects_and_notifies() {
    let io: &'static Io = Box::leak(Box::new(Io::new()));
    // A Target paired before, with the public address 01:02:03:04:05:06, and a Target paired
    // before it.
    let mut memory = Memory::default();
    let both = Devices {
        keyboard: true,
        mouse: true,
    };
    let identity = Identity::load_or_create(&mut memory, both, &mut || 1).unwrap();
    let paired = Bond {
        address: [1, 2, 3, 4, 5, 6],
        key_size: 16,
        secure_connections: true,
        peer: Some(Keys {
            ltk: Some([7; 16]),
            ..Keys::default()
        }),
        ..Bond::default()
    };
    let older = Bond {
        address: [9; 6],
        random: true,
        ..paired.clone()
    };
    for (path, bond) in [
        (BOND, &paired),
        ("/combinations/keyboard-mouse/bonds/2/bond.pb", &older),
    ] {
        memory
            .0
            .insert(path.into(), store::encode(&bond.to_saved()).unwrap());
    }
    let store: &'static Shared<Memory> = Box::leak(Box::new(Shared::new(memory)));
    let state = Box::leak(Box::new(State::new(io, now, fatal)));
    let backend = unsafe { Backend::new(state, store, None, [9; 8]) };
    let mut harness = Harness {
        io,
        backend,
        events: Vec::new(),
        acl: Vec::new(),
        commands: Vec::new(),
    };
    let devices = Devices {
        keyboard: true,
        mouse: true,
    };
    let descriptor = devices.descriptor();
    harness
        .backend
        .start(&Config {
            name: "Debonair",
            devices,
            security: Security::JustWorks,
            battery: 100,
            descriptor: &descriptor,
            manufacturer: "Debonair",
            model: "Test",
            firmware: "0.0.0",
        })
        .unwrap();
    harness.run();
    assert!(
        harness.events.contains(&Event::Ready),
        "{:?}",
        harness.events
    );
    // The saved bonds were read into BTstack, beside the combination's saved identity.
    assert_eq!(
        Identity::load_or_create(&mut { store }, both, &mut || 2).unwrap(),
        identity
    );
    let paired_target = Target {
        address: [1, 2, 3, 4, 5, 6],
        random: false,
    };
    let older_target = Target {
        address: [9; 6],
        random: true,
    };
    assert_eq!(harness.backend.targets(), [older_target, paired_target]);
    harness.backend.set_pairing(true);
    harness.run();
    // LE Set Random Address, Advertising Parameters, Data and Enable.
    for opcode in [0x2005, 0x2006, 0x2008, 0x200a] {
        assert!(harness.commands.contains(&opcode), "missing {opcode:#x}");
    }

    // LE Connection Complete: handle 0x40, peripheral role, public peer address.
    harness.inject(&[
        0x3e, 19, 0x01, 0x00, 0x40, 0x00, 0x01, 0x00, 6, 5, 4, 3, 2, 1, 0x18, 0, 0, 0, 0x2c, 0x01,
        0x00,
    ]);
    let connected = harness
        .events
        .iter()
        .find_map(|e| match e {
            Event::Connected(t) => Some(*t),
            _ => None,
        })
        .expect("connected");
    assert_eq!(connected.address, [1, 2, 3, 4, 5, 6]);
    assert!(!connected.random);
    // Read Request for the device name, which has the same handle in every database.
    let replies = harness.att(&[0x0a, 0x03, 0x00]);
    assert_eq!(replies, [[&[0x0b][..], b"Debonair"].concat()]);
    harness.backend.set_name("Desk").unwrap();
    let replies = harness.att(&[0x0a, 0x03, 0x00]);
    assert_eq!(replies, [[&[0x0b][..], b"Desk"].concat()]);
    // Discover characteristic declarations as a host does: (UUID, value handle).
    let mut characteristics = Vec::new();
    let mut start = 1u16;
    loop {
        let mut request = vec![0x08];
        request.extend(start.to_le_bytes());
        request.extend([0xff, 0xff, 0x03, 0x28]);
        let reply = harness.att(&request).pop().unwrap();
        if reply[0] != 0x09 {
            break;
        }
        let length = usize::from(reply[1]);
        for entry in reply[2..].chunks(length) {
            let handle = u16::from_le_bytes([entry[0], entry[1]]);
            let value = u16::from_le_bytes([entry[3], entry[4]]);
            let uuid = u16::from_le_bytes([entry[5], entry[6]]);
            characteristics.push((uuid, value));
            start = handle + 1;
        }
    }
    let handle = |uuid: u16| {
        characteristics
            .iter()
            .find(|c| c.0 == uuid)
            .map(|c| c.1)
            .unwrap_or_else(|| panic!("no characteristic {uuid:#x}"))
    };
    let reports = characteristics.iter().filter(|c| c.0 == 0x2a4d).count();
    assert_eq!(reports, 5);
    // The report map is readable without encryption and matches the descriptor.
    let map = handle(0x2a4b);
    let mut request = vec![0x0a];
    request.extend(map.to_le_bytes());
    let reply = harness.att(&request).pop().unwrap();
    assert_eq!(reply[0], 0x0b);
    assert_eq!(&reply[1..], &descriptor[..reply.len() - 1]);
    // Protocol Mode written to boot protocol.
    let mut command = vec![0x52];
    command.extend(handle(0x2a4e).to_le_bytes());
    command.push(0);
    harness.att(&command);
    assert!(harness.events.contains(&Event::ProtocolMode { boot: true }));

    // Nothing can be sent before encryption.
    assert!(!harness.backend.can_send());

    // LE Long Term Key Request for a Secure Connections key: BTstack answers with the saved bond's
    // key, then Encryption Change: success, handle 0x40, enabled.
    harness.commands.clear();
    harness.inject(&[0x3e, 13, 0x05, 0x40, 0x00, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    assert!(
        harness.commands.contains(&0x201a),
        "{:x?}",
        harness.commands
    );
    harness.acl.clear();
    harness.inject(&[0x08, 4, 0x00, 0x40, 0x00, 0x01]);
    assert!(harness.events.contains(&Event::Secured));
    assert!(harness.backend.can_send());

    // The paired Target has not discovered this database, so it is told to discover again; once
    // it confirms, its bond records the database.
    let service_changed = handle(0x2a05);
    let indication = harness
        .acl
        .iter()
        .find(|p| p.len() > 10 && p[6..8] == [4, 0] && p[8] == 0x1d)
        .expect("Service Changed indication");
    assert_eq!(&indication[9..11], service_changed.to_le_bytes());
    assert!(saved_bond(store).unwrap().database.is_empty());
    harness.att(&[0x1e]);
    let bond = saved_bond(store).unwrap();
    assert_eq!(bond.database.len(), 16);
    assert!(bond.subscriptions.is_empty());

    // A subscription to keyboard input is saved with the bond. BTstack puts a characteristic's
    // configuration right after its value.
    let keyboard = handle(0x2a4d);
    let mut request = vec![0x12];
    request.extend((keyboard + 1).to_le_bytes());
    request.extend([1, 0]);
    assert_eq!(harness.att(&request), [[0x13]]);
    let bond = saved_bond(store).unwrap();
    assert_eq!(bond.subscriptions.len(), 1);
    assert_eq!(bond.subscriptions[0].handle, keyboard + 1);
    assert_eq!(bond.subscriptions[0].value, 1);
    // Turning it off removes it.
    request.truncate(3);
    request.extend([0, 0]);
    assert_eq!(harness.att(&request), [[0x13]]);
    assert!(saved_bond(store).unwrap().subscriptions.is_empty());
    // Nothing else was saved: the identity and the two bonds.
    assert_eq!(store.borrow().0.len(), 3);
    harness.acl.clear();
    let report = Report::Keyboard {
        modifiers: 0x02,
        keys: [0x04, 0, 0, 0, 0, 0],
    }
    .encode(false)
    .unwrap();
    harness.backend.send(&report).unwrap();
    harness.run();
    // ACL header (4), L2CAP header (4) with the ATT channel, then a notification.
    let notification = harness
        .acl
        .iter()
        .find(|p| p.len() > 9 && p[6..8] == [4, 0] && p[8] == 0x1b)
        .expect("notification");
    assert_eq!(&notification[11..], report.data());

    // Disconnection Complete.
    harness.inject(&[0x05, 4, 0x00, 0x40, 0x00, 0x13]);
    assert!(harness.events.contains(&Event::Disconnected));
    assert!(!harness.backend.can_send());

    // A Target whose saved bond cannot be removed is gone until the adapter restarts, and the
    // removal is tried again.
    let older_bond = "/combinations/keyboard-mouse/bonds/2/bond.pb";
    FAIL.store(true, Ordering::Relaxed);
    assert_eq!(
        harness.backend.forget(older_target),
        Err(ErrorCode::StorageFailed)
    );
    FAIL.store(false, Ordering::Relaxed);
    assert_eq!(harness.backend.targets(), [paired_target]);
    assert!(store.borrow().0.contains_key(older_bond));
    NOW.fetch_add(5000, Ordering::Relaxed);
    harness.run();
    assert!(!store.borrow().0.contains_key(older_bond));

    // Forgetting a Target removes its saved bond before the Client hears it succeeded.
    harness.backend.forget(paired_target).unwrap();
    assert_eq!(saved_bond(store), None);
    assert!(harness.backend.targets().is_empty());

    // A Target that connects outside pairing and never encrypts is disconnected, so it cannot
    // keep paired Targets out.
    harness.backend.set_pairing(false);
    harness.inject(&[
        0x3e, 19, 0x01, 0x00, 0x41, 0x00, 0x01, 0x00, 6, 5, 4, 3, 2, 9, 0x18, 0, 0, 0, 0x2c, 0x01,
        0x00,
    ]);
    harness.commands.clear();
    for _ in 0..12 {
        harness.run();
    }
    assert!(
        harness.commands.contains(&0x0406),
        "no disconnect: {:x?}",
        harness.commands
    );
}
