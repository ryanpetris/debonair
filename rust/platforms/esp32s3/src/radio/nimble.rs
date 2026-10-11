//! ESP-IDF's NimBLE host and controller.
use crate::Store;
use debonair_core::store::Shared;
use debonair_esp32s3::nimble::{self, Nimble};

pub struct Radio {
    pub backend: Nimble<Store>,
}

pub fn spawn(_: embassy_executor::Spawner) {}

pub fn new(store: &'static Shared<Store>, unique: [u8; 8], random: fn() -> u64) -> Radio {
    Radio {
        backend: Nimble::new(store, unique, random),
    }
}

/// NimBLE starts its controller together with the host.
pub fn prepare() -> bool {
    true
}

impl Radio {
    pub fn poll(&mut self) {}
    pub async fn changed(&self) {
        nimble::changed().await;
    }
}
