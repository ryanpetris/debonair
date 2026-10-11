//! The core radio trait on BTstack. C callbacks copy their data before returning to the stack.
use crate::{ffi, storage::Bonds, transport::Io};
use alloc::{collections::VecDeque, vec::Vec};
use core::{
    cell::{Cell, RefCell},
    ffi::{c_char, c_int, c_void},
};
use debonair_core::{
    ErrorCode,
    hid::{Channel, Encoded},
    protocol::{USB_PRODUCT_ID, USB_VENDOR_ID},
    radio::{self, Config, Event, Radio, Security, Target},
    store::{self, Shared, Store},
};
use embassy_sync::{blocking_mutex::raw::NoopRawMutex, signal::Signal};
use heapless::Vec as Bounded;

const EVENTS: usize = 32;

/// Only the owner task calls into C. The controller task exchanges packets through Io and never
/// invokes BTstack or touches this state.
pub struct State {
    io: &'static Io,
    now: fn() -> u64,
    fatal: fn() -> !,
    wake: Signal<NoopRawMutex, ()>,
    events: RefCell<VecDeque<Event>>,
    fault: Cell<bool>,
    /// The bond table entry of a pairing that succeeded since the last poll, which counts once it
    /// is saved.
    paired: Cell<Option<usize>>,
}

impl State {
    pub fn new(io: &'static Io, now: fn() -> u64, fatal: fn() -> !) -> Self {
        let mut events = VecDeque::new();
        events.reserve_exact(EVENTS);
        Self {
            io,
            now,
            fatal,
            wake: Signal::new(),
            events: RefCell::new(events),
            fault: Cell::new(false),
            paired: Cell::new(None),
        }
    }
    /// Completes when BTstack or the controller has work for the owner.
    pub async fn changed(&self) {
        embassy_futures::select::select(self.wake.wait(), self.io.wait()).await;
    }
    fn push(&self, event: Event) {
        let mut events = self.events.borrow_mut();
        if events.len() == EVENTS {
            // CanSend repeats when needed; keep lifecycle events instead.
            if let Some(index) = events.iter().position(|e| *e == Event::CanSend) {
                events.remove(index);
            } else {
                events.pop_front();
            }
        }
        events.push_back(event);
    }
    unsafe extern "C" fn time(context: *mut c_void) -> u32 {
        let state = unsafe { &*context.cast::<Self>() };
        (state.now)() as u32
    }
    unsafe extern "C" fn wake(context: *mut c_void) {
        unsafe { &*context.cast::<Self>() }.wake.signal(());
    }
    unsafe extern "C" fn can_send(context: *mut c_void) -> c_int {
        i32::from(unsafe { &*context.cast::<Self>() }.io.can_send())
    }
    unsafe extern "C" fn send(
        context: *mut c_void,
        kind: u8,
        data: *const u8,
        length: u16,
    ) -> c_int {
        let state = unsafe { &*context.cast::<Self>() };
        let bytes = unsafe { core::slice::from_raw_parts(data, length.into()) };
        if state.io.send(kind, bytes) { 0 } else { -1 }
    }
    unsafe extern "C" fn fatal(context: *mut c_void) {
        (unsafe { &*context.cast::<Self>() }.fatal)()
    }
    unsafe extern "C" fn event(context: *mut c_void, raw: *const ffi::Event) {
        let state = unsafe { &*context.cast::<Self>() };
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
        let event = match raw.kind {
            ffi::EVENT_READY => Event::Ready,
            ffi::EVENT_CONNECTED => Event::Connected(target),
            ffi::EVENT_IDENTIFIED => Event::Identified(target),
            ffi::EVENT_DISCONNECTED => Event::Disconnected,
            ffi::EVENT_SECURED => Event::Secured,
            ffi::EVENT_PASSKEY_REQUEST => Event::PasskeyRequest,
            ffi::EVENT_PAIRING_COMPLETE => {
                if raw.code != 0
                    && let Ok(entry) = usize::try_from(raw.entry)
                {
                    state.paired.set(Some(entry));
                }
                Event::PairingComplete {
                    success: raw.code != 0,
                }
            }
            ffi::EVENT_OUTPUT => Event::Output {
                report_id: raw.report_id,
                data: bounded(),
            },
            ffi::EVENT_FEATURE => Event::Feature {
                report_id: raw.report_id,
                data: bounded(),
            },
            ffi::EVENT_PROTOCOL_MODE => Event::ProtocolMode {
                boot: raw.code == 0,
            },
            ffi::EVENT_TARGETS_CHANGED => Event::TargetsChanged,
            ffi::EVENT_CAN_SEND => Event::CanSend,
            _ => return,
        };
        state.push(event);
    }
}

pub struct Backend<S: 'static> {
    state: &'static State,
    bonds: Bonds<S>,
    unique: [u8; 8],
    started: bool,
}

impl<S: Store + 'static> Backend<S> {
    /// Sets up BTstack's run loop and HCI transport. `unique` is the board's unique ID, from which
    /// each device combination's Bluetooth address is derived. `store` holds each combination's
    /// identity roots and bonds.
    ///
    /// # Safety
    /// BTstack is process-global. Call once per boot, on its sole owner; keep the chipset table
    /// valid for that boot.
    pub unsafe fn new(
        state: &'static State,
        store: &'static Shared<S>,
        chipset: Option<&'static ffi::Chipset>,
        unique: [u8; 8],
    ) -> Self {
        let callbacks = ffi::RuntimeCallbacks {
            context: core::ptr::from_ref(state).cast_mut().cast(),
            time_ms: State::time,
            wake: State::wake,
            can_send: State::can_send,
            send: State::send,
            fatal: State::fatal,
        };
        unsafe {
            ffi::debonair_runtime_init(
                &callbacks,
                chipset.map_or(core::ptr::null(), core::ptr::from_ref),
            );
        }
        Self {
            state,
            bonds: Bonds::new(store),
            unique,
            started: false,
        }
    }

    /// Saves every bond change still waiting, as before the board restarts.
    pub fn flush(&mut self) -> Result<(), store::Error> {
        self.bonds.save((self.state.now)(), true)
    }

    /// Exchanges packets with the controller and runs BTstack's timers and callbacks.
    pub fn poll(&mut self) {
        if self.state.io.take_failure() {
            self.state.fault.set(true);
        }
        if let Some(ok) = self.state.io.take_completion() {
            if ok {
                unsafe { ffi::debonair_runtime_sent() };
            } else {
                self.state.fault.set(true);
            }
        }
        while let Some(mut packet) = self.state.io.receive() {
            unsafe {
                ffi::debonair_runtime_receive(packet.kind, packet.incoming_ptr(), packet.len)
            };
        }
        unsafe { ffi::debonair_runtime_poll() };
        // A new pairing is saved before the application hears of it. A failed save is retried.
        let now = (self.state.now)();
        let _ = self.bonds.save(now, false);
        if let Some(slot) = self.state.paired.take()
            && self.bonds.unsaved(slot)
        {
            self.reject(slot, now);
        }
    }

    /// Fails a pairing in entry `slot` that did not reach storage: a pairing that would be lost at
    /// the next restart is not reported as made. Removes it, which disconnects the Target, and
    /// reports the pairing as failed.
    fn reject(&mut self, slot: usize, now: u64) {
        if let Some(target) = self.bonds.target(slot) {
            unsafe { ffi::debonair_hid_forget(u8::from(target.random), target.address.as_ptr()) };
        }
        if let Some(event) = self
            .state
            .events
            .borrow_mut()
            .iter_mut()
            .rev()
            .find(|e| **e == Event::PairingComplete { success: true })
        {
            *event = Event::PairingComplete { success: false };
        }
        let _ = self.bonds.save(now, false);
    }

    /// Milliseconds until BTstack's next timer, or a negative value for none.
    pub fn timeout(&self) -> i32 {
        unsafe { ffi::debonair_runtime_timeout() }
    }

    fn check(result: c_int) -> Result<(), ErrorCode> {
        if result == 0 {
            Ok(())
        } else {
            Err(ErrorCode::RadioFailed)
        }
    }
}

impl<S: Store + 'static> Radio for Backend<S> {
    fn start(&mut self, config: &Config<'_>) -> Result<(), ErrorCode> {
        if self.started {
            return Err(ErrorCode::Busy);
        }
        let name = config.name.as_bytes();
        let terminated = |text: &str| {
            let mut bytes = Vec::with_capacity(text.len() + 1);
            bytes.extend_from_slice(text.as_bytes());
            bytes.push(0);
            bytes
        };
        let manufacturer = terminated(config.manufacturer);
        let model = terminated(config.model);
        let firmware = terminated(config.firmware);
        self.bonds
            .load(config.devices)
            .map_err(|_| ErrorCode::StorageFailed)?;
        let devices = (if config.devices.keyboard {
            ffi::KEYBOARD
        } else {
            0
        }) | (if config.devices.mouse { ffi::MOUSE } else { 0 });
        let raw = ffi::Config {
            name: name.as_ptr().cast::<c_char>(),
            name_length: name.len() as u16,
            devices,
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
        let context = core::ptr::from_ref(self.state).cast_mut().cast();
        // C copies every string and the descriptor before returning.
        Self::check(unsafe { ffi::debonair_hid_start(&raw, context, State::event) })?;
        self.started = true;
        Ok(())
    }
    fn set_name(&mut self, name: &str) -> Result<(), ErrorCode> {
        Self::check(unsafe { ffi::debonair_hid_set_name(name.as_ptr().cast(), name.len() as u16) })
    }
    fn set_security(&mut self, security: Security) {
        unsafe { ffi::debonair_hid_set_security(u8::from(security == Security::Passkey)) };
    }
    fn set_pairing(&mut self, enabled: bool) {
        unsafe { ffi::debonair_hid_set_pairing(enabled) };
    }
    fn passkey(&mut self, passkey: Option<u32>) -> Result<(), ErrorCode> {
        Self::check(unsafe {
            ffi::debonair_hid_passkey(c_int::from(passkey.is_some()), passkey.unwrap_or(0))
        })
    }
    fn disconnect(&mut self) -> Result<(), ErrorCode> {
        Self::check(unsafe { ffi::debonair_hid_disconnect() })
    }
    fn targets(&mut self) -> Vec<Target> {
        let mut raw = [0u8; 7 * 16];
        let count = unsafe { ffi::debonair_hid_targets(raw.as_mut_ptr(), 16) }.max(0) as usize;
        raw.as_chunks::<7>()
            .0
            .iter()
            .take(count)
            .map(|entry| Target {
                address: entry[1..7].try_into().unwrap(),
                random: entry[0] & 1 != 0,
            })
            .collect()
    }
    fn max_targets(&self) -> u32 {
        unsafe { ffi::debonair_hid_max_targets() }.max(0) as u32
    }
    fn forget(&mut self, target: Target) -> Result<(), ErrorCode> {
        let slot = self.bonds.slot(target);
        let held = slot.map(|slot| self.bonds.held(slot)).unwrap_or_default();
        let result =
            unsafe { ffi::debonair_hid_forget(u8::from(target.random), target.address.as_ptr()) };
        if result != 0 {
            return Err(ErrorCode::NotFound);
        }
        // The removal reaches flash before the Client hears it succeeded. Other bonds' saves do not
        // decide the result, and a Target with nothing saved has nothing to remove.
        let _ = self.bonds.save((self.state.now)(), false);
        match slot {
            Some(slot) if !held.is_empty() && self.bonds.holds(slot, &held) => {
                Err(ErrorCode::StorageFailed)
            }
            _ => Ok(()),
        }
    }
    fn set_battery(&mut self, level: u8) {
        unsafe { ffi::debonair_hid_set_battery(level) };
    }
    fn can_send(&mut self) -> bool {
        unsafe { ffi::debonair_hid_can_send() != 0 }
    }
    fn send(&mut self, report: &Encoded) -> Result<(), ErrorCode> {
        let (channel, id) = match report.channel {
            Channel::Input(id) => (ffi::CHANNEL_INPUT, id),
            Channel::BootKeyboard => (ffi::CHANNEL_BOOT_KEYBOARD, 0),
            Channel::BootMouse => (ffi::CHANNEL_BOOT_MOUSE, 0),
        };
        let data = report.data();
        Self::check(unsafe {
            ffi::debonair_hid_send(channel, id, data.as_ptr(), data.len() as u16)
        })
    }
    fn next_event(&mut self) -> Option<Event> {
        if self.state.fault.take() {
            self.state.events.borrow_mut().clear();
            return Some(Event::Failed(ErrorCode::RadioFailed));
        }
        self.state.events.borrow_mut().pop_front()
    }
}
