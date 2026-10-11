//! Saves the bonds BTstack keeps in its RAM device database, `c/bond_db.c`, and fills the database
//! from the saved bonds before the stack starts. BTstack never reads or writes storage itself.
use crate::ffi;
use alloc::vec::Vec;
use core::ffi::c_int;
use debonair_core::{
    bonds::{self, Bond, Entry, Identity, MAX_BONDS, Native, Table},
    hid::Devices,
    radio::Target,
    store::{Error, Shared, Store},
};

/// How long saving new identity roots waits after a failure before it tries again.
const RETRY_MS: u64 = 5000;

pub struct Bonds<S: 'static> {
    store: &'static Shared<S>,
    devices: Option<Devices>,
    table: Option<Table>,
    /// Roots the security manager made that have not reached storage, and when to try again.
    unsaved: Option<(Identity, u64)>,
}

fn export(slot: usize) -> Option<Bond> {
    let mut native = Native::default();
    (unsafe { ffi::debonair_bonds_export(slot as c_int, &mut native) } != 0)
        .then(|| Bond::from(&native))
}

/// BTstack writes an entry's address and keys together, so an entry is never incomplete.
fn entry(slot: usize) -> Entry {
    export(slot).into()
}

impl<S: Store + 'static> Bonds<S> {
    pub const fn new(store: &'static Shared<S>) -> Self {
        Self {
            store,
            devices: None,
            table: None,
            unsaved: None,
        }
    }

    /// Loads the identity roots and bonds of `devices` into the device database. A combination
    /// without roots gets them from BTstack's security manager, which makes them from the
    /// controller's random numbers once the stack starts; they are saved before any bond.
    pub fn load(&mut self, devices: Devices) -> Result<(), Error> {
        let mut store = self.store;
        let identity = Identity::load(&mut store, devices)?;
        let loaded = bonds::load(&mut store, devices)?;
        match &identity {
            Some(identity) => unsafe {
                ffi::debonair_bonds_reset(identity.ir.as_ptr(), identity.er.as_ptr())
            },
            None => unsafe { ffi::debonair_bonds_reset(core::ptr::null(), core::ptr::null()) },
        }
        let mut table = Table::new(devices, loaded.next_id);
        for (id, bond) in &loaded.bonds {
            let slot = unsafe { ffi::debonair_bonds_import(&Native::from(bond)) };
            if let Ok(slot) = usize::try_from(slot) {
                table.assign(slot, *id);
            }
        }
        self.devices = Some(devices);
        self.table = Some(table);
        Ok(())
    }

    /// The Target entry `slot` holds.
    pub fn target(&self, slot: usize) -> Option<Target> {
        export(slot).map(|b| Target {
            address: b.address,
            random: b.random,
        })
    }

    /// Whether the pairing or keys of entry `slot`, or the identity roots its keys depend on, have
    /// not reached storage.
    pub fn unsaved(&self, slot: usize) -> bool {
        self.table.as_ref().is_some_and(|t| !t.saved(slot))
    }

    /// The device database entry that holds `target`.
    pub fn slot(&self, target: Target) -> Option<usize> {
        (0..MAX_BONDS).find(|slot| {
            export(*slot).is_some_and(|b| b.address == target.address && b.random == target.random)
        })
    }

    /// The IDs of the saved bonds entry `slot` holds or replaced.
    pub fn held(&self, slot: usize) -> Vec<u64> {
        self.table
            .as_ref()
            .map(|t| t.held(slot))
            .unwrap_or_default()
    }

    /// Whether one of the saved bonds `ids` of entry `slot` has not been removed yet.
    pub fn holds(&self, slot: usize, ids: &[u64]) -> bool {
        self.table.as_ref().is_some_and(|t| t.holds(slot, ids))
    }

    /// Saves what BTstack changed: new identity roots first, then new pairings, keys and removals at
    /// once, and other changes once they pause. With `all`, saves everything waiting.
    pub fn save(&mut self, now: u64, all: bool) -> Result<(), Error> {
        let (Some(table), Some(devices)) = (&mut self.table, self.devices) else {
            return Ok(());
        };
        let mut store = self.store;
        let (mut ir, mut er) = ([0; 16], [0; 16]);
        if unsafe { ffi::debonair_bonds_take_roots(ir.as_mut_ptr(), er.as_mut_ptr()) } != 0 {
            self.unsaved = Some((Identity::from_roots(ir, er), now));
        }
        let (mut added, mut updated) = (0, 0);
        let changed = unsafe { ffi::debonair_bonds_take(&mut added, &mut updated) };
        for slot in 0..MAX_BONDS {
            let bit = 1 << slot;
            if added & bit != 0 {
                table.added(slot, now);
            } else if changed & bit != 0 {
                table.changed(slot, now);
            }
            if updated & bit != 0 {
                table.updated(slot, now);
            }
        }
        // Bonds are saved only beside the roots their keys depend on. A new pairing tries the roots
        // again at once.
        if let Some((identity, at)) = &mut self.unsaved {
            if !all && added == 0 && now < *at {
                return Ok(());
            }
            if let Err(e) = identity.save(&mut store, devices) {
                *at = now + RETRY_MS;
                return Err(e);
            }
            self.unsaved = None;
        }
        if !table.pending() {
            return Ok(());
        }
        table.save(&mut store, now, all, entry)
    }
}
