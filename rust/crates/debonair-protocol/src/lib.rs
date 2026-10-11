//! The Debonair serial API: protobuf messages generated from `proto/debonair.proto` and the
//! frame encoding used on the USB serial port; and the Dongle's saved records in [`storage`].
//!
//! A Client sends [`ClientMessage`] frames and receives [`DongleMessage`] frames. [`frame`] turns
//! messages into frames and back.
#![no_std]

extern crate alloc;

#[allow(clippy::all, clippy::pedantic, missing_docs)]
mod generated {
    include!(concat!(env!("OUT_DIR"), "/debonair.rs"));
}
pub use generated::*;

/// The records the Dongle saves in its filesystem, generated from `proto/storage.proto`. Each file
/// holds one of these messages.
#[allow(clippy::all, clippy::pedantic, missing_docs)]
pub mod storage {
    include!(concat!(env!("OUT_DIR"), "/debonair.storage.rs"));
}

pub mod frame;

/// USB vendor ID of every Dongle (pid.codes).
pub const USB_VENDOR_ID: u16 = 0x1209;
/// USB product ID of every Dongle. This is a pid.codes test ID.
pub const USB_PRODUCT_ID: u16 = 0x0001;
/// USB manufacturer string of every Dongle.
pub const USB_MANUFACTURER: &str = "Debonair";
/// The longest client message a Dongle accepts, before frame encoding.
pub const MAX_CLIENT_MESSAGE_BYTES: usize = 512;
/// The longest name a Dongle shows to Targets, in UTF-8 bytes.
pub const MAX_NAME_BYTES: usize = 29;
