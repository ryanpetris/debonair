//! Board-independent firmware logic. Platforms feed it serial bytes and Bluetooth events and give
//! it a [`radio::Radio`] and a [`store::Store`]; it never touches a vendor API itself.
#![no_std]

extern crate alloc;
#[cfg(test)]
extern crate std;

pub mod app;
#[cfg(test)]
mod app_tests;
pub mod bonds;
pub mod hid;
pub mod radio;
pub mod reports;
pub mod settings;
pub mod store;

pub use debonair_protocol as protocol;
pub use debonair_protocol::ErrorCode;
