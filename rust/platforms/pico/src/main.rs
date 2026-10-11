#![no_std]
#![no_main]

extern crate alloc;

include!(concat!(env!("OUT_DIR"), "/metadata.rs"));

use alloc::boxed::Box;
use debonair_btstack::{
    backend::{Backend, State},
    transport::Io,
};
use debonair_core::{
    app::{Action, App, Build},
    store::{self, Shared},
};
use debonair_pico::{DmaIrqs, board, radio, services, usb};
use debonair_usb::{Buffers, Usb, owner::Owner};
use embassy_executor::Spawner;
use embassy_futures::select::{Either, select, select3};
use embassy_rp::flash::{Async, Flash};
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, signal::Signal};
use embassy_time::{Instant, Timer};

static USB_IO: debonair_usb::Io = debonair_usb::Io::new();
static RADIO_IO: Io = Io::new();
static RADIO_STARTED: Signal<CriticalSectionRawMutex, ()> = Signal::new();
static LED: Signal<CriticalSectionRawMutex, Led> = Signal::new();

/// The LittleFS blocks after the storage layout's guard block.
const STORAGE_BLOCKS: usize = ((board::STORAGE_END - board::STORAGE_START) / 4096 - 1) as usize;
type Fs = debonair_storage::Storage<
    Flash<'static, embassy_rp::peripherals::FLASH, Async, { board::FLASH_BYTES }>,
    STORAGE_BLOCKS,
>;
/// Storage that failed to open, or holds another layout, stays unavailable with that error.
type Store = Result<Fs, store::Error>;

/// What the status LED shows.
#[derive(Clone, Copy, Eq, PartialEq)]
enum Led {
    /// Bluetooth has not started.
    Off,
    /// Waiting for a paired Target.
    Slow,
    /// Pairing.
    Fast,
    /// A Target is connected.
    On,
}

fn now() -> u64 {
    Instant::now().as_millis()
}
fn fatal() -> ! {
    loop {
        core::hint::spin_loop();
    }
}
#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
    fatal()
}

#[embassy_executor::task]
async fn usb_task(
    driver: embassy_rp::Peri<'static, embassy_rp::peripherals::USB>,
    serial: &'static str,
) {
    let buffers = Box::leak(Box::new(Buffers::new(&USB_IO)));
    Usb::new(
        usb::driver(driver),
        serial,
        board::DEFAULT_ADAPTER_NAME,
        &USB_IO,
        buffers,
    )
    .run()
    .await;
}

#[embassy_executor::task]
async fn owner_task(
    store: &'static Shared<Store>,
    state: &'static State,
    identity: services::Identity,
) {
    // This is the only BTstack owner for the lifetime of the board.
    let mut radio = unsafe { Backend::new(state, store, None, identity.unique) };
    let mut files = store;
    let mut app = App::new(
        Build {
            version: FIRMWARE_VERSION,
            board: board::HARDWARE,
            model: board::DEFAULT_ADAPTER_NAME,
            development: cfg!(feature = "development"),
            adapter_id: identity.adapter,
        },
        &mut files,
    );
    let mut owner = Owner::new(&USB_IO);
    let mut shown = None;
    loop {
        // The serial API runs while the radio finishes starting.
        if RADIO_STARTED.try_take().is_some() {
            app.start(&mut radio);
        }
        radio.poll();
        while let Some(event) = debonair_core::radio::Radio::next_event(&mut radio) {
            app.radio_event(event, &mut radio);
        }
        let action = owner.poll(&mut app, &mut radio, &mut files, now());
        if action.is_some() {
            // Bond changes still waiting are saved before the board restarts.
            let _ = radio.flush();
        }
        match action {
            Some(Action::Restart) => services::restart(),
            #[cfg(feature = "development")]
            Some(Action::Bootloader) => services::bootloader(),
            _ => {}
        }
        let led = if !app.ready() {
            Led::Off
        } else if app.connected() {
            Led::On
        } else if app.pairing() {
            Led::Fast
        } else {
            Led::Slow
        };
        if shown != Some(led) {
            LED.signal(led);
            shown = Some(led);
        }
        // BTstack timers and the application have millisecond resolution.
        select3(USB_IO.changed(), state.changed(), Timer::after_millis(1)).await;
    }
}

#[embassy_executor::main(
    executor = "embassy_rp::executor::Executor",
    entry = "cortex_m_rt::entry"
)]
async fn main(spawner: Spawner) {
    let mut config = embassy_rp::config::Config::default();
    config.clocks = board::clocks();
    let p = embassy_rp::init(config);
    let mut indicator = debonair_pico::indicator_pin!(p);
    unsafe {
        services::init_heap();
    }
    let mut flash = Flash::<_, Async, { board::FLASH_BYTES }>::new(p.FLASH, p.DMA_CH1, DmaIrqs);
    let identity = services::identity(&mut flash).expect("board identity");
    let serial = Box::leak(identity.adapter.clone().into_boxed_str());
    let unique = identity.unique;
    let store: &'static Shared<Store> =
        Box::leak(Box::new(Shared::new(store::opened(Fs::open_or_provision_blank(
            flash,
            board::STORAGE_START..board::STORAGE_END,
            board::STORAGE_IDENTITY,
        )))));
    let state = Box::leak(Box::new(State::new(&RADIO_IO, now, fatal)));
    spawner.spawn(owner_task(store, state, identity).unwrap());
    // Queue USB before radio initialization. The synchronous SDK radio start finishes before this
    // executor can poll USB; the Embassy radio start yields and lets USB run during startup.
    spawner.spawn(usb_task(p.USB, serial).unwrap());
    let Some((mut control, _)) = radio::init(
        spawner,
        p.PIO0,
        p.DMA_CH0,
        p.DMA_CH2,
        debonair_pico::radio_pins!(p),
        &RADIO_IO,
        unique,
    )
    .await
    else {
        RADIO_IO.fail();
        core::future::pending::<()>().await;
        unreachable!();
    };
    RADIO_STARTED.signal(());
    let mut pattern = Led::Off;
    let mut lit = false;
    let mut ticks = 0u32;
    loop {
        if let Either::First(next) = select(LED.wait(), Timer::after_millis(125)).await {
            pattern = next;
        }
        ticks = ticks.wrapping_add(1);
        let want = match pattern {
            Led::Off => false,
            Led::On => true,
            Led::Fast => ticks.is_multiple_of(2),
            Led::Slow => ticks.is_multiple_of(16),
        };
        if want != lit {
            lit = want;
            if let Some(led) = indicator.as_mut() {
                led.set_level((lit ^ board::MCU_LED_ACTIVE_LOW).into());
            }
            if let Some((pin, active_low)) = board::RADIO_LED {
                control.led(pin, lit ^ active_low).await;
            }
        }
    }
}
