//! The core radio trait on ESP-IDF's NimBLE host, through `components/platform/nimble_hid.c`.
//!
//! NimBLE runs in its own FreeRTOS task. Commands and reports reach it through queues; its events
//! arrive here on that task and are queued for the owner task, which is woken through `changed`.
use debonair_core::{
    ErrorCode,
    bonds::{self, Bond, Entry, Identity, MAX_BONDS, Native, Table},
    hid::{Channel, Encoded},
    protocol::{USB_PRODUCT_ID, USB_VENDOR_ID},
    radio::{self, Config, Event, Radio, Security, Target},
    store::{self, Shared, Store},
};
use embassy_sync::{
    blocking_mutex::{Mutex, raw::CriticalSectionRawMutex as Raw},
    signal::Signal,
};
use esp_idf_sys::platform as p;
use heapless::Vec as Bounded;
use std::{
    cell::RefCell,
    collections::VecDeque,
    ffi::c_char,
    sync::atomic::{AtomicBool, Ordering},
};

const EVENTS: usize = 32;
// The C side's debonair_bond has the layout of the core's.
const _: () = assert!(size_of::<p::debonair_bond>() == size_of::<Native>());

/// A successful pairing as NimBLE reported it: the Target and the pairing record entry that holds
/// it. The pairing counts once that entry holds it and it is saved.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Paired {
    target: Target,
    slot: usize,
}

/// Events for the owner task, each successful pairing with what confirms it.
type Queue = VecDeque<(Event, Option<Paired>)>;
static QUEUE: Mutex<Raw, RefCell<Queue>> = Mutex::new(RefCell::new(VecDeque::new()));
static CHANGED: Signal<Raw, ()> = Signal::new();
static FAILED: AtomicBool = AtomicBool::new(false);

/// Completes when NimBLE has an event for the owner task.
pub async fn changed() {
    CHANGED.wait().await;
}

/// Runs on the NimBLE host task. Copies the event before returning.
unsafe extern "C" fn emit(raw: *const p::debonair_event) {
    let raw = unsafe { &*raw };
    let data = if raw.length == 0 || raw.data.is_null() {
        &[][..]
    } else {
        unsafe { core::slice::from_raw_parts(raw.data, raw.length.into()) }
    };
    let bounded = || {
        let mut out = Bounded::<u8, 8>::new();
        let _ = out.extend_from_slice(&data[..data.len().min(8)]);
        out
    };
    let target = Target {
        address: raw.address,
        random: raw.code & 1 != 0,
    };
    let mut paired = None;
    let event = match u32::from(raw.kind) {
        p::DEBONAIR_EVENT_READY => Event::Ready,
        p::DEBONAIR_EVENT_CONNECTED => Event::Connected(target),
        p::DEBONAIR_EVENT_IDENTIFIED => Event::Identified(target),
        p::DEBONAIR_EVENT_DISCONNECTED => Event::Disconnected,
        p::DEBONAIR_EVENT_SECURED => Event::Secured,
        p::DEBONAIR_EVENT_PASSKEY_REQUEST => Event::PasskeyRequest,
        p::DEBONAIR_EVENT_PAIRING_COMPLETE => {
            if raw.code != 0 {
                paired = usize::try_from(raw.entry).ok().map(|slot| Paired {
                    target: Target {
                        address: raw.address,
                        random: raw.random != 0,
                    },
                    slot,
                });
            }
            Event::PairingComplete {
                success: raw.code != 0,
            }
        }
        p::DEBONAIR_EVENT_OUTPUT => Event::Output {
            report_id: raw.report_id,
            data: bounded(),
        },
        p::DEBONAIR_EVENT_FEATURE => Event::Feature {
            report_id: raw.report_id,
            data: bounded(),
        },
        p::DEBONAIR_EVENT_PROTOCOL_MODE => Event::ProtocolMode {
            boot: raw.code == 0,
        },
        p::DEBONAIR_EVENT_TARGETS_CHANGED => Event::TargetsChanged,
        p::DEBONAIR_EVENT_CAN_SEND => Event::CanSend,
        // The owner takes the changes when it next polls for events.
        p::DEBONAIR_EVENT_BONDS_CHANGED => {
            CHANGED.signal(());
            return;
        }
        p::DEBONAIR_EVENT_FAILED => {
            FAILED.store(true, Ordering::Release);
            CHANGED.signal(());
            return;
        }
        _ => return,
    };
    QUEUE.lock(|queue| {
        let mut queue = queue.borrow_mut();
        if queue.len() == EVENTS {
            // CanSend repeats when needed; keep lifecycle events instead.
            match queue.iter().position(|(e, _)| *e == Event::CanSend) {
                Some(index) => {
                    queue.remove(index);
                }
                None => {
                    queue.pop_front();
                }
            }
        }
        queue.push_back((event, paired));
    });
    CHANGED.signal(());
}

pub struct Nimble<S: 'static> {
    store: &'static Shared<S>,
    unique: [u8; 8],
    random: fn() -> u64,
    started: bool,
    /// Which saved bond each pairing record holds, once started.
    bonds: Option<Table>,
    /// The sequence of the last FORGET command.
    forgets: u32,
    /// Targets being removed, because they were forgotten or their pairing did not reach storage,
    /// that the host task has not removed yet, with the sequence of the FORGET command the host
    /// task took. Their entries are never saved and they are not listed as paired.
    unpairing: Vec<(Target, Option<u32>)>,
}

fn now() -> u64 {
    embassy_time::Instant::now().as_millis()
}

impl<S: Store + 'static> Nimble<S> {
    /// `unique` is the board's unique ID, from which each device combination's Bluetooth address
    /// is derived. `store` holds each combination's identity roots and bonds; `random` makes the
    /// roots of a combination that has none.
    pub fn new(store: &'static Shared<S>, unique: [u8; 8], random: fn() -> u64) -> Self {
        // Reserved here so that queueing an event on the NimBLE task never allocates.
        QUEUE.lock(|queue| queue.borrow_mut().reserve_exact(EVENTS));
        Self {
            store,
            unique,
            random,
            started: false,
            bonds: None,
            forgets: 0,
            unpairing: Vec::new(),
        }
    }

    /// Saves every bond change still waiting, as before the board restarts.
    pub fn flush(&mut self) -> Result<(), store::Error> {
        self.save_bonds(true)
    }

    fn command(&self, kind: u32, change: impl FnOnce(&mut p::debonair_command)) -> Result<(), ErrorCode> {
        let mut command = p::debonair_command {
            kind: kind as u8,
            ..Default::default()
        };
        change(&mut command);
        if unsafe { p::debonair_nimble_command(&command) } == 0 {
            Ok(())
        } else {
            Err(ErrorCode::Busy)
        }
    }

    /// Loads the identity roots and bonds of the combination into NimBLE's pairing records,
    /// creating the identity the first time the combination starts. Returns the identity.
    fn load_bonds(&mut self, config: &Config<'_>) -> Result<Identity, store::Error> {
        let mut store = self.store;
        let random = self.random;
        let identity = Identity::load_or_create(&mut store, config.devices, &mut || random())?;
        let loaded = bonds::load(&mut store, config.devices)?;
        unsafe { p::debonair_nimble_bonds_reset() };
        let mut table = Table::new(config.devices, loaded.next_id);
        for (id, bond) in &loaded.bonds {
            let native = Native::from(bond);
            let slot = unsafe { p::debonair_nimble_bonds_import((&raw const native).cast()) };
            if let Ok(slot) = usize::try_from(slot) {
                table.assign(slot, *id);
            }
        }
        self.bonds = Some(table);
        Ok(identity)
    }

    /// Saves what NimBLE changed: new pairings, keys and removals at once, other changes once
    /// they pause, and retries a failed save. With `all`, saves everything waiting.
    fn save_bonds(&mut self, all: bool) -> Result<(), store::Error> {
        let Some(table) = &mut self.bonds else {
            return Ok(());
        };
        let now = now();
        let (mut added, mut updated) = (0, 0);
        let changed = unsafe { p::debonair_nimble_bonds_take(&mut added, &mut updated) };
        for slot in 0..MAX_BONDS {
            let bit = 1 << slot;
            if added & bit != 0 {
                table.added(slot, now);
            } else if changed & bit != 0 {
                table.changed(slot, now);
            }
            if updated & bit != 0 {
                table.updated(slot, now);
            }
        }
        if !table.pending() {
            return Ok(());
        }
        let mut store = self.store;
        let unpairing = &self.unpairing;
        // NimBLE writes a pairing's two sides one after the other, and a Target pairing again has
        // its keys deleted one side after the other: such a record is incomplete, and saved once
        // the new keys are all written. A Target being removed is never saved, even before the
        // host task removes it.
        table.save(&mut store, now, all, |slot| match export(slot) {
            Entry::Bond(bond) if unpairing.iter().any(|(t, _)| holds(&bond, *t)) => {
                Entry::Incomplete
            }
            entry => entry,
        })
    }

    /// The pairing record entry that holds a complete pairing with `target`.
    fn slot(target: Target) -> Option<usize> {
        (0..MAX_BONDS)
            .find(|slot| matches!(export(*slot), Entry::Bond(bond) if holds(&bond, target)))
    }

    /// Queues the removal of the pairing with `target` on the host task, which disconnects it.
    /// Returns the command's sequence, which `debonair_nimble_forgotten` reports once done.
    fn send_forget(&mut self, target: Target) -> Result<u32, ErrorCode> {
        self.forgets = self.forgets.wrapping_add(1);
        let sequence = self.forgets;
        self.command(p::DEBONAIR_COMMAND_FORGET, |c| {
            c.value = u8::from(target.random);
            c.address = target.address;
            c.sequence = sequence;
        })?;
        Ok(sequence)
    }

    /// Whether the pairing or keys of entry `slot` have not reached storage.
    fn unsaved(&self, slot: usize) -> bool {
        self.bonds.as_ref().is_some_and(|t| !t.saved(slot))
    }

    /// Checks that a pairing reached storage: a pairing that would be lost at the next restart is
    /// not reported as made. One whose entry no longer holds it complete, or that did not reach
    /// storage, is removed, which disconnects the Target. Returns whether the pairing counts.
    fn confirm(&mut self, paired: Paired) -> bool {
        // A Target removed since the last check may have paired again.
        self.unpair();
        // NimBLE wrote the keys before it reported the pairing.
        let _ = self.save_bonds(false);
        let held =
            matches!(export(paired.slot), Entry::Bond(bond) if holds(&bond, paired.target));
        if held && !self.unsaved(paired.slot) {
            return true;
        }
        self.remove(paired.target);
        false
    }

    /// Stops saving `target` and has the host task remove it, without waiting.
    fn remove(&mut self, target: Target) {
        if !self.unpairing.iter().any(|(t, _)| *t == target) {
            self.unpairing.push((target, None));
        }
        self.unpair();
    }

    /// Removes the Targets being removed without waiting: sends each a FORGET command until the
    /// host task takes one, and drops each once the host task has carried it out.
    fn unpair(&mut self) {
        if self.unpairing.is_empty() {
            return;
        }
        // The host task carries out commands in order, so every command up to the last one it
        // carried out is done.
        let done = unsafe { p::debonair_nimble_forgotten() };
        let before = self.unpairing.len();
        self.unpairing
            .retain(|(_, sent)| sent.is_none_or(|sent| (done.wrapping_sub(sent) as i32) < 0));
        for index in 0..self.unpairing.len() {
            let (target, sent) = self.unpairing[index];
            if sent.is_none() {
                self.unpairing[index].1 = self.send_forget(target).ok();
            }
        }
        if self.unpairing.len() < before {
            let _ = self.save_bonds(false);
        }
    }
}

/// What pairing record entry `slot` holds.
fn export(slot: usize) -> Entry {
    let mut native = Native::default();
    match unsafe { p::debonair_nimble_bonds_export(slot as i32, (&raw mut native).cast()) } {
        0 => Entry::Empty,
        1 => Entry::Bond(Bond::from(&native)),
        _ => Entry::Incomplete,
    }
}

/// Whether `bond` is a pairing with `target`.
fn holds(bond: &Bond, target: Target) -> bool {
    bond.address == target.address && bond.random == target.random
}

impl<S: Store + 'static> Radio for Nimble<S> {
    fn start(&mut self, config: &Config<'_>) -> Result<(), ErrorCode> {
        if self.started {
            return Err(ErrorCode::Busy);
        }
        let identity = self
            .load_bonds(config)
            .map_err(|_| ErrorCode::StorageFailed)?;
        let terminated = |text: &str| {
            let mut bytes = Vec::with_capacity(text.len() + 1);
            bytes.extend_from_slice(text.as_bytes());
            bytes.push(0);
            bytes
        };
        let manufacturer = terminated(config.manufacturer);
        let model = terminated(config.model);
        let firmware = terminated(config.firmware);
        let devices = (if config.devices.keyboard {
            p::DEBONAIR_KEYBOARD
        } else {
            0
        }) | (if config.devices.mouse {
            p::DEBONAIR_MOUSE
        } else {
            0
        });
        let raw = p::debonair_hid_config {
            name: config.name.as_ptr().cast(),
            name_length: config.name.len() as u16,
            devices: devices as u8,
            passkey: u8::from(config.security == Security::Passkey),
            battery: config.battery,
            descriptor: config.descriptor.as_ptr(),
            descriptor_length: config.descriptor.len() as u16,
            manufacturer: manufacturer.as_ptr().cast(),
            model: model.as_ptr().cast(),
            firmware: firmware.as_ptr().cast(),
            vendor_id: USB_VENDOR_ID,
            product_id: USB_PRODUCT_ID,
            identity: radio::static_address(&self.unique, config.devices),
        };
        // C copies every string, the descriptor and the key before returning.
        let result =
            unsafe { p::debonair_nimble_start(&raw, identity.irk.as_ptr(), Some(emit)) };
        if result != 0 {
            return Err(ErrorCode::RadioFailed);
        }
        self.started = true;
        Ok(())
    }
    fn set_name(&mut self, name: &str) -> Result<(), ErrorCode> {
        if !self.started {
            // The name is passed to start.
            return Ok(());
        }
        let bytes = name.as_bytes();
        self.command(p::DEBONAIR_COMMAND_NAME, |c| {
            let length = bytes.len().min(c.name.len());
            for (to, from) in c.name.iter_mut().zip(&bytes[..length]) {
                *to = *from as c_char;
            }
            c.name_length = length as u8;
        })
    }
    fn set_security(&mut self, security: Security) {
        let _ = self.command(p::DEBONAIR_COMMAND_SECURITY, |c| {
            c.value = u8::from(security == Security::Passkey)
        });
    }
    fn set_pairing(&mut self, enabled: bool) {
        let _ = self.command(p::DEBONAIR_COMMAND_PAIRING, |c| c.value = u8::from(enabled));
    }
    fn passkey(&mut self, passkey: Option<u32>) -> Result<(), ErrorCode> {
        self.command(p::DEBONAIR_COMMAND_PASSKEY, |c| {
            c.value = u8::from(passkey.is_some());
            c.passkey = passkey.unwrap_or(0);
        })
    }
    fn disconnect(&mut self) -> Result<(), ErrorCode> {
        self.command(p::DEBONAIR_COMMAND_DISCONNECT, |_| {})
    }
    fn targets(&mut self) -> Vec<Target> {
        let mut raw = [0u8; 7 * 16];
        let count = unsafe { p::debonair_nimble_targets(raw.as_mut_ptr(), 16) }.max(0) as usize;
        raw.as_chunks::<7>()
            .0
            .iter()
            .take(count)
            .map(|entry| Target {
                address: entry[1..7].try_into().unwrap(),
                random: entry[0] & 1 != 0,
            })
            .filter(|target| !self.unpairing.iter().any(|(t, _)| t == target))
            .collect()
    }
    fn max_targets(&self) -> u32 {
        unsafe { p::debonair_nimble_max_targets() }.max(0) as u32
    }
    fn forget(&mut self, target: Target) -> Result<(), ErrorCode> {
        let Some(slot) = Self::slot(target) else {
            return Err(ErrorCode::NotFound);
        };
        // The saved pairing is removed first, here, so the response tells whether it left flash.
        // The host task then removes the Target, which disconnects it, without the owner waiting;
        // until it has, the Target is neither saved nor listed. Other bonds' saves do not decide
        // the result.
        let held = self
            .bonds
            .as_mut()
            .map(|t| {
                let held = t.held(slot);
                t.forget(slot);
                held
            })
            .unwrap_or_default();
        self.remove(target);
        let _ = self.save_bonds(false);
        if self.bonds.as_ref().is_some_and(|t| t.removing(&held)) {
            return Err(ErrorCode::StorageFailed);
        }
        Ok(())
    }
    fn set_battery(&mut self, level: u8) {
        let _ = self.command(p::DEBONAIR_COMMAND_BATTERY, |c| c.value = level);
    }
    fn can_send(&mut self) -> bool {
        unsafe { p::debonair_nimble_can_send() != 0 }
    }
    fn send(&mut self, report: &Encoded) -> Result<(), ErrorCode> {
        let (channel, id) = match report.channel {
            Channel::Input(id) => (p::DEBONAIR_CHANNEL_INPUT, id),
            Channel::BootKeyboard => (p::DEBONAIR_CHANNEL_BOOT_KEYBOARD, 0),
            Channel::BootMouse => (p::DEBONAIR_CHANNEL_BOOT_MOUSE, 0),
        };
        let data = report.data();
        let result = unsafe {
            p::debonair_nimble_send(channel as u8, id, data.as_ptr(), data.len() as u16)
        };
        if result == 0 {
            Ok(())
        } else {
            Err(ErrorCode::RadioFailed)
        }
    }
    fn next_event(&mut self) -> Option<Event> {
        // A new pairing is saved before the application hears of it. A failed save is retried.
        let _ = self.save_bonds(false);
        if FAILED.swap(false, Ordering::AcqRel) {
            QUEUE.lock(|queue| queue.borrow_mut().clear());
            return Some(Event::Failed(ErrorCode::RadioFailed));
        }
        self.unpair();
        let (event, paired) = QUEUE.lock(|queue| queue.borrow_mut().pop_front())?;
        if event == (Event::PairingComplete { success: true }) {
            // A success without its confirmation does not count either.
            let success = paired.is_some_and(|paired| self.confirm(paired));
            return Some(Event::PairingComplete { success });
        }
        Some(event)
    }
}
