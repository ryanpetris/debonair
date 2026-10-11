//! HID report descriptors and report encodings shared by every Bluetooth backend.
//!
//! Report IDs are fixed: 1 is the keyboard (input and LED output), 2 is the mouse (input and the
//! resolution multiplier feature) and 3 is consumer controls. A Dongle offers only the reports of
//! its enabled devices.
use alloc::vec::Vec;

pub const KEYBOARD_ID: u8 = 1;
pub const MOUSE_ID: u8 = 2;
pub const CONSUMER_ID: u8 = 3;

/// Report bytes, without the report ID.
pub const KEYBOARD_BYTES: usize = 8;
pub const MOUSE_BYTES: usize = 9;
pub const CONSUMER_BYTES: usize = 4;
pub const BOOT_MOUSE_BYTES: usize = 3;
/// Held consumer controls a report carries.
pub const CONSUMER_SLOTS: usize = 2;
/// The largest consumer usage the descriptor declares.
pub const CONSUMER_MAX: u16 = 0x3ff;
/// Wheel and pan units per detent when the Target enables the resolution multiplier.
pub const HIGH_RESOLUTION: i32 = 120;

/// Keyboard with modifiers, six held keys and LED output.
const KEYBOARD: &[u8] = &[
    0x05,
    0x01, // Usage Page (Generic Desktop)
    0x09,
    0x06, // Usage (Keyboard)
    0xa1,
    0x01, // Collection (Application)
    0x85,
    KEYBOARD_ID, // Report ID
    0x05,
    0x07, // Usage Page (Keyboard/Keypad)
    0x19,
    0xe0, // Usage Minimum (Left Control)
    0x29,
    0xe7, // Usage Maximum (Right GUI)
    0x15,
    0x00, // Logical Minimum (0)
    0x25,
    0x01, // Logical Maximum (1)
    0x75,
    0x01, // Report Size (1)
    0x95,
    0x08, // Report Count (8)
    0x81,
    0x02, // Input (Data, Variable, Absolute)
    0x75,
    0x08, // Report Size (8)
    0x95,
    0x01, // Report Count (1)
    0x81,
    0x01, // Input (Constant)
    0x05,
    0x08, // Usage Page (LEDs)
    0x19,
    0x01, // Usage Minimum (Num Lock)
    0x29,
    0x05, // Usage Maximum (Kana)
    0x75,
    0x01, // Report Size (1)
    0x95,
    0x05, // Report Count (5)
    0x91,
    0x02, // Output (Data, Variable, Absolute)
    0x75,
    0x03, // Report Size (3)
    0x95,
    0x01, // Report Count (1)
    0x91,
    0x01, // Output (Constant)
    0x05,
    0x07, // Usage Page (Keyboard/Keypad)
    0x19,
    0x00, // Usage Minimum (0)
    0x2a,
    0xff,
    0x00, // Usage Maximum (255)
    0x15,
    0x00, // Logical Minimum (0)
    0x26,
    0xff,
    0x00, // Logical Maximum (255)
    0x75,
    0x08, // Report Size (8)
    0x95,
    0x06, // Report Count (6)
    0x81,
    0x00, // Input (Data, Array, Absolute)
    0xc0, // End Collection
];

/// Consumer controls such as media and volume keys.
const CONSUMER: &[u8] = &[
    0x05,
    0x0c, // Usage Page (Consumer)
    0x09,
    0x01, // Usage (Consumer Control)
    0xa1,
    0x01, // Collection (Application)
    0x85,
    CONSUMER_ID, // Report ID
    0x19,
    0x00, // Usage Minimum (0)
    0x2a,
    0xff,
    0x03, // Usage Maximum (0x3ff)
    0x15,
    0x00, // Logical Minimum (0)
    0x26,
    0xff,
    0x03, // Logical Maximum (0x3ff)
    0x75,
    0x10, // Report Size (16)
    0x95,
    CONSUMER_SLOTS as u8, // Report Count
    0x81,
    0x00, // Input (Data, Array, Absolute)
    0xc0, // End Collection
];

/// Five-button mouse with 16-bit motion, and a wheel and horizontal pan that each sit in a logical
/// collection with a resolution multiplier. A Target that sets the multipliers reads wheel and pan
/// in 1/120 of a detent; otherwise it reads whole detents.
const MOUSE: &[u8] = &[
    0x05,
    0x01, // Usage Page (Generic Desktop)
    0x09,
    0x02, // Usage (Mouse)
    0xa1,
    0x01, // Collection (Application)
    0x85,
    MOUSE_ID, // Report ID
    0x09,
    0x01, // Usage (Pointer)
    0xa1,
    0x00, // Collection (Physical)
    0x05,
    0x09, // Usage Page (Button)
    0x19,
    0x01, // Usage Minimum (1)
    0x29,
    0x05, // Usage Maximum (5)
    0x15,
    0x00, // Logical Minimum (0)
    0x25,
    0x01, // Logical Maximum (1)
    0x75,
    0x01, // Report Size (1)
    0x95,
    0x05, // Report Count (5)
    0x81,
    0x02, // Input (Data, Variable, Absolute)
    0x75,
    0x03, // Report Size (3)
    0x95,
    0x01, // Report Count (1)
    0x81,
    0x01, // Input (Constant)
    0x05,
    0x01, // Usage Page (Generic Desktop)
    0x09,
    0x30, // Usage (X)
    0x09,
    0x31, // Usage (Y)
    0x16,
    0x01,
    0x80, // Logical Minimum (-32767)
    0x26,
    0xff,
    0x7f, // Logical Maximum (32767)
    0x75,
    0x10, // Report Size (16)
    0x95,
    0x02, // Report Count (2)
    0x81,
    0x06, // Input (Data, Variable, Relative)
    0xa1,
    0x02, // Collection (Logical)
    0x09,
    0x48, // Usage (Resolution Multiplier)
    0x15,
    0x00, // Logical Minimum (0)
    0x25,
    0x01, // Logical Maximum (1)
    0x35,
    0x01, // Physical Minimum (1)
    0x45,
    HIGH_RESOLUTION as u8, // Physical Maximum (120)
    0x75,
    0x02, // Report Size (2)
    0x95,
    0x01, // Report Count (1)
    0xb1,
    0x02, // Feature (Data, Variable, Absolute)
    0x35,
    0x00, // Physical Minimum (0)
    0x45,
    0x00, // Physical Maximum (0)
    0x09,
    0x38, // Usage (Wheel)
    0x16,
    0x01,
    0x80, // Logical Minimum (-32767)
    0x26,
    0xff,
    0x7f, // Logical Maximum (32767)
    0x75,
    0x10, // Report Size (16)
    0x95,
    0x01, // Report Count (1)
    0x81,
    0x06, // Input (Data, Variable, Relative)
    0xc0, // End Collection
    0xa1,
    0x02, // Collection (Logical)
    0x09,
    0x48, // Usage (Resolution Multiplier)
    0x15,
    0x00, // Logical Minimum (0)
    0x25,
    0x01, // Logical Maximum (1)
    0x35,
    0x01, // Physical Minimum (1)
    0x45,
    HIGH_RESOLUTION as u8, // Physical Maximum (120)
    0x75,
    0x02, // Report Size (2)
    0x95,
    0x01, // Report Count (1)
    0xb1,
    0x02, // Feature (Data, Variable, Absolute)
    0x35,
    0x00, // Physical Minimum (0)
    0x45,
    0x00, // Physical Maximum (0)
    0x75,
    0x04, // Report Size (4)
    0xb1,
    0x01, // Feature (Constant)
    0x05,
    0x0c, // Usage Page (Consumer)
    0x0a,
    0x38,
    0x02, // Usage (AC Pan)
    0x16,
    0x01,
    0x80, // Logical Minimum (-32767)
    0x26,
    0xff,
    0x7f, // Logical Maximum (32767)
    0x75,
    0x10, // Report Size (16)
    0x95,
    0x01, // Report Count (1)
    0x81,
    0x06, // Input (Data, Variable, Relative)
    0xc0, // End Collection
    0xc0, // End Collection
    0xc0, // End Collection
];

/// Which devices a Dongle offers. At least one is enabled.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Devices {
    pub keyboard: bool,
    pub mouse: bool,
}
impl Devices {
    /// A small stable number for each combination, used to keep each combination's Bluetooth
    /// identity and pairings apart.
    pub fn index(self) -> u8 {
        u8::from(self.keyboard) | (u8::from(self.mouse) << 1)
    }
    /// The name of the combination's directory in storage.
    pub fn name(self) -> &'static str {
        match (self.keyboard, self.mouse) {
            (true, false) => "keyboard",
            (false, true) => "mouse",
            _ => "keyboard-mouse",
        }
    }
    /// The Bluetooth GAP appearance: keyboard, mouse or generic HID.
    pub fn appearance(self) -> u16 {
        match (self.keyboard, self.mouse) {
            (true, false) => 0x03c1,
            (false, true) => 0x03c2,
            _ => 0x03c0,
        }
    }
    pub fn descriptor(self) -> Vec<u8> {
        let mut out = Vec::new();
        if self.keyboard {
            out.extend_from_slice(KEYBOARD);
            out.extend_from_slice(CONSUMER);
        }
        if self.mouse {
            out.extend_from_slice(MOUSE);
        }
        out
    }
}

/// Where an encoded report goes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Channel {
    /// A report protocol input report with this ID.
    Input(u8),
    BootKeyboard,
    BootMouse,
}

/// An input report ready for a backend to send.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Encoded {
    pub channel: Channel,
    pub length: usize,
    pub bytes: [u8; MOUSE_BYTES],
}
impl Encoded {
    pub fn data(&self) -> &[u8] {
        &self.bytes[..self.length]
    }
}

/// One complete report state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Report {
    /// Modifier byte and up to six held keys.
    Keyboard {
        modifiers: u8,
        keys: [u8; 6],
    },
    Mouse {
        buttons: u8,
        x: i16,
        y: i16,
        wheel: i16,
        pan: i16,
    },
    Consumer([u16; CONSUMER_SLOTS]),
}
impl Report {
    /// Encodes this report for the Target's protocol mode. Consumer reports have no boot protocol
    /// form and return None in boot mode, as do reports of a disabled device.
    pub fn encode(&self, boot: bool) -> Option<Encoded> {
        let mut bytes = [0; MOUSE_BYTES];
        let (channel, length) = match *self {
            Report::Keyboard { modifiers, keys } => {
                bytes[0] = modifiers;
                bytes[2..8].copy_from_slice(&keys);
                let channel = if boot {
                    Channel::BootKeyboard
                } else {
                    Channel::Input(KEYBOARD_ID)
                };
                (channel, KEYBOARD_BYTES)
            }
            Report::Mouse {
                buttons,
                x,
                y,
                wheel,
                pan,
            } => {
                if boot {
                    bytes[0] = buttons & 0x07;
                    bytes[1] = x.clamp(-127, 127) as i8 as u8;
                    bytes[2] = y.clamp(-127, 127) as i8 as u8;
                    (Channel::BootMouse, BOOT_MOUSE_BYTES)
                } else {
                    bytes[0] = buttons & 0x1f;
                    bytes[1..3].copy_from_slice(&x.to_le_bytes());
                    bytes[3..5].copy_from_slice(&y.to_le_bytes());
                    bytes[5..7].copy_from_slice(&wheel.to_le_bytes());
                    bytes[7..9].copy_from_slice(&pan.to_le_bytes());
                    (Channel::Input(MOUSE_ID), MOUSE_BYTES)
                }
            }
            Report::Consumer(usages) => {
                if boot {
                    return None;
                }
                for (i, usage) in usages.iter().enumerate() {
                    bytes[2 * i..2 * i + 2].copy_from_slice(&usage.to_le_bytes());
                }
                (Channel::Input(CONSUMER_ID), CONSUMER_BYTES)
            }
        };
        Some(Encoded {
            channel,
            length,
            bytes,
        })
    }
}

/// The multipliers a Target set through the mouse feature report: whether wheel and pan are in
/// 1/120 of a detent.
pub fn multipliers(feature: &[u8]) -> (bool, bool) {
    let value = feature.first().copied().unwrap_or(0);
    (value & 0x03 != 0, value & 0x0c != 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Walks short items and returns (report ID, input bits) pairs, checking collection balance.
    fn input_bits(descriptor: &[u8]) -> Vec<(u8, u32)> {
        let mut result: Vec<(u8, u32)> = Vec::new();
        let (mut id, mut size, mut count, mut depth) = (0u8, 0u32, 0u32, 0i32);
        let mut at = 0;
        while at < descriptor.len() {
            let prefix = descriptor[at];
            let length = [0, 1, 2, 4][usize::from(prefix & 3)];
            let mut value = 0u32;
            for i in 0..length {
                value |= u32::from(descriptor[at + 1 + i]) << (8 * i);
            }
            match prefix & 0xfc {
                0x84 => id = value as u8,
                0x74 => size = value,
                0x94 => count = value,
                0xa0 => depth += 1,
                0xc0 => depth -= 1,
                0x80 => match result.iter_mut().find(|(i, _)| *i == id) {
                    Some(entry) => entry.1 += size * count,
                    None => result.push((id, size * count)),
                },
                _ => {}
            }
            at += 1 + length;
        }
        assert_eq!(depth, 0);
        result
    }

    #[test]
    fn descriptor_sizes_match_encodings() {
        let all = Devices {
            keyboard: true,
            mouse: true,
        };
        let bits = input_bits(&all.descriptor());
        assert_eq!(
            bits,
            [
                (KEYBOARD_ID, KEYBOARD_BYTES as u32 * 8),
                (CONSUMER_ID, CONSUMER_BYTES as u32 * 8),
                (MOUSE_ID, MOUSE_BYTES as u32 * 8),
            ]
        );
    }

    #[test]
    fn descriptors_offer_only_enabled_devices() {
        let keyboard = Devices {
            keyboard: true,
            mouse: false,
        };
        let mouse = Devices {
            keyboard: false,
            mouse: true,
        };
        let ids = |d: Devices| {
            input_bits(&d.descriptor())
                .iter()
                .map(|e| e.0)
                .collect::<Vec<_>>()
        };
        assert_eq!(ids(keyboard), [KEYBOARD_ID, CONSUMER_ID]);
        assert_eq!(ids(mouse), [MOUSE_ID]);
        assert_ne!(keyboard.index(), mouse.index());
        assert_eq!(keyboard.appearance(), 0x03c1);
        assert_eq!(mouse.appearance(), 0x03c2);
    }

    #[test]
    fn boot_mouse_clamps_motion() {
        let report = Report::Mouse {
            buttons: 0x1f,
            x: 500,
            y: -500,
            wheel: 3,
            pan: 0,
        };
        let boot = report.encode(true).unwrap();
        assert_eq!(boot.channel, Channel::BootMouse);
        assert_eq!(boot.data(), [0x07, 127, (-127i8) as u8]);
        let full = report.encode(false).unwrap();
        assert_eq!(full.data(), [0x1f, 0xf4, 0x01, 0x0c, 0xfe, 3, 0, 0, 0]);
        assert!(Report::Consumer([0xe9, 0]).encode(true).is_none());
    }

    #[test]
    fn feature_report_enables_each_multiplier() {
        assert_eq!(multipliers(&[0x01]), (true, false));
        assert_eq!(multipliers(&[0x04]), (false, true));
        assert_eq!(multipliers(&[0x05]), (true, true));
        assert_eq!(multipliers(&[]), (false, false));
    }
}
