//! The Bluetooth identity and bonds of each combination of keyboard and mouse, saved in a form
//! independent of any Bluetooth stack.
//!
//! A Bluetooth backend keeps its stack's bond table in RAM only. When it starts, it fills the table
//! from the saved bonds; when the stack changes an entry, it tells a [`Table`], which saves the
//! entry's [`Bond`] as its own record. Keys are most-significant byte first; backends reverse
//! native little-endian keys at their boundaries.
use crate::{
    hid::Devices,
    protocol::storage as saved,
    store::{self, Error, Store},
};
use aes::{
    Aes128, Block,
    cipher::{BlockEncrypt, KeyInit},
};
use alloc::{format, string::String, vec::Vec};
use heapless::Vec as Bounded;

/// The bonds each combination keeps. A pairing beyond them replaces the oldest.
pub const MAX_BONDS: usize = 8;
/// The subscriptions each bond keeps.
pub const MAX_SUBSCRIPTIONS: usize = 8;
/// The longest database identifier a bond keeps.
pub const DATABASE_BYTES: usize = 16;
/// How long a bond waits after its last deferred change before it is saved.
const QUIET_MS: u64 = 250;
/// How long a bond waits at most after its first deferred change.
const LATEST_MS: u64 = 2000;
/// How long saving waits after a failure before it tries again.
const RETRY_MS: u64 = 5000;

/// The directory holding the records of a combination of keyboard and mouse.
pub fn directory(devices: Devices) -> String {
    format!("/combinations/{}", devices.name())
}
fn identity_path(devices: Devices) -> String {
    format!("{}/identity.pb", directory(devices))
}
fn bonds_directory(devices: Devices) -> String {
    format!("{}/bonds", directory(devices))
}
fn bond_path(devices: Devices, id: u64) -> String {
    format!("{}/bonds/{id}/bond.pb", directory(devices))
}

/// A combination's identity roots.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Identity {
    pub ir: [u8; 16],
    pub er: [u8; 16],
    /// Derived from `ir`, as BTstack derives it.
    pub irk: [u8; 16],
}

impl Identity {
    fn derive_irk(ir: &[u8; 16]) -> [u8; 16] {
        let cipher = Aes128::new(&(*ir).into());
        let mut block = Block::default();
        block[15] = 1;
        cipher.encrypt_block(&mut block);
        block.into()
    }

    /// The identity with roots `ir` and `er`.
    pub fn from_roots(ir: [u8; 16], er: [u8; 16]) -> Self {
        Self {
            ir,
            er,
            irk: Self::derive_irk(&ir),
        }
    }

    /// The saved identity of `devices`, or None when it has none. Missing roots beside saved bonds
    /// are `Corrupt`: a new identity would not match what Targets keep.
    pub fn load(store: &mut impl Store, devices: Devices) -> Result<Option<Self>, Error> {
        if let Some(bytes) = store.load(&identity_path(devices))? {
            let saved: saved::Identity = store::decode(&bytes)?;
            let identity = Self::from_roots(store::array(&saved.ir)?, store::array(&saved.er)?);
            if store::array::<16>(&saved.irk)? != identity.irk {
                return Err(Error::Corrupt);
            }
            return Ok(Some(identity));
        }
        if store
            .list(&bonds_directory(devices))?
            .iter()
            .any(|name| store::parse_id(name).is_some())
        {
            return Err(Error::Corrupt);
        }
        Ok(None)
    }

    /// Saves the identity of `devices`.
    pub fn save(&self, store: &mut impl Store, devices: Devices) -> Result<(), Error> {
        let saved = saved::Identity {
            ir: self.ir.into(),
            er: self.er.into(),
            irk: self.irk.into(),
        };
        store.save(&identity_path(devices), &store::encode(&saved)?)
    }

    /// The saved identity of `devices`, or a new one with roots from `random`, saved before it is
    /// returned.
    pub fn load_or_create(
        store: &mut impl Store,
        devices: Devices,
        random: &mut dyn FnMut() -> u64,
    ) -> Result<Self, Error> {
        if let Some(identity) = Self::load(store, devices)? {
            return Ok(identity);
        }
        let mut roots = [0u8; 32];
        for chunk in roots.as_chunks_mut::<8>().0 {
            chunk.copy_from_slice(&random().to_be_bytes());
        }
        let identity = Self::from_roots(
            roots[..16].try_into().unwrap(),
            roots[16..].try_into().unwrap(),
        );
        identity.save(store, devices)?;
        Ok(identity)
    }
}

/// Keys one side of a pairing distributed.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Keys {
    pub ltk: Option<[u8; 16]>,
    pub ediv: u16,
    pub rand: [u8; 8],
    pub irk: Option<[u8; 16]>,
    pub csrk: Option<[u8; 16]>,
    pub sign_counter: u32,
}

impl Keys {
    fn to_saved(&self) -> saved::Keys {
        let key = |k: &Option<[u8; 16]>| k.map(Vec::from).unwrap_or_default();
        saved::Keys {
            ltk: key(&self.ltk),
            ediv: self.ediv.into(),
            rand: if self.ltk.is_some() {
                self.rand.into()
            } else {
                Vec::new()
            },
            irk: key(&self.irk),
            csrk: key(&self.csrk),
            sign_counter: self.sign_counter,
        }
    }
    fn from_saved(saved: &saved::Keys) -> Result<Self, Error> {
        let key = |bytes: &[u8]| -> Result<Option<[u8; 16]>, Error> {
            if bytes.is_empty() {
                Ok(None)
            } else {
                store::array(bytes).map(Some)
            }
        };
        Ok(Self {
            ltk: key(&saved.ltk)?,
            ediv: u16::try_from(saved.ediv).map_err(|_| Error::Corrupt)?,
            rand: if saved.rand.is_empty() {
                [0; 8]
            } else {
                store::array(&saved.rand)?
            },
            irk: key(&saved.irk)?,
            csrk: key(&saved.csrk)?,
            sign_counter: saved.sign_counter,
        })
    }
}

/// A Client Characteristic Configuration a Target wrote.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Subscription {
    /// The attribute handle the Bluetooth stack keys the subscription by.
    pub handle: u16,
    pub value: u16,
}

/// A Target paired with one combination, as the Bluetooth stack keeps it.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Bond {
    /// The identity address, most-significant byte first.
    pub address: [u8; 6],
    pub random: bool,
    pub key_size: u8,
    pub authenticated: bool,
    pub authorized: bool,
    pub secure_connections: bool,
    pub local: Option<Keys>,
    pub peer: Option<Keys>,
    /// The mouse feature report the Target last set.
    pub feature: Option<u8>,
    /// Identifies the GATT database the Target last discovered; empty when unknown.
    pub database: Bounded<u8, DATABASE_BYTES>,
    pub subscriptions: Bounded<Subscription, MAX_SUBSCRIPTIONS>,
}

impl Bond {
    pub fn to_saved(&self) -> saved::Bond {
        saved::Bond {
            address: self.address.into(),
            random: self.random,
            key_size: self.key_size.into(),
            authenticated: self.authenticated,
            authorized: self.authorized,
            secure_connections: self.secure_connections,
            local: self.local.as_ref().map(Keys::to_saved),
            peer: self.peer.as_ref().map(Keys::to_saved),
            feature: self.feature.map(u32::from),
            database: self.database.to_vec(),
            subscriptions: self
                .subscriptions
                .iter()
                .map(|s| saved::Subscription {
                    handle: s.handle.into(),
                    value: s.value.into(),
                })
                .collect(),
        }
    }

    /// The bond a saved record holds. `Corrupt` when the record breaks the schema's rules.
    pub fn from_saved(saved: &saved::Bond) -> Result<Self, Error> {
        let mut subscriptions = Bounded::new();
        for s in &saved.subscriptions {
            let handle = u16::try_from(s.handle).map_err(|_| Error::Corrupt)?;
            if handle == 0 {
                return Err(Error::Corrupt);
            }
            let value = u16::try_from(s.value).map_err(|_| Error::Corrupt)?;
            subscriptions
                .push(Subscription { handle, value })
                .map_err(|_| Error::Corrupt)?;
        }
        let key_size = u8::try_from(saved.key_size).map_err(|_| Error::Corrupt)?;
        if key_size > 16 {
            return Err(Error::Corrupt);
        }
        Ok(Self {
            address: store::array(&saved.address)?,
            random: saved.random,
            key_size,
            authenticated: saved.authenticated,
            authorized: saved.authorized,
            secure_connections: saved.secure_connections,
            local: saved.local.as_ref().map(Keys::from_saved).transpose()?,
            peer: saved.peer.as_ref().map(Keys::from_saved).transpose()?,
            feature: saved
                .feature
                .map(|f| u8::try_from(f).map_err(|_| Error::Corrupt))
                .transpose()?,
            database: Bounded::from_slice(&saved.database).map_err(|_| Error::Corrupt)?,
            subscriptions,
        })
    }
}

/// The C layout both Bluetooth backends exchange bonds in, `debonair_bond` in their C headers.
/// Keys and the address are most-significant byte first.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Native {
    pub address: [u8; 6],
    pub random: u8,
    pub key_size: u8,
    /// [`Native::AUTHENTICATED`], [`Native::AUTHORIZED`], [`Native::SECURE_CONNECTIONS`] and
    /// [`Native::FEATURE`].
    pub flags: u8,
    pub feature: u8,
    pub database_length: u8,
    pub database: [u8; DATABASE_BYTES],
    pub local: NativeKeys,
    pub peer: NativeKeys,
    /// Handle zero for none.
    pub subscriptions: [NativeSubscription; MAX_SUBSCRIPTIONS],
}

/// `debonair_keys`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct NativeKeys {
    /// [`NativeKeys::PRESENT`] when the side's keys are kept, with [`NativeKeys::LTK`],
    /// [`NativeKeys::IRK`] and [`NativeKeys::CSRK`] for the keys it distributed.
    pub flags: u8,
    pub ediv: u16,
    pub rand: [u8; 8],
    pub ltk: [u8; 16],
    pub irk: [u8; 16],
    pub csrk: [u8; 16],
    pub sign_counter: u32,
}

/// `debonair_subscription`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct NativeSubscription {
    pub handle: u16,
    pub value: u16,
}

const _: () = assert!(core::mem::size_of::<NativeKeys>() == 64);
const _: () = assert!(core::mem::size_of::<Native>() == 188);

impl NativeKeys {
    pub const LTK: u8 = 1;
    pub const IRK: u8 = 2;
    pub const CSRK: u8 = 4;
    pub const PRESENT: u8 = 0x80;

    fn keys(&self) -> Option<Keys> {
        let key = |flag: u8, value: [u8; 16]| (self.flags & flag != 0).then_some(value);
        (self.flags & Self::PRESENT != 0).then(|| Keys {
            ltk: key(Self::LTK, self.ltk),
            ediv: self.ediv,
            rand: self.rand,
            irk: key(Self::IRK, self.irk),
            csrk: key(Self::CSRK, self.csrk),
            sign_counter: self.sign_counter,
        })
    }
    fn from_keys(keys: Option<&Keys>) -> Self {
        let Some(keys) = keys else {
            return Self::default();
        };
        let mut native = Self {
            flags: Self::PRESENT,
            ediv: keys.ediv,
            rand: keys.rand,
            sign_counter: keys.sign_counter,
            ..Self::default()
        };
        for (flag, key, to) in [
            (Self::LTK, keys.ltk, &mut native.ltk),
            (Self::IRK, keys.irk, &mut native.irk),
            (Self::CSRK, keys.csrk, &mut native.csrk),
        ] {
            if let Some(key) = key {
                *to = key;
                native.flags |= flag;
            }
        }
        native
    }
}

impl Native {
    pub const AUTHENTICATED: u8 = 1;
    pub const AUTHORIZED: u8 = 2;
    pub const SECURE_CONNECTIONS: u8 = 4;
    pub const FEATURE: u8 = 8;
}

impl From<&Bond> for Native {
    fn from(bond: &Bond) -> Self {
        let mut native = Self {
            address: bond.address,
            random: bond.random.into(),
            key_size: bond.key_size,
            flags: (if bond.authenticated {
                Self::AUTHENTICATED
            } else {
                0
            }) | (if bond.authorized { Self::AUTHORIZED } else { 0 })
                | (if bond.secure_connections {
                    Self::SECURE_CONNECTIONS
                } else {
                    0
                })
                | (if bond.feature.is_some() {
                    Self::FEATURE
                } else {
                    0
                }),
            feature: bond.feature.unwrap_or(0),
            database_length: bond.database.len() as u8,
            local: NativeKeys::from_keys(bond.local.as_ref()),
            peer: NativeKeys::from_keys(bond.peer.as_ref()),
            ..Self::default()
        };
        native.database[..bond.database.len()].copy_from_slice(&bond.database);
        for (to, from) in native.subscriptions.iter_mut().zip(&bond.subscriptions) {
            *to = NativeSubscription {
                handle: from.handle,
                value: from.value,
            };
        }
        native
    }
}

impl From<&Native> for Bond {
    fn from(native: &Native) -> Self {
        let length = usize::from(native.database_length).min(DATABASE_BYTES);
        Self {
            address: native.address,
            random: native.random != 0,
            key_size: native.key_size,
            authenticated: native.flags & Native::AUTHENTICATED != 0,
            authorized: native.flags & Native::AUTHORIZED != 0,
            secure_connections: native.flags & Native::SECURE_CONNECTIONS != 0,
            local: native.local.keys(),
            peer: native.peer.keys(),
            feature: (native.flags & Native::FEATURE != 0).then_some(native.feature),
            database: Bounded::from_slice(&native.database[..length]).unwrap(),
            subscriptions: native
                .subscriptions
                .iter()
                .filter(|s| s.handle != 0)
                .map(|s| Subscription {
                    handle: s.handle,
                    value: s.value,
                })
                .collect(),
        }
    }
}

/// A combination's saved bonds.
#[derive(Debug, Default, Eq, PartialEq)]
pub struct Loaded {
    /// At most [`MAX_BONDS`], from the oldest pairing to the newest, with their IDs.
    pub bonds: Vec<(u64, Bond)>,
    /// The ID the next new bond gets: above every saved one.
    pub next_id: u64,
}

/// Reads the saved bonds of `devices`. Removes, as far as it can, record directories without a
/// bond, bonds that break the schema's rules, older bonds of an address saved again, and the
/// oldest bonds beyond [`MAX_BONDS`].
pub fn load(store: &mut impl Store, devices: Devices) -> Result<Loaded, Error> {
    let mut ids: Vec<u64> = store
        .list(&bonds_directory(devices))?
        .iter()
        .filter_map(|name| store::parse_id(name))
        .collect();
    ids.sort_unstable();
    let next_id = ids.last().map_or(1, |id| id + 1);
    let mut bonds: Vec<(u64, Bond)> = Vec::new();
    bonds
        .try_reserve_exact(ids.len())
        .map_err(|_| Error::Unavailable)?;
    let mut discard = Vec::new();
    for id in ids {
        let bond = match store.load(&bond_path(devices, id))? {
            Some(bytes) => store::decode(&bytes).and_then(|saved| Bond::from_saved(&saved)),
            None => Err(Error::Missing),
        };
        match bond {
            Ok(bond) => {
                if let Some(index) = bonds
                    .iter()
                    .position(|(_, b)| b.address == bond.address && b.random == bond.random)
                {
                    discard.push(bonds.remove(index).0);
                }
                bonds.push((id, bond));
            }
            Err(Error::Missing | Error::Corrupt) => discard.push(id),
            Err(e) => return Err(e),
        }
    }
    let extra = bonds.len().saturating_sub(MAX_BONDS);
    discard.extend(bonds.drain(..extra).map(|(id, _)| id));
    // Cleanup is best effort: a record that stays is found and removed again next time.
    for id in discard {
        let _ = store.remove(&bond_path(devices, id));
    }
    Ok(Loaded { bonds, next_id })
}

/// What an entry of a Bluetooth stack's bond table holds.
#[derive(Clone, Debug, Eq, PartialEq)]
// Passed by value, one entry at a time, and never stored.
#[allow(clippy::large_enum_variant)]
pub enum Entry {
    Empty,
    /// A pairing whose keys the stack has not all written yet. The saved bonds the entry holds or
    /// replaced stay until it holds a bond or is emptied.
    Incomplete,
    Bond(Bond),
}

impl From<Option<Bond>> for Entry {
    fn from(bond: Option<Bond>) -> Self {
        bond.map_or(Self::Empty, Self::Bond)
    }
}

#[derive(Clone, Copy, Debug)]
struct Dirty {
    first: u64,
    last: u64,
    /// Saved at the next save, without waiting for changes to pause.
    urgent: bool,
    /// The last try failed, so the next waits for the backoff.
    failed: bool,
}

#[derive(Clone, Copy, Debug, Default)]
struct Slot {
    /// The ID of the bond saved for this entry.
    id: Option<u64>,
    /// A bond this entry's pairing replaced, removed once the new bond is saved.
    replaced: Option<u64>,
    dirty: Option<Dirty>,
}

#[derive(Clone, Copy, Debug)]
struct Removal {
    id: u64,
    failed: bool,
}

/// Which saved bond each entry of a Bluetooth stack's bond table holds, and the entries waiting
/// to be saved.
///
/// A new pairing, changed keys and a removal are saved at the next [`Table::save`], before the
/// application hears of them. An entry holding an incomplete pairing waits until it holds a bond,
/// which is then saved and replaces the saved bond the entry held, or until it is emptied, which
/// removes that saved bond. The feature report, the database identifier and subscriptions change
/// in bursts while a Target sets up its connection, so they are saved once an entry has had no
/// change for 250 ms, and at most 2 s after its first change. A save or removal that failed is tried
/// again after 5 s, and a later change to its entry is tried at once.
pub struct Table {
    devices: Devices,
    slots: [Slot; MAX_BONDS],
    removals: Vec<Removal>,
    next_id: u64,
    retry_at: Option<u64>,
}

impl Table {
    /// A table for the bonds of `devices` whose next new bond gets `next_id`.
    pub fn new(devices: Devices, next_id: u64) -> Self {
        Self {
            devices,
            slots: [Slot::default(); MAX_BONDS],
            removals: Vec::new(),
            next_id: next_id.max(1),
            retry_at: None,
        }
    }

    /// The stack's entry `slot` holds the saved bond `id`.
    pub fn assign(&mut self, slot: usize, id: u64) {
        if let Some(s) = self.slots.get_mut(slot) {
            s.id = Some(id);
        }
    }

    fn mark(&mut self, slot: usize, now: u64, urgent: bool) {
        let Some(s) = self.slots.get_mut(slot) else {
            return;
        };
        let dirty = s.dirty.get_or_insert(Dirty {
            first: now,
            last: now,
            urgent,
            failed: false,
        });
        dirty.last = now;
        dirty.urgent |= urgent;
        dirty.failed = false;
    }

    /// A new pairing replaced whatever entry `slot` held.
    pub fn added(&mut self, slot: usize, now: u64) {
        if let Some(s) = self.slots.get_mut(slot)
            && let Some(id) = s.id.take()
        {
            if s.replaced.is_some() {
                // The entry's last pairing never reached storage; the bond it replaced stays until
                // this one does.
                self.removals.push(Removal { id, failed: false });
            } else {
                s.replaced = Some(id);
            }
        }
        self.mark(slot, now, true);
    }

    /// Entry `slot` changed its keys or was removed.
    pub fn changed(&mut self, slot: usize, now: u64) {
        self.mark(slot, now, true);
    }

    /// Entry `slot` changed its feature report, database identifier or subscriptions.
    pub fn updated(&mut self, slot: usize, now: u64) {
        self.mark(slot, now, false);
    }

    /// The pairing in entry `slot` is being removed: the saved bonds the entry holds or replaced are
    /// removed at the next save, whatever the entry holds until the stack has emptied it.
    pub fn forget(&mut self, slot: usize) {
        if let Some(s) = self.slots.get_mut(slot) {
            for id in [s.id.take(), s.replaced.take()].into_iter().flatten() {
                self.removals.push(Removal { id, failed: false });
            }
            s.dirty = None;
        }
    }

    /// Whether one of the saved bonds `ids` waits to be removed.
    pub fn removing(&self, ids: &[u64]) -> bool {
        self.removals.iter().any(|r| ids.contains(&r.id))
    }

    /// Whether anything waits to be saved or removed.
    pub fn pending(&self) -> bool {
        !self.removals.is_empty() || self.slots.iter().any(|s| s.dirty.is_some())
    }

    /// The IDs of the saved bonds entry `slot` holds or replaced.
    pub fn held(&self, slot: usize) -> Vec<u64> {
        self.slots
            .get(slot)
            .map(|s| s.id.into_iter().chain(s.replaced).collect())
            .unwrap_or_default()
    }

    /// Whether entry `slot` holds a saved bond whose pairing and keys have no change waiting. A
    /// waiting feature report, database identifier or subscription change does not count.
    pub fn saved(&self, slot: usize) -> bool {
        self.slots
            .get(slot)
            .is_some_and(|s| s.id.is_some() && !s.dirty.is_some_and(|d| d.urgent))
    }

    /// Whether one of the saved bonds `ids` of entry `slot` waits to be removed, or the entry waits
    /// to be saved.
    pub fn holds(&self, slot: usize, ids: &[u64]) -> bool {
        self.slots.get(slot).is_some_and(|s| s.dirty.is_some())
            || self.removals.iter().any(|r| ids.contains(&r.id))
    }

    /// Saves the entries that are due and removes the bonds they no longer hold; with `all`,
    /// everything waiting, whatever the timing. `export` gives what an entry holds. Returns the
    /// first failure of what was tried; what failed, and an entry holding an incomplete pairing,
    /// waits to be tried again.
    pub fn save(
        &mut self,
        store: &mut impl Store,
        now: u64,
        all: bool,
        mut export: impl FnMut(usize) -> Entry,
    ) -> Result<(), Error> {
        let backoff = !all && self.retry_at.is_some_and(|at| now < at);
        let mut result = Ok(());
        for slot in 0..MAX_BONDS {
            let Some(dirty) = self.slots[slot].dirty else {
                continue;
            };
            let due = if dirty.urgent {
                dirty.first
            } else {
                (dirty.last + QUIET_MS).min(dirty.first + LATEST_MS)
            };
            if !all && (now < due || backoff && dirty.failed) {
                continue;
            }
            let s = &mut self.slots[slot];
            let bond = match export(slot) {
                Entry::Bond(bond) => bond,
                Entry::Incomplete => continue,
                Entry::Empty => {
                    for id in [s.id.take(), s.replaced.take()].into_iter().flatten() {
                        self.removals.push(Removal { id, failed: false });
                    }
                    s.dirty = None;
                    continue;
                }
            };
            let id = *s.id.get_or_insert_with(|| {
                let id = self.next_id;
                self.next_id += 1;
                id
            });
            match store::encode(&bond.to_saved())
                .and_then(|bytes| store.save(&bond_path(self.devices, id), &bytes))
            {
                Ok(()) => {
                    s.dirty = None;
                    if let Some(id) = s.replaced.take() {
                        self.removals.push(Removal { id, failed: false });
                    }
                }
                Err(e) => {
                    if let Some(dirty) = &mut s.dirty {
                        dirty.failed = true;
                    }
                    if result.is_ok() {
                        result = Err(e);
                    }
                }
            }
        }
        let devices = self.devices;
        self.removals.retain_mut(|removal| {
            if backoff && removal.failed {
                return true;
            }
            match store.remove(&bond_path(devices, removal.id)) {
                Ok(()) => false,
                Err(e) => {
                    removal.failed = true;
                    if result.is_ok() {
                        result = Err(e);
                    }
                    true
                }
            }
        });
        if result.is_err() {
            self.retry_at = Some(now + RETRY_MS);
        } else if !backoff {
            self.retry_at = None;
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::memory::Memory;

    const BOTH: Devices = Devices {
        keyboard: true,
        mouse: true,
    };
    const KEYBOARD: Devices = Devices {
        keyboard: true,
        mouse: false,
    };

    fn bond(last: u8) -> Bond {
        Bond {
            address: [1, 2, 3, 4, 5, last],
            random: true,
            key_size: 16,
            authenticated: true,
            authorized: false,
            secure_connections: true,
            local: Some(Keys {
                irk: Some([7; 16]),
                ..Keys::default()
            }),
            peer: Some(Keys {
                ltk: Some([last; 16]),
                ediv: 0x1234,
                rand: [1, 2, 3, 4, 5, 6, 7, 8],
                irk: Some([9; 16]),
                csrk: None,
                sign_counter: 0,
            }),
            feature: Some(5),
            database: Bounded::from_slice(&[0xaa; 16]).unwrap(),
            subscriptions: Bounded::from_slice(&[Subscription {
                handle: 0x20,
                value: 1,
            }])
            .unwrap(),
        }
    }

    fn save(store: &mut Memory, devices: Devices, id: u64, bond: &Bond) {
        store
            .save(
                &bond_path(devices, id),
                &store::encode(&bond.to_saved()).unwrap(),
            )
            .unwrap();
    }

    #[test]
    fn bonds_round_trip_through_their_records() {
        for bond in [
            bond(1),
            Bond {
                address: [1; 6],
                ..Bond::default()
            },
        ] {
            let bytes = store::encode(&bond.to_saved()).unwrap();
            // Every bond has its address, so no record is empty.
            assert!(!bytes.is_empty());
            let saved: saved::Bond = store::decode(&bytes).unwrap();
            assert_eq!(Bond::from_saved(&saved).unwrap(), bond);
        }
    }

    #[test]
    fn bonds_round_trip_through_the_native_layout() {
        for bond in [
            bond(1),
            Bond {
                address: [1; 6],
                peer: Some(Keys::default()),
                ..Bond::default()
            },
        ] {
            assert_eq!(Bond::from(&Native::from(&bond)), bond);
        }
        let native = Native::from(&bond(3));
        assert_eq!(native.peer.flags, NativeKeys::PRESENT | 3);
        assert_eq!(native.local.flags, NativeKeys::PRESENT | NativeKeys::IRK);
        assert_eq!(
            native.flags,
            Native::AUTHENTICATED | Native::SECURE_CONNECTIONS | Native::FEATURE
        );
        assert_eq!(native.subscriptions[1], NativeSubscription::default());
        // A side without kept keys has no keys, and a database length beyond the array is cut.
        let bond = Bond::from(&Native {
            database_length: 200,
            ..Native::default()
        });
        assert_eq!((bond.local, bond.peer), (None, None));
        assert_eq!(bond.database.len(), DATABASE_BYTES);
    }

    #[test]
    fn records_that_break_the_rules_are_corrupt() {
        let good = bond(1).to_saved();
        let mut cases = Vec::new();
        let mut b = good.clone();
        b.address = alloc::vec![1; 5];
        cases.push(b);
        let mut b = good.clone();
        b.key_size = 17;
        cases.push(b);
        let mut b = good.clone();
        b.peer.as_mut().unwrap().ltk = alloc::vec![1; 15];
        cases.push(b);
        let mut b = good.clone();
        b.peer.as_mut().unwrap().rand = alloc::vec![1; 7];
        cases.push(b);
        let mut b = good.clone();
        b.peer.as_mut().unwrap().ediv = 0x10000;
        cases.push(b);
        let mut b = good.clone();
        b.feature = Some(256);
        cases.push(b);
        let mut b = good.clone();
        b.database = alloc::vec![0; 17];
        cases.push(b);
        let mut b = good.clone();
        b.subscriptions[0].handle = 0;
        cases.push(b);
        let mut b = good.clone();
        b.subscriptions = (1..=9)
            .map(|handle| saved::Subscription { handle, value: 1 })
            .collect();
        cases.push(b);
        for case in cases {
            assert_eq!(Bond::from_saved(&case), Err(Error::Corrupt), "{case:?}");
        }
    }

    #[test]
    fn identities_are_created_once_per_combination() {
        let mut store = Memory::default();
        let mut counter = 0;
        let mut random = || {
            counter += 1;
            counter
        };
        let first = Identity::load_or_create(&mut store, BOTH, &mut random).unwrap();
        assert_eq!(first.irk, Identity::derive_irk(&first.ir));
        assert_ne!(first.ir, first.er);
        assert!(
            store
                .files
                .contains_key("/combinations/keyboard-mouse/identity.pb")
        );
        assert_eq!(
            Identity::load_or_create(&mut store, BOTH, &mut random).unwrap(),
            first
        );
        let keyboard = Identity::load_or_create(&mut store, KEYBOARD, &mut random).unwrap();
        assert_ne!(keyboard, first);
        assert!(
            store
                .files
                .contains_key("/combinations/keyboard/identity.pb")
        );
    }

    #[test]
    fn identity_roots_that_do_not_match_or_are_lost_are_corrupt() {
        let mut store = Memory::default();
        let mut random = || 3;
        Identity::load_or_create(&mut store, BOTH, &mut random).unwrap();
        let path = identity_path(BOTH);
        let mut saved: saved::Identity = store::decode(&store.files[&path]).unwrap();
        saved.irk[0] ^= 1;
        store
            .files
            .insert(path.clone(), store::encode(&saved).unwrap());
        assert_eq!(
            Identity::load_or_create(&mut store, BOTH, &mut random),
            Err(Error::Corrupt)
        );
        // Bonds without their identity are never given a new one.
        store.files.remove(&path);
        save(&mut store, BOTH, 1, &bond(1));
        assert_eq!(
            Identity::load_or_create(&mut store, BOTH, &mut random),
            Err(Error::Corrupt)
        );
        assert!(!store.files.contains_key(&path));
    }

    #[test]
    fn loading_keeps_the_newest_bonds_and_cleans_up() {
        let mut store = Memory::default();
        assert_eq!(
            load(&mut store, BOTH).unwrap(),
            Loaded {
                bonds: Vec::new(),
                next_id: 1
            }
        );
        for id in 1..=10u64 {
            save(&mut store, BOTH, id * 2, &bond(id as u8));
        }
        // The same Target saved again, an empty record, a corrupt one and a stray name.
        save(&mut store, BOTH, 30, &bond(10));
        store.files.insert(bond_path(BOTH, 31), Vec::new());
        store.files.insert(bond_path(BOTH, 32), alloc::vec![0xff]);
        store.files.insert(
            "/combinations/keyboard-mouse/bonds/x".into(),
            alloc::vec![1],
        );
        // Another combination's bonds stay apart.
        save(&mut store, KEYBOARD, 40, &bond(1));
        let loaded = load(&mut store, BOTH).unwrap();
        assert_eq!(loaded.next_id, 33);
        let ids: Vec<u64> = loaded.bonds.iter().map(|(id, _)| *id).collect();
        assert_eq!(ids, [6, 8, 10, 12, 14, 16, 18, 30]);
        assert_eq!(loaded.bonds[7].1, bond(10));
        for id in [2, 4, 20, 31, 32] {
            assert!(!store.files.contains_key(&bond_path(BOTH, id)), "{id}");
        }
        assert!(store.files.contains_key(&bond_path(KEYBOARD, 40)));
        assert_eq!(load(&mut store, KEYBOARD).unwrap().bonds.len(), 1);
    }

    #[test]
    fn loading_fails_when_a_bond_cannot_be_read() {
        let mut store = Memory::default();
        save(&mut store, BOTH, 1, &bond(1));
        store.fail = true;
        assert_eq!(load(&mut store, BOTH), Err(Error::Io));
    }

    #[test]
    fn the_table_saves_pairings_at_once_and_coalesces_updates() {
        let mut store = Memory::default();
        let mut table = Table::new(BOTH, 5);
        let mut native: [Option<Bond>; MAX_BONDS] = Default::default();
        native[0] = Some(bond(1));
        table.added(0, 100);
        assert!(!table.saved(0));
        table
            .save(&mut store, 100, false, |s| native[s].clone().into())
            .unwrap();
        assert!(store.files.contains_key(&bond_path(BOTH, 5)));
        assert_eq!(store.writes, 1);
        assert!(table.saved(0));

        // Subscriptions, the feature and the database wait for changes to pause.
        native[0].as_mut().unwrap().feature = Some(1);
        table.updated(0, 200);
        assert!(table.saved(0), "a deferred update leaves the pairing saved");
        table
            .save(&mut store, 300, false, |s| native[s].clone().into())
            .unwrap();
        native[0].as_mut().unwrap().feature = Some(2);
        table.updated(0, 400);
        table
            .save(&mut store, 600, false, |s| native[s].clone().into())
            .unwrap();
        assert_eq!(store.writes, 1);
        table
            .save(&mut store, 650, false, |s| native[s].clone().into())
            .unwrap();
        assert_eq!(store.writes, 2);
        assert!(!table.pending());

        // Changes that keep coming are saved 2 s after the first.
        for at in (1000..=3000).step_by(100) {
            native[0].as_mut().unwrap().feature = Some((at / 100) as u8);
            table.updated(0, at);
            table
                .save(&mut store, at, false, |s| native[s].clone().into())
                .unwrap();
        }
        assert_eq!(store.writes, 3);

        // A new pairing in the entry replaces the old bond once it is saved.
        native[0] = Some(bond(2));
        table.added(0, 4000);
        table
            .save(&mut store, 4000, false, |s| native[s].clone().into())
            .unwrap();
        assert!(!store.files.contains_key(&bond_path(BOTH, 5)));
        let loaded = load(&mut store, BOTH).unwrap();
        assert_eq!(loaded.bonds, [(6, bond(2))]);

        // A removal deletes the record.
        native[0] = None;
        table.changed(0, 5000);
        table
            .save(&mut store, 5000, false, |s| native[s].clone().into())
            .unwrap();
        assert!(store.files.is_empty());
        assert!(!table.pending());
    }

    #[test]
    fn an_incomplete_pairing_keeps_the_saved_bond_until_it_completes_or_is_emptied() {
        let mut store = Memory::default();
        let mut table = Table::new(BOTH, 5);
        table.assign(0, 3);
        save(&mut store, BOTH, 3, &bond(1));
        // The stack clears the entry's keys as the Target pairs again.
        table.changed(0, 0);
        table
            .save(&mut store, 0, true, |_| Entry::Incomplete)
            .unwrap();
        assert!(store.files.contains_key(&bond_path(BOTH, 3)));
        assert!(table.pending());
        assert!(table.holds(0, &[3]));
        // Its new keys are written one side after the other.
        table.added(0, 10);
        table
            .save(&mut store, 10, false, |_| Entry::Incomplete)
            .unwrap();
        assert_eq!(table.held(0), [3]);
        assert!(store.files.contains_key(&bond_path(BOTH, 3)));
        table.changed(0, 20);
        table
            .save(&mut store, 20, false, |_| Entry::Bond(bond(2)))
            .unwrap();
        assert_eq!(load(&mut store, BOTH).unwrap().bonds, [(5, bond(2))]);
        assert!(!table.pending());

        // A pairing that never completes removes the saved bond once its entry is emptied.
        table.changed(0, 30);
        table
            .save(&mut store, 30, false, |_| Entry::Incomplete)
            .unwrap();
        assert!(store.files.contains_key(&bond_path(BOTH, 5)));
        table.changed(0, 40);
        table.save(&mut store, 40, false, |_| Entry::Empty).unwrap();
        assert!(store.files.is_empty());
        assert!(!table.pending());
    }

    #[test]
    fn a_forgotten_entry_removes_its_saved_bond_before_the_stack_empties_it() {
        let mut store = Memory::default();
        let mut table = Table::new(BOTH, 5);
        table.assign(0, 3);
        save(&mut store, BOTH, 3, &bond(1));
        table.forget(0);
        store.fail = true;
        // The stack still holds the pairing, which is not saved again.
        assert_eq!(
            table.save(&mut store, 0, false, |_| Entry::Incomplete),
            Err(Error::Io)
        );
        assert!(table.removing(&[3]));
        store.fail = false;
        table
            .save(&mut store, 5000, false, |_| Entry::Incomplete)
            .unwrap();
        assert!(!table.removing(&[3]));
        assert!(store.files.is_empty());
        // Once the stack empties the entry, nothing is left to remove or write.
        table.changed(0, 6000);
        table
            .save(&mut store, 6000, false, |_| Entry::Empty)
            .unwrap();
        assert!(!table.pending());
        // Only the bond saved before it was forgotten was ever written.
        assert_eq!(store.writes, 1);
    }

    #[test]
    fn the_table_retries_failures_and_flushes_everything() {
        let mut store = Memory::default();
        let mut table = Table::new(BOTH, 1);
        let native = [Some(bond(1)), Some(bond(2))];
        table.assign(1, 9);
        table.added(0, 0);
        store.fail = true;
        assert_eq!(
            table.save(&mut store, 0, false, |s| native[s].clone().into()),
            Err(Error::Io)
        );
        store.fail = false;
        assert!(table.holds(0, &[]));
        // What failed is not tried again before the backoff ends.
        table
            .save(&mut store, 100, false, |s| native[s].clone().into())
            .unwrap();
        assert!(store.files.is_empty());
        // A change made meanwhile is tried at once: here the other entry's removal.
        store.files.insert(bond_path(BOTH, 9), alloc::vec![1]);
        table.changed(1, 200);
        table
            .save(&mut store, 200, false, |_| Entry::Empty)
            .unwrap();
        assert!(store.files.is_empty());
        assert!(!table.holds(1, &[9]));
        assert!(table.holds(0, &[]));
        table
            .save(&mut store, 5000, false, |s| native[s].clone().into())
            .unwrap();
        assert!(store.files.contains_key(&bond_path(BOTH, 1)));
        assert!(!table.holds(0, &table.held(0)));
        table.assign(1, 9);

        // Deferred updates are written at once when everything is flushed.
        table.updated(1, 6000);
        table
            .save(&mut store, 6000, true, |s| native[s].clone().into())
            .unwrap();
        assert!(store.files.contains_key(&bond_path(BOTH, 9)));
        assert!(!table.pending());
    }
}
