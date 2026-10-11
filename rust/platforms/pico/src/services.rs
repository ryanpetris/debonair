use alloc::{format, string::String};
use embassy_rp::flash::{Flash, Instance, Mode};

/// Startup calls this once, before allocating application or driver state.
///
/// # Safety
/// No other code may use or have initialized the linker-reserved heap.
pub unsafe fn init_heap() {
    unsafe extern "C" {
        static mut __sheap: u8;
        static mut _heap_end: u8;
    }
    let start = core::ptr::addr_of_mut!(__sheap);
    let end = core::ptr::addr_of_mut!(_heap_end);
    assert!(unsafe { crate::allocator::init(start, end.addr() - start.addr()) });
}

pub struct Identity {
    pub unique: [u8; 8],
    pub adapter: String,
}
pub fn identity<T: Instance, M: Mode, const N: usize>(
    flash: &mut Flash<'_, T, M, N>,
) -> Result<Identity, embassy_rp::flash::Error> {
    #[cfg(feature = "rp2040")]
    let mut unique = [0; 8];
    #[cfg(feature = "rp2040")]
    flash.blocking_unique_id(&mut unique)?;
    #[cfg(any(feature = "rp235xa", feature = "rp235xb"))]
    let unique = {
        let _ = flash;
        embassy_rp::otp::get_chipid()
            .map_err(|_| embassy_rp::flash::Error::Other)?
            .to_be_bytes()
    };
    let adapter = format!("{:016X}", u64::from_be_bytes(unique));
    Ok(Identity { unique, adapter })
}

/// The CYW43 driver's fallback MAC address when OTP has none, derived from the board's flash ID.
pub fn fallback_wifi_address(unique: [u8; 8]) -> [u8; 6] {
    let mut wifi: [u8; 6] = unique[2..].try_into().unwrap();
    wifi[0] = (wifi[0] & !1) | 2;
    wifi
}

#[cfg(feature = "development")]
pub fn bootloader() -> ! {
    embassy_rp::rom_data::reset_to_usb_boot(0, 0);
    loop {
        core::hint::spin_loop();
    }
}

pub fn restart() -> ! {
    cortex_m::peripheral::SCB::sys_reset()
}
