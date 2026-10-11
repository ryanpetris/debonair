//! The boundary between the application and a Bluetooth backend. Every backend, whatever host
//! stack and controller it uses, offers the Dongle to Targets as a HID keyboard and/or mouse through
//! this trait.
use crate::{ErrorCode, hid::Devices, hid::Encoded};
use alloc::vec::Vec;
use heapless::Vec as Bounded;

pub use debonair_protocol::Security;

/// A Target's Bluetooth identity.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Target {
    /// Most significant byte first.
    pub address: [u8; 6],
    pub random: bool,
}

/// What a backend needs to start.
pub struct Config<'a> {
    pub name: &'a str,
    pub devices: Devices,
    pub security: Security,
    pub battery: u8,
    /// The HID report descriptor for `devices`.
    pub descriptor: &'a [u8],
    /// Manufacturer, model and firmware strings for the Device Information service.
    pub manufacturer: &'a str,
    pub model: &'a str,
    pub firmware: &'a str,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Event {
    /// The stack is running and the Dongle can be found by paired Targets.
    Ready,
    /// The stack could not start or stopped working.
    Failed(ErrorCode),
    Connected(Target),
    /// The connected Target's identity address became known, such as after resolving a private
    /// address.
    Identified(Target),
    Disconnected,
    /// The link is encrypted, so reports can be sent.
    Secured,
    /// The Target asks for the passkey it shows.
    PasskeyRequest,
    PairingComplete {
        success: bool,
    },
    /// The Target wrote an output report, such as keyboard LEDs. Report ID 0 is the boot keyboard.
    Output {
        report_id: u8,
        data: Bounded<u8, 8>,
    },
    /// The Target wrote a feature report.
    Feature {
        report_id: u8,
        data: Bounded<u8, 8>,
    },
    /// The Target switched between boot and report protocol.
    ProtocolMode {
        boot: bool,
    },
    /// The set of paired Targets changed.
    TargetsChanged,
    /// A report that could not be sent earlier can be sent now.
    CanSend,
}

/// A static random Bluetooth address for one device combination of one board. `unique` is the
/// board's unique ID. Each combination is a separate identity to Targets.
pub fn static_address(unique: &[u8], devices: Devices) -> [u8; 6] {
    // FNV-1a: stable across builds and platforms.
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for &byte in unique.iter().chain(&[devices.index()]) {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    let mut address = [0; 6];
    address.copy_from_slice(&hash.to_be_bytes()[..6]);
    // A static address has its two most significant bits set and is not all ones in the rest.
    address[0] |= 0xc0;
    if address[1..] == [0xff; 5] && address[0] == 0xff {
        address[5] = 0xfe;
    }
    address
}

pub trait Radio {
    /// Starts the stack. A Ready or Failed event follows.
    fn start(&mut self, config: &Config<'_>) -> Result<(), ErrorCode>;
    fn set_name(&mut self, name: &str) -> Result<(), ErrorCode>;
    fn set_security(&mut self, security: Security);
    /// Whether a new Target may pair. While pairing, the Dongle is discoverable.
    fn set_pairing(&mut self, enabled: bool);
    /// Answers a passkey request; None rejects the pairing.
    fn passkey(&mut self, passkey: Option<u32>) -> Result<(), ErrorCode>;
    fn disconnect(&mut self) -> Result<(), ErrorCode>;
    /// Targets paired with the current device combination.
    fn targets(&mut self) -> Vec<Target>;
    fn max_targets(&self) -> u32;
    fn forget(&mut self, target: Target) -> Result<(), ErrorCode>;
    fn set_battery(&mut self, level: u8);
    /// Whether a report can be sent now. When it returns false, the backend sends CanSend once it
    /// can.
    fn can_send(&mut self) -> bool;
    fn send(&mut self, report: &Encoded) -> Result<(), ErrorCode>;
    fn next_event(&mut self) -> Option<Event>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn static_addresses_differ_per_combination_and_board() {
        let keyboard = Devices {
            keyboard: true,
            mouse: false,
        };
        let both = Devices {
            keyboard: true,
            mouse: true,
        };
        let a = static_address(&[1, 2, 3, 4, 5, 6, 7, 8], keyboard);
        assert_eq!(a, static_address(&[1, 2, 3, 4, 5, 6, 7, 8], keyboard));
        assert_ne!(a, static_address(&[1, 2, 3, 4, 5, 6, 7, 8], both));
        assert_ne!(a, static_address(&[1, 2, 3, 4, 5, 6, 7, 9], keyboard));
        assert_eq!(a[0] & 0xc0, 0xc0);
    }
}
