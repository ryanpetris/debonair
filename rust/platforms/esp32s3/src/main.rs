include!(concat!(env!("OUT_DIR"), "/metadata.rs"));

use debonair_core::{
    app::{Action, App, Build},
    radio::Radio,
    store::{self, Shared},
};
use debonair_esp32s3::{board, indicator::Indicator, runtime, storage, usb};
use debonair_usb::{Buffers, Usb, owner::Owner};
use embassy_time::{Instant, Timer};
use esp_idf_sys as sys;

/// Storage that failed to open, or holds another layout, stays unavailable with that error.
type Store = Result<storage::Storage, store::Error>;
static USB_IO: debonair_usb::Io = debonair_usb::Io::new();

#[cfg(feature = "btstack")]
#[path = "radio/btstack.rs"]
mod radio;
#[cfg(feature = "esp-nimble")]
#[path = "radio/nimble.rs"]
mod radio;

fn now() -> u64 {
    Instant::now().as_millis()
}
fn restart() -> ! {
    unsafe { sys::esp_restart() }
}
/// 64 bits from the hardware random number generator, with an entropy source enabled.
fn random() -> u64 {
    let mut bytes = [0u8; 8];
    unsafe { sys::platform::debonair_esp_random(bytes.as_mut_ptr(), bytes.len()) };
    u64::from_be_bytes(bytes)
}

#[embassy_executor::task]
async fn usb_task(serial: &'static str) {
    let out = Box::leak(Box::new([0u8; 256]));
    let buffers = Box::leak(Box::new(Buffers::new(&USB_IO)));
    Usb::new(
        usb::driver(out).expect("USB peripheral"),
        serial,
        board::DEFAULT_ADAPTER_NAME,
        &USB_IO,
        buffers,
    )
    .run()
    .await;
}

#[embassy_executor::task]
async fn owner_task(store: &'static Shared<Store>, unique: [u8; 8], serial: &'static str) {
    let mut bluetooth = radio::new(store, unique, random);
    let mut files = store;
    let mut app = App::new(
        Build {
            version: FIRMWARE_VERSION,
            board: board::HARDWARE,
            model: board::DEFAULT_ADAPTER_NAME,
            development: cfg!(feature = "development"),
            adapter_id: serial.into(),
        },
        &mut files,
    );
    if radio::prepare() {
        app.start(&mut bluetooth.backend);
    } else {
        app.radio_event(
            debonair_core::radio::Event::Failed(debonair_core::ErrorCode::RadioFailed),
            &mut bluetooth.backend,
        );
    }
    let mut owner = Owner::new(&USB_IO);
    let mut indicator = Indicator::new(board::MCU_LED);
    loop {
        bluetooth.poll();
        while let Some(event) = bluetooth.backend.next_event() {
            app.radio_event(event, &mut bluetooth.backend);
        }
        let action = owner.poll(&mut app, &mut bluetooth.backend, &mut files, now());
        if action.is_some() {
            // Bond changes still waiting are saved before the board restarts.
            let _ = bluetooth.backend.flush();
        }
        match action {
            Some(Action::Restart) => restart(),
            #[cfg(feature = "development")]
            Some(Action::Bootloader) => unsafe { sys::platform::debonair_esp_bootloader() },
            _ => {}
        }
        // Off until Bluetooth starts, on while connected, fast blink while pairing, a short flash
        // every two seconds while waiting for a paired Target.
        let ticks = (now() / 125) as u32;
        let lit = if !app.ready() {
            false
        } else if app.connected() {
            true
        } else if app.pairing() {
            ticks.is_multiple_of(2)
        } else {
            ticks.is_multiple_of(16)
        };
        indicator.set(lit);
        embassy_futures::select::select3(USB_IO.changed(), bluetooth.changed(), Timer::after_millis(1))
            .await;
    }
}

fn main() {
    sys::link_patches();
    let mut unique = [0; 8];
    assert_eq!(
        unsafe {
            sys::esp_efuse_read_field_blob(
                core::ptr::addr_of_mut!(sys::ESP_EFUSE_OPTIONAL_UNIQUE_ID).cast(),
                unique.as_mut_ptr().cast(),
                64,
            )
        },
        sys::ESP_OK
    );
    // Not every chip has the optional unique ID; the factory MAC address is unique too.
    if unique == [0; 8] {
        let mut mac = [0u8; 6];
        assert_eq!(unsafe { sys::esp_efuse_mac_get_default(mac.as_mut_ptr()) }, sys::ESP_OK);
        unique[2..].copy_from_slice(&mac);
    }
    let serial = Box::leak(format!("{:016X}", u64::from_le_bytes(unique)).into_boxed_str());
    let store: &'static Shared<Store> = Box::leak(Box::new(Shared::new(store::opened(
        storage::open(board::STORAGE_IDENTITY, board::STORAGE_START..board::STORAGE_END),
    ))));
    runtime::run(|spawner| {
        spawner.spawn(usb_task(serial).unwrap());
        radio::spawn(spawner);
        spawner.spawn(owner_task(store, unique, serial).unwrap());
    });
}
