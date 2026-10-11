//! The Dongle's USB device: one CDC-ACM serial port that carries the Debonair serial API.
//!
//! The USB tasks exchange bounded, owned packets with the owner task through [`Io`]. Every packet
//! carries the serial session it belongs to. A session starts when the Client opens the port
//! (raises DTR), or when data arrives on a port that is not open, and ends when the Client closes
//! the port or the bus resets.
#![no_std]

#[cfg(test)]
extern crate std;

pub mod owner;

use core::cell::Cell;
use debonair_protocol::{USB_MANUFACTURER, USB_PRODUCT_ID, USB_VENDOR_ID};
use embassy_futures::{join::join3, select::select};
use embassy_sync::{
    blocking_mutex::{Mutex, raw::CriticalSectionRawMutex as CS},
    channel::Channel,
    signal::Signal,
};
use embassy_usb::{
    Builder, Config, Handler, UsbDevice,
    class::cdc_acm,
    control::{OutResponse, Recipient, Request, RequestType},
    driver::Driver,
};

/// IN packets stay shorter than the 64-byte maximum, so each one ends a transfer and reaches the
/// Client's read at once without a zero-length packet.
pub const IN_BYTES: usize = 63;
pub const OUT_BYTES: usize = 64;

#[derive(Clone, Copy, Default, Debug, Eq, PartialEq)]
pub struct Status {
    pub configured: bool,
    pub suspended: bool,
    pub open: bool,
    /// Changes whenever a session starts or ends.
    pub session: u64,
}

#[derive(Clone, Copy)]
pub struct Packet {
    pub session: u64,
    pub length: usize,
    pub bytes: [u8; OUT_BYTES],
}
impl Packet {
    pub fn data(&self) -> &[u8] {
        &self.bytes[..self.length]
    }
}

pub struct Io {
    status: Mutex<CS, Cell<Status>>,
    changed: Signal<CS, ()>,
    session_changed: Signal<CS, ()>,
    rx: Channel<CS, Packet, 4>,
    tx: Channel<CS, Packet, 2>,
    /// Packets queued for the Client that the USB task has not finished with.
    unwritten: Mutex<CS, Cell<usize>>,
}
impl Default for Io {
    fn default() -> Self {
        Self::new()
    }
}
impl Io {
    pub const fn new() -> Self {
        Self {
            status: Mutex::new(Cell::new(Status {
                configured: false,
                suspended: false,
                open: false,
                session: 0,
            })),
            changed: Signal::new(),
            session_changed: Signal::new(),
            rx: Channel::new(),
            tx: Channel::new(),
            unwritten: Mutex::new(Cell::new(0)),
        }
    }
    pub fn status(&self) -> Status {
        self.status.lock(Cell::get)
    }
    /// Completes after the status changes or a packet arrives.
    pub async fn changed(&self) {
        self.changed.wait().await;
    }
    fn update(&self, change: impl FnOnce(&mut Status)) {
        let (before, after) = self.status.lock(|cell| {
            let before = cell.get();
            let mut after = before;
            change(&mut after);
            cell.set(after);
            (before, after)
        });
        if before != after {
            if before.session != after.session {
                self.session_changed.signal(());
            }
            self.changed.signal(());
        }
    }
    fn set_open(&self, open: bool) {
        self.update(|s| {
            let open = open && s.configured;
            if s.open != open {
                s.open = open;
                s.session = s.session.wrapping_add(1);
            }
        });
    }
    fn reset(&self, configured: bool) {
        self.update(|s| {
            if s.open {
                s.session = s.session.wrapping_add(1);
            }
            s.open = false;
            s.configured = configured;
            s.suspended = false;
        });
    }

    /// A packet the Client sent, for the owner task.
    pub fn receive(&self) -> Option<Packet> {
        self.rx.try_receive().ok()
    }
    /// Whether the owner can queue another packet for the Client.
    pub fn can_send(&self) -> bool {
        !self.tx.is_full()
    }
    /// Queues up to IN_BYTES for the Client in the current session.
    pub fn send(&self, data: &[u8]) -> bool {
        let status = self.status();
        if !status.open || data.len() > IN_BYTES {
            return false;
        }
        let mut packet = Packet {
            session: status.session,
            length: data.len(),
            bytes: [0; OUT_BYTES],
        };
        packet.bytes[..data.len()].copy_from_slice(data);
        // Counted before the USB task can take it.
        self.unwritten.lock(|count| count.set(count.get() + 1));
        let queued = self.tx.try_send(packet).is_ok();
        if !queued {
            self.finished();
        }
        queued
    }
    /// Whether every packet queued for the Client has been handed to the USB controller or
    /// abandoned with its session.
    pub fn written(&self) -> bool {
        self.unwritten.lock(Cell::get) == 0
    }
    fn finished(&self) {
        self.unwritten
            .lock(|count| count.set(count.get().saturating_sub(1)));
    }
}

pub struct BusHandler<'a>(pub &'a Io);
impl Handler for BusHandler<'_> {
    fn control_out(&mut self, req: Request, _: &[u8]) -> Option<OutResponse> {
        // SET_CONTROL_LINE_STATE on the CDC interface. Observed before the class accepts the
        // request, so data the Client sends after opening belongs to the new session.
        if (req.request_type, req.recipient, req.index, req.request)
            == (RequestType::Class, Recipient::Interface, 0, 0x22)
        {
            self.0.set_open(req.value & 1 != 0);
        }
        None
    }
    fn enabled(&mut self, enabled: bool) {
        if !enabled {
            self.0.reset(false);
        }
    }
    fn reset(&mut self) {
        self.0.reset(false);
    }
    fn configured(&mut self, configured: bool) {
        self.0.reset(configured);
    }
    fn suspended(&mut self, suspended: bool) {
        self.0.update(|s| s.suspended = suspended);
    }
}

/// Descriptor and class storage that lives as long as the USB task.
pub struct Buffers<'a> {
    pub config: [u8; 128],
    pub bos: [u8; 32],
    // USB strings use UTF-16; board names allow up to 64 UTF-8 bytes.
    pub control: [u8; 256],
    pub cdc: cdc_acm::State<'a>,
    pub handler: BusHandler<'a>,
}
impl<'a> Buffers<'a> {
    pub fn new(io: &'a Io) -> Self {
        Self {
            config: [0; 128],
            bos: [0; 32],
            control: [0; 256],
            cdc: cdc_acm::State::new(),
            handler: BusHandler(io),
        }
    }
}

pub struct Usb<'d, D: Driver<'d>> {
    device: UsbDevice<'d, D>,
    tx: cdc_acm::Sender<'d, D>,
    rx: cdc_acm::Receiver<'d, D>,
    io: &'d Io,
}
impl<'d, D: Driver<'d>> Usb<'d, D> {
    pub fn new(
        driver: D,
        serial: &'d str,
        product: &'d str,
        io: &'d Io,
        buffers: &'d mut Buffers<'d>,
    ) -> Self {
        let mut config = Config::new(USB_VENDOR_ID, USB_PRODUCT_ID);
        config.manufacturer = Some(USB_MANUFACTURER);
        config.product = Some(product);
        config.serial_number = Some(serial);
        config.device_release = 0x0100;
        config.max_power = 100;
        let mut builder = Builder::new(
            driver,
            config,
            &mut buffers.config,
            &mut buffers.bos,
            &mut [],
            &mut buffers.control,
        );
        builder.handler(&mut buffers.handler);
        let class = cdc_acm::CdcAcmClass::new(&mut builder, &mut buffers.cdc, OUT_BYTES as u16);
        let (tx, rx) = class.split();
        Self {
            device: builder.build(),
            tx,
            rx,
            io,
        }
    }

    pub async fn run(self) -> ! {
        let Self {
            mut device,
            mut tx,
            mut rx,
            io,
        } = self;
        join3(
            device.run(),
            async {
                let mut buffer = [0; OUT_BYTES];
                loop {
                    match rx.read_packet(&mut buffer).await {
                        Ok(0) => {}
                        Ok(length) => {
                            // Some hosts never raise DTR; their data starts a session.
                            if !io.status().open {
                                io.set_open(true);
                            }
                            let packet = Packet {
                                session: io.status().session,
                                length,
                                bytes: buffer,
                            };
                            io.rx.send(packet).await;
                            io.changed.signal(());
                        }
                        Err(_) => rx.wait_connection().await,
                    }
                }
            },
            async {
                loop {
                    let packet = io.tx.receive().await;
                    // Only a session change after this check abandons the write.
                    io.session_changed.reset();
                    let status = io.status();
                    // A closed or replaced session abandons its unsent output.
                    if status.open && status.session == packet.session {
                        let _ =
                            select(tx.write_packet(packet.data()), io.session_changed.wait()).await;
                    }
                    io.finished();
                    io.changed.signal(());
                }
            },
        )
        .await;
        unreachable!()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line_state(io: &Io, dtr: bool) {
        BusHandler(io).control_out(
            Request {
                direction: embassy_usb::driver::Direction::Out,
                request_type: RequestType::Class,
                recipient: Recipient::Interface,
                request: 0x22,
                value: u16::from(dtr),
                index: 0,
                length: 0,
            },
            &[],
        );
    }

    #[test]
    fn sessions_follow_dtr_and_bus_state() {
        let io = Io::new();
        line_state(&io, true);
        assert!(!io.status().open, "not configured yet");
        BusHandler(&io).configured(true);
        line_state(&io, true);
        let first = io.status();
        assert!(first.open);
        assert!(io.send(b"hello"));
        line_state(&io, false);
        assert!(!io.status().open);
        assert!(!io.send(b"late"));
        line_state(&io, true);
        let second = io.status();
        assert_ne!(first.session, second.session);
        BusHandler(&io).reset();
        assert!(!io.status().open);
        assert!(!io.status().configured);
    }

    #[test]
    fn output_is_bounded() {
        let io = Io::new();
        BusHandler(&io).configured(true);
        line_state(&io, true);
        assert!(!io.send(&[0; IN_BYTES + 1]));
        assert!(io.written());
        assert!(io.send(&[0; IN_BYTES]));
        assert!(io.send(&[0; 1]));
        assert!(!io.can_send());
        assert!(!io.send(&[0; 1]));
        assert!(!io.written());
        io.finished();
        io.finished();
        assert!(io.written());
    }
}
