//! Bluetooth LE HID peripheral on BTstack. The C adapter in `c/` owns every vendor call; this crate
//! exchanges HCI packets with the board's controller task and implements the core radio trait.
#![no_std]
extern crate alloc;
#[cfg(test)]
extern crate std;

pub mod transport;

#[cfg(feature = "ffi")]
pub mod backend;
#[cfg(feature = "ffi")]
pub mod ffi;
#[cfg(feature = "ffi")]
pub mod storage;
