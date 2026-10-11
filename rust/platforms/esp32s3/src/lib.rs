#[cfg(feature = "btstack")]
pub mod controller;
pub mod indicator;
#[cfg(feature = "esp-nimble")]
pub mod nimble;
pub mod runtime;
pub mod storage;
pub mod usb;

pub mod board {
    include!(concat!(env!("OUT_DIR"), "/board.rs"));
}
