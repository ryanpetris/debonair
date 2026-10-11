//! ABI declarations for the C adapter. Vendor structs stay behind this boundary.
use core::ffi::{c_char, c_int, c_void};
use debonair_core::bonds::Native;

#[repr(C)]
pub struct Event {
    pub kind: u8,
    pub code: u8,
    pub report_id: u8,
    /// For a successful pairing, the bond table entry that holds it, or -1 for none.
    pub entry: i8,
    pub address: [u8; 6],
    pub length: u16,
    pub data: *const u8,
}
pub type Emit = unsafe extern "C" fn(*mut c_void, *const Event);

pub const EVENT_READY: u8 = 1;
pub const EVENT_CONNECTED: u8 = 2;
pub const EVENT_IDENTIFIED: u8 = 3;
pub const EVENT_DISCONNECTED: u8 = 4;
pub const EVENT_SECURED: u8 = 5;
pub const EVENT_PASSKEY_REQUEST: u8 = 6;
pub const EVENT_PAIRING_COMPLETE: u8 = 7;
pub const EVENT_OUTPUT: u8 = 8;
pub const EVENT_FEATURE: u8 = 9;
pub const EVENT_PROTOCOL_MODE: u8 = 10;
pub const EVENT_TARGETS_CHANGED: u8 = 11;
pub const EVENT_CAN_SEND: u8 = 12;

pub const KEYBOARD: u8 = 1;
pub const MOUSE: u8 = 2;
pub const CHANNEL_INPUT: u8 = 0;
pub const CHANNEL_BOOT_KEYBOARD: u8 = 1;
pub const CHANNEL_BOOT_MOUSE: u8 = 2;

#[repr(C)]
pub struct Config {
    pub name: *const c_char,
    pub name_length: u16,
    pub devices: u8,
    pub passkey: u8,
    pub battery: u8,
    pub descriptor: *const u8,
    pub descriptor_length: u16,
    pub manufacturer: *const c_char,
    pub model: *const c_char,
    pub firmware: *const c_char,
    pub vendor_id: u16,
    pub product_id: u16,
    pub identity: [u8; 6],
}

#[repr(C)]
pub struct RuntimeCallbacks {
    pub context: *mut c_void,
    pub time_ms: unsafe extern "C" fn(*mut c_void) -> u32,
    pub wake: unsafe extern "C" fn(*mut c_void),
    pub can_send: unsafe extern "C" fn(*mut c_void) -> c_int,
    pub send: unsafe extern "C" fn(*mut c_void, u8, *const u8, u16) -> c_int,
    pub fatal: unsafe extern "C" fn(*mut c_void),
}

/// BTstack's `btstack_chipset_t`.
#[repr(C)]
pub struct Chipset {
    pub name: *const c_char,
    pub init: Option<unsafe extern "C" fn(*const c_void)>,
    pub next_command: Option<unsafe extern "C" fn(*mut u8) -> c_int>,
    pub set_baudrate: Option<unsafe extern "C" fn(u32, *mut u8)>,
    pub set_address: Option<unsafe extern "C" fn(*const u8, *mut u8)>,
}

unsafe extern "C" {
    pub fn debonair_runtime_init(callbacks: *const RuntimeCallbacks, chipset: *const Chipset);
    pub fn debonair_runtime_poll();
    pub fn debonair_runtime_timeout() -> i32;
    pub fn debonair_runtime_receive(kind: u8, data: *mut u8, size: u16);
    pub fn debonair_runtime_sent();

    pub fn debonair_bonds_reset(ir: *const u8, er: *const u8);
    pub fn debonair_bonds_import(bond: *const Native) -> c_int;
    pub fn debonair_bonds_export(slot: c_int, out: *mut Native) -> c_int;
    pub fn debonair_bonds_take(added: *mut u32, updated: *mut u32) -> u32;
    pub fn debonair_bonds_take_roots(ir: *mut u8, er: *mut u8) -> c_int;

    pub fn debonair_hid_start(config: *const Config, context: *mut c_void, emit: Emit) -> c_int;
    pub fn debonair_hid_set_name(name: *const c_char, length: u16) -> c_int;
    pub fn debonair_hid_set_security(passkey: u8);
    pub fn debonair_hid_set_pairing(enabled: bool);
    pub fn debonair_hid_passkey(accept: c_int, passkey: u32) -> c_int;
    pub fn debonair_hid_disconnect() -> c_int;
    pub fn debonair_hid_targets(out: *mut u8, capacity: c_int) -> c_int;
    pub fn debonair_hid_max_targets() -> c_int;
    pub fn debonair_hid_forget(address_type: u8, address: *const u8) -> c_int;
    pub fn debonair_hid_set_battery(level: u8);
    pub fn debonair_hid_can_send() -> c_int;
    pub fn debonair_hid_send(channel: u8, report_id: u8, data: *const u8, length: u16) -> c_int;
}
