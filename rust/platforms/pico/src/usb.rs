use embassy_rp::peripherals::USB;
use embassy_rp::usb::Driver;
use embassy_rp::{Peri, bind_interrupts};

bind_interrupts!(pub struct Irqs {
    USBCTRL_IRQ => embassy_rp::usb::InterruptHandler<USB>;
});

/// Copies USB DPRAM with byte accesses, since unaligned wider accesses fault.
///
/// # Safety
/// Source and destination are valid for `len` bytes and do not overlap.
#[cfg(all(feature = "firmware", any(feature = "rp235xa", feature = "rp235xb")))]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __wrap_memcpy(dst: *mut u8, src: *const u8, len: usize) -> *mut u8 {
    let base = embassy_rp::pac::USB_DPRAM.as_ptr() as usize;
    let dpram = base..base + 4096;
    if dpram.contains(&(dst as usize)) || dpram.contains(&(src as usize)) {
        for offset in 0..len {
            unsafe {
                dst.add(offset)
                    .write_volatile(src.add(offset).read_volatile())
            };
        }
        dst
    } else {
        unsafe extern "C" {
            fn __real_memcpy(dst: *mut u8, src: *const u8, len: usize) -> *mut u8;
        }
        unsafe { __real_memcpy(dst, src, len) }
    }
}

pub fn driver(usb: Peri<'static, USB>) -> Driver<'static, USB> {
    Driver::new(usb, Irqs)
}
