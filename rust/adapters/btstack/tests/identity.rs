//! A combination that starts for the first time gets identity roots that BTstack's security
//! manager makes from the controller's random numbers, and they are saved.
use core::task::Poll;
use debonair_btstack::{
    backend::{Backend, State},
    transport::{Io, Packet},
};
use debonair_core::{
    bonds::Identity,
    hid::Devices,
    radio::{Config, Event, Radio, Security},
    store::{Error, Shared, Store},
};
use embassy_futures::{block_on, poll_once};
use std::{
    collections::BTreeMap,
    sync::atomic::{AtomicU64, Ordering},
};

#[derive(Default)]
struct Memory(BTreeMap<String, Vec<u8>>);
impl Store for Memory {
    fn load(&mut self, path: &str) -> Result<Option<Vec<u8>>, Error> {
        Ok(self.0.get(path).filter(|v| !v.is_empty()).cloned())
    }
    fn save(&mut self, path: &str, bytes: &[u8]) -> Result<(), Error> {
        self.0.insert(path.into(), bytes.into());
        Ok(())
    }
    fn remove(&mut self, path: &str) -> Result<(), Error> {
        self.0.remove(path);
        Ok(())
    }
    fn list(&mut self, _: &str) -> Result<Vec<String>, Error> {
        Ok(Vec::new())
    }
}

static NOW: AtomicU64 = AtomicU64::new(0);
fn now() -> u64 {
    NOW.load(Ordering::Relaxed)
}
fn fatal() -> ! {
    panic!("BTstack assertion")
}

/// Answers a command, with the return parameters BTstack's startup depends on.
fn command_complete(opcode: u16) -> Vec<u8> {
    let mut parameters = vec![0u8];
    match opcode {
        // Read Local Supported Features: LE supported.
        0x1003 => {
            let mut features = [0u8; 8];
            features[4] = 1 << 6;
            parameters.extend(features);
        }
        // Read Buffer Size and LE Read Buffer Size.
        0x1005 => parameters.extend([0xfd, 0x03, 0x40, 8, 0, 8, 0]),
        0x2002 => parameters.extend([0xfb, 0x00, 8]),
        // LE Rand: the controller's random numbers, which differ on every call.
        0x2018 => {
            let n = NOW.fetch_add(1, Ordering::Relaxed);
            parameters.extend(n.wrapping_mul(0x9e37_79b9_7f4a_7c15).to_le_bytes());
        }
        _ => parameters.extend([0u8; 64]),
    }
    let mut event = vec![0x0e, 0, 1];
    event.extend(opcode.to_le_bytes());
    event.extend(parameters);
    event[1] = (event.len() - 2) as u8;
    event
}

#[test]
fn a_new_combination_saves_the_roots_btstack_makes() {
    let io: &'static Io = Box::leak(Box::new(Io::new()));
    let store: &'static Shared<Memory> = Box::leak(Box::new(Shared::new(Memory::default())));
    let state = Box::leak(Box::new(State::new(io, now, fatal)));
    let mut backend = unsafe { Backend::new(state, store, None, [3; 8]) };
    let devices = Devices {
        keyboard: true,
        mouse: false,
    };
    let descriptor = devices.descriptor();
    backend
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
    let mut ready = false;
    for _ in 0..2000 {
        NOW.fetch_add(1, Ordering::Relaxed);
        backend.poll();
        while let Some(event) = backend.next_event() {
            ready |= event == Event::Ready;
        }
        if let Poll::Ready(outbound) = poll_once(io.next_outbound()) {
            io.finish_outbound(true);
            assert_eq!(outbound.kind, 1);
            let opcode = u16::from_le_bytes([outbound.data()[0], outbound.data()[1]]);
            let mut packet = Packet::empty();
            let event = command_complete(opcode);
            packet.kind = 4;
            packet.len = event.len() as u16;
            packet.data_mut().copy_from_slice(&event);
            block_on(io.received(packet));
        }
    }
    assert!(ready);
    let identity = Identity::load(&mut { store }, devices)
        .unwrap()
        .expect("saved roots");
    assert_ne!(identity.ir, [0; 16]);
    assert_ne!(identity.er, [0; 16]);
    assert_ne!(identity.ir, identity.er);
    assert_eq!(store.borrow().0.len(), 1);
}
