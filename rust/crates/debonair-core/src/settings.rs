//! Settings saved on the Dongle in `/adapter.pb`: how it appears to Targets.
use crate::{
    hid::Devices,
    protocol::{MAX_NAME_BYTES, Security, storage as saved},
    store::{self, Store},
};
use alloc::string::String;

pub const PATH: &str = "/adapter.pb";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Settings {
    /// Empty for the default name.
    pub name: String,
    pub devices: Devices,
    pub security: Security,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            name: String::new(),
            devices: Devices {
                keyboard: true,
                mouse: true,
            },
            security: Security::JustWorks,
        }
    }
}

/// Whether `name` can be shown to Targets: 1..29 bytes without control characters.
pub fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= MAX_NAME_BYTES
        && name == name.trim()
        && !name.chars().any(char::is_control)
}

impl Settings {
    /// Loads saved settings. Missing settings give the defaults, and so does each value of a record
    /// that breaks its rules.
    pub fn load(store: &mut impl Store) -> Result<Self, store::Error> {
        let Some(bytes) = store.load(PATH)? else {
            return Ok(Self::default());
        };
        let Ok(saved) = store::decode::<saved::Adapter>(&bytes) else {
            return Ok(Self::default());
        };
        let mut settings = Self::default();
        if let Some(name) = saved.name.filter(|name| valid_name(name)) {
            settings.name = name;
        }
        if saved.keyboard || saved.mouse {
            settings.devices = Devices {
                keyboard: saved.keyboard,
                mouse: saved.mouse,
            };
        }
        if saved.security == saved::PairingSecurity::Passkey as i32 {
            settings.security = Security::Passkey;
        }
        Ok(settings)
    }

    pub fn save(&self, store: &mut impl Store) -> Result<(), store::Error> {
        let saved = saved::Adapter {
            name: (!self.name.is_empty()).then(|| self.name.clone()),
            keyboard: self.devices.keyboard,
            mouse: self.devices.mouse,
            security: match self.security {
                Security::JustWorks => saved::PairingSecurity::JustWorks,
                Security::Passkey => saved::PairingSecurity::Passkey,
            }
            .into(),
        };
        store.save(PATH, &store::encode(&saved)?)
    }

    pub fn effective_name<'a>(&'a self, default: &'a str) -> &'a str {
        if self.name.is_empty() {
            default
        } else {
            &self.name
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::memory::Memory;

    #[test]
    fn round_trips_and_defaults() {
        let mut store = Memory::default();
        assert_eq!(Settings::load(&mut store).unwrap(), Settings::default());
        // The defaults still encode as a nonempty record.
        Settings::default().save(&mut store).unwrap();
        assert!(!store.files[PATH].is_empty());
        let settings = Settings {
            name: "Desk".into(),
            devices: Devices {
                keyboard: false,
                mouse: true,
            },
            security: Security::Passkey,
        };
        settings.save(&mut store).unwrap();
        assert_eq!(Settings::load(&mut store).unwrap(), settings);
        store.files.insert(PATH.into(), alloc::vec![0xff, 0xff]);
        assert_eq!(Settings::load(&mut store).unwrap(), Settings::default());
    }

    #[test]
    fn values_that_break_their_rules_read_as_defaults() {
        let mut store = Memory::default();
        let saved = saved::Adapter {
            name: Some(" padded".into()),
            keyboard: false,
            mouse: false,
            security: 7,
        };
        store
            .files
            .insert(PATH.into(), store::encode(&saved).unwrap());
        assert_eq!(Settings::load(&mut store).unwrap(), Settings::default());
    }

    #[test]
    fn validates_names() {
        assert!(valid_name("Keyboard"));
        assert!(!valid_name(""));
        assert!(!valid_name(" padded"));
        assert!(!valid_name("tab\there"));
        assert!(!valid_name(&"x".repeat(30)));
        assert!(valid_name(&"x".repeat(29)));
    }
}
