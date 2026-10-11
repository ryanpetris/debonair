//! BTstack on the ESP32-S3 controller, through ESP-IDF's VHCI interface.
use crate::{Store, now};
use debonair_btstack::{
    backend::{Backend, State},
    transport::Io,
};
use debonair_core::store::Shared;
use debonair_esp32s3::controller;

static IO: Io = Io::new();

pub struct Radio {
    pub backend: Backend<Store>,
    state: &'static State,
}

fn fatal() -> ! {
    std::process::abort()
}

#[embassy_executor::task]
async fn transport() {
    controller::run(&IO).await;
}

pub fn spawn(spawner: embassy_executor::Spawner) {
    spawner.spawn(transport().unwrap());
}

/// BTstack makes a new combination's identity roots from the controller's random numbers, so it
/// takes no random source.
pub fn new(store: &'static Shared<Store>, unique: [u8; 8], _: fn() -> u64) -> Radio {
    let state = Box::leak(Box::new(State::new(&IO, now, fatal)));
    // This is the only BTstack owner for the lifetime of the board.
    let backend = unsafe { Backend::new(state, store, None, unique) };
    Radio { backend, state }
}

/// Starts the controller. The application then starts the host.
pub fn prepare() -> bool {
    controller::initialize(&IO).is_ok()
}

impl Radio {
    pub fn poll(&mut self) {
        self.backend.poll();
    }
    pub async fn changed(&self) {
        self.state.changed().await;
    }
}
