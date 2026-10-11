use debonair_core::store::{self, Error, Store};
use embedded_storage_async::nor_flash::{ErrorType, NorFlash, NorFlashErrorKind, ReadNorFlash};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

/// 30 LittleFS blocks after the guard, from 4 KiB into a 128 KiB flash.
type Storage = debonair_storage::Storage<Flash, 30>;
const RANGE: std::ops::Range<u32> = 4096..128 * 1024;
const IDENTITY: [u8; 32] = [1; 32];

#[derive(Clone)]
struct Flash(Rc<RefCell<Vec<u8>>>);
impl Flash {
    fn blank() -> Self {
        Self(Rc::new(RefCell::new(vec![0xff; 128 * 1024])))
    }
    fn copy(&self) -> Self {
        Self(Rc::new(RefCell::new(self.0.borrow().clone())))
    }
}
impl ErrorType for Flash {
    type Error = NorFlashErrorKind;
}
impl ReadNorFlash for Flash {
    const READ_SIZE: usize = 1;
    async fn read(&mut self, offset: u32, bytes: &mut [u8]) -> Result<(), Self::Error> {
        let data = self.0.borrow();
        let source = data
            .get(offset as usize..offset as usize + bytes.len())
            .ok_or(NorFlashErrorKind::OutOfBounds)?;
        bytes.copy_from_slice(source);
        Ok(())
    }
    fn capacity(&self) -> usize {
        self.0.borrow().len()
    }
}
impl NorFlash for Flash {
    const WRITE_SIZE: usize = 4;
    const ERASE_SIZE: usize = 4096;
    async fn erase(&mut self, from: u32, to: u32) -> Result<(), Self::Error> {
        if !(from as usize).is_multiple_of(Self::ERASE_SIZE)
            || !(to as usize).is_multiple_of(Self::ERASE_SIZE)
        {
            return Err(NorFlashErrorKind::NotAligned);
        }
        self.0
            .borrow_mut()
            .get_mut(from as usize..to as usize)
            .ok_or(NorFlashErrorKind::OutOfBounds)?
            .fill(0xff);
        Ok(())
    }
    async fn write(&mut self, offset: u32, bytes: &[u8]) -> Result<(), Self::Error> {
        let mut data = self.0.borrow_mut();
        let target = data
            .get_mut(offset as usize..offset as usize + bytes.len())
            .ok_or(NorFlashErrorKind::OutOfBounds)?;
        for (old, new) in target.iter_mut().zip(bytes) {
            if *old & *new != *new {
                return Err(NorFlashErrorKind::Other);
            }
            *old &= *new;
        }
        Ok(())
    }
}

/// Flash that fails every access once `remaining` mutations have happened, as when power is cut.
struct CutFlash {
    flash: Flash,
    remaining: usize,
    cut: bool,
}
impl CutFlash {
    fn new(flash: Flash, remaining: usize) -> Self {
        Self {
            flash,
            remaining,
            cut: false,
        }
    }
    fn mutation(&mut self) -> Result<(), NorFlashErrorKind> {
        if self.remaining == 0 {
            self.cut = true;
            return Err(NorFlashErrorKind::Other);
        }
        self.remaining -= 1;
        Ok(())
    }
}
impl ErrorType for CutFlash {
    type Error = NorFlashErrorKind;
}
impl ReadNorFlash for CutFlash {
    const READ_SIZE: usize = 1;
    fn capacity(&self) -> usize {
        self.flash.capacity()
    }
    async fn read(&mut self, offset: u32, bytes: &mut [u8]) -> Result<(), Self::Error> {
        if self.cut {
            return Err(NorFlashErrorKind::Other);
        }
        self.flash.read(offset, bytes).await
    }
}
impl NorFlash for CutFlash {
    const WRITE_SIZE: usize = 4;
    const ERASE_SIZE: usize = 4096;
    async fn write(&mut self, off: u32, bytes: &[u8]) -> Result<(), Self::Error> {
        self.mutation()?;
        self.flash.write(off, bytes).await
    }
    async fn erase(&mut self, from: u32, to: u32) -> Result<(), Self::Error> {
        self.mutation()?;
        self.flash.erase(from, to).await
    }
}

/// Counts reads and mutations, fails writes and erases while `fail_writes` is set and fails every
/// access while `fail` is set.
#[derive(Clone)]
struct Probe {
    flash: Flash,
    reads: Rc<Cell<usize>>,
    mutations: Rc<Cell<usize>>,
    fail: Rc<Cell<bool>>,
    fail_writes: Rc<Cell<bool>>,
}
impl Probe {
    fn new(flash: Flash) -> Self {
        Self {
            flash,
            reads: Rc::default(),
            mutations: Rc::default(),
            fail: Rc::default(),
            fail_writes: Rc::default(),
        }
    }
    fn mutation(&self) -> Result<(), NorFlashErrorKind> {
        if self.fail.get() || self.fail_writes.get() {
            return Err(NorFlashErrorKind::Other);
        }
        self.mutations.set(self.mutations.get() + 1);
        Ok(())
    }
}
impl ErrorType for Probe {
    type Error = NorFlashErrorKind;
}
impl ReadNorFlash for Probe {
    const READ_SIZE: usize = 1;
    fn capacity(&self) -> usize {
        self.flash.capacity()
    }
    async fn read(&mut self, offset: u32, bytes: &mut [u8]) -> Result<(), Self::Error> {
        self.reads.set(self.reads.get() + 1);
        if self.fail.get() {
            return Err(NorFlashErrorKind::Other);
        }
        self.flash.read(offset, bytes).await
    }
}
impl NorFlash for Probe {
    const WRITE_SIZE: usize = 4;
    const ERASE_SIZE: usize = 4096;
    async fn write(&mut self, off: u32, bytes: &[u8]) -> Result<(), Self::Error> {
        self.mutation()?;
        self.flash.write(off, bytes).await
    }
    async fn erase(&mut self, from: u32, to: u32) -> Result<(), Self::Error> {
        self.mutation()?;
        self.flash.erase(from, to).await
    }
}

fn provisioned() -> (Flash, Storage) {
    let flash = Flash::blank();
    let store = Storage::provision_blank(flash.clone(), RANGE, IDENTITY).unwrap();
    (flash, store)
}

#[test]
fn blank_flash_is_provisioned_and_reopens() {
    let flash = Flash::blank();
    let mut store = Storage::open_or_provision_blank(flash.clone(), RANGE, IDENTITY).unwrap();
    store::open(&mut store).unwrap();
    store.save("/adapter.pb", b"adapter").unwrap();
    // The guard holds the layout identity and the space before the range is never touched.
    assert_eq!(&flash.0.borrow()[4096..4096 + 32], &IDENTITY);
    assert!(flash.0.borrow()[4096 + 32..8192].iter().all(|&b| b == 0xff));
    assert!(flash.0.borrow()[..4096].iter().all(|&b| b == 0xff));
    drop(store);
    let mut store = Storage::open_or_provision_blank(flash, RANGE, IDENTITY).unwrap();
    store::open(&mut store).unwrap();
    assert_eq!(store.load("/adapter.pb").unwrap().unwrap(), b"adapter");
}

#[test]
fn unknown_or_partly_used_flash_is_never_provisioned_or_changed() {
    let flash = Flash::blank();
    assert!(matches!(
        Storage::open(flash.clone(), RANGE, IDENTITY),
        Err(Error::Unprovisioned)
    ));
    // A byte written anywhere in the range keeps a blank guard from being provisioned.
    flash.0.borrow_mut()[128 * 1024 - 1] = 0;
    let before = flash.0.borrow().clone();
    assert!(matches!(
        Storage::open_or_provision_blank(flash.clone(), RANGE, IDENTITY),
        Err(Error::Layout)
    ));
    assert_eq!(*flash.0.borrow(), before);
    // So does a guard holding something else, such as another layout's identity.
    flash.0.borrow_mut()[4096] = 7;
    let before = flash.0.borrow().clone();
    assert!(matches!(
        Storage::open_or_provision_blank(flash.clone(), RANGE, IDENTITY),
        Err(Error::Layout)
    ));
    assert_eq!(*flash.0.borrow(), before);
}

#[test]
fn another_layout_identity_is_refused_and_left_untouched() {
    let (flash, mut store) = provisioned();
    store.save("/adapter.pb", b"adapter").unwrap();
    drop(store);
    let before = flash.0.borrow().clone();
    assert!(matches!(
        Storage::open_or_provision_blank(flash.clone(), RANGE, [2; 32]),
        Err(Error::Layout)
    ));
    assert_eq!(*flash.0.borrow(), before);
}

#[test]
fn a_matching_guard_over_a_broken_filesystem_is_never_reformatted() {
    let flash = Flash::blank();
    flash.0.borrow_mut()[4096..4096 + 32].copy_from_slice(&IDENTITY);
    flash.0.borrow_mut()[8192..8192 + 64].fill(0);
    let before = flash.0.borrow().clone();
    assert!(Storage::open_or_provision_blank(flash.clone(), RANGE, IDENTITY).is_err());
    assert_eq!(*flash.0.borrow(), before);
}

#[test]
fn interrupted_provisioning_never_reformats_a_claimed_range() {
    let mut completed = false;
    for cut in 0..40 {
        let flash = Flash::blank();
        let result = debonair_storage::Storage::<_, 30>::provision_blank(
            CutFlash::new(flash.clone(), cut),
            RANGE,
            IDENTITY,
        );
        let claimed = flash.0.borrow()[RANGE.start as usize..]
            .iter()
            .any(|b| *b != 0xff);
        let before = flash.0.borrow().clone();
        let reopened = Storage::open_or_provision_blank(flash.clone(), RANGE, IDENTITY);
        if claimed {
            assert_eq!(*flash.0.borrow(), before, "cut {cut}");
        }
        if result.is_ok() {
            assert!(reopened.is_ok());
            completed = true;
            break;
        }
    }
    assert!(completed);
}

#[test]
fn rejects_a_range_of_the_wrong_size() {
    assert!(matches!(
        Storage::provision_blank(Flash::blank(), 4096..120 * 1024, IDENTITY),
        Err(Error::Bounds)
    ));
}

#[test]
fn records_are_saved_in_place_and_equal_bytes_are_not_written() {
    let flash = Flash::blank();
    Storage::provision_blank(flash.clone(), RANGE, IDENTITY).unwrap();
    let probe = Probe::new(flash.clone());
    let mut store =
        debonair_storage::Storage::<_, 30>::open(probe.clone(), RANGE, IDENTITY).unwrap();
    let path = "/combinations/keyboard/bonds/1/bond.pb";
    let data: Vec<u8> = (0..9000).map(|n| (n % 251) as u8).collect();
    store.save(path, &data).unwrap();
    let mutations = probe.mutations.get();
    store.save(path, &data).unwrap();
    assert_eq!(
        probe.mutations.get(),
        mutations,
        "equal bytes are not written"
    );
    // Contents of the same length that differ are written.
    let mut changed = data.clone();
    changed[8999] ^= 1;
    store.save(path, &changed).unwrap();
    assert!(probe.mutations.get() > mutations);
    for n in 0..300u32 {
        store.save("/adapter.pb", &n.to_le_bytes()).unwrap();
    }
    drop(store);
    let mut store = Storage::open(flash, RANGE, IDENTITY).unwrap();
    assert_eq!(store.load(path).unwrap().unwrap(), changed);
    assert_eq!(
        store.load("/adapter.pb").unwrap().unwrap(),
        299u32.to_le_bytes()
    );
    // A record is saved in place, beside nothing else.
    assert_eq!(
        store.list("/combinations/keyboard/bonds/1").unwrap(),
        ["bond.pb"]
    );
    assert!(store.load("/../adapter.pb").is_err());
    assert_eq!(store.save("/adapter.pb", b""), Err(Error::Bounds));
}

#[test]
fn an_empty_record_file_holds_no_record() {
    let (_, mut store) = provisioned();
    store.replace_file("/format.pb", b"").unwrap();
    assert_eq!(store.load("/format.pb").unwrap(), None);
    // The root record is written over an interrupted creation of it.
    store::open(&mut store).unwrap();
    assert!(store.load("/format.pb").unwrap().is_some());
    store.replace_file("/adapter.pb", b"").unwrap();
    store.save("/adapter.pb", b"adapter").unwrap();
    assert_eq!(store.load("/adapter.pb").unwrap().unwrap(), b"adapter");
}

#[test]
fn removing_a_record_removes_its_record_directory() {
    let (_, mut store) = provisioned();
    store
        .save("/combinations/mouse/bonds/7/bond.pb", b"bond")
        .unwrap();
    store
        .save("/combinations/mouse/identity.pb", b"identity")
        .unwrap();
    let mut names = store.list("/combinations/mouse").unwrap();
    names.sort();
    assert_eq!(names, ["bonds", "identity.pb"]);
    store.remove("/combinations/mouse/bonds/7/bond.pb").unwrap();
    assert!(store.list("/combinations/mouse/bonds").unwrap().is_empty());
    assert!(store.directory_exists("/combinations/mouse/bonds").unwrap());
    // Removing what is not there succeeds; a directory that is not a record directory stays.
    store.remove("/combinations/mouse/bonds/7/bond.pb").unwrap();
    store.remove("/missing/bond.pb").unwrap();
    store.remove("/combinations/mouse/identity.pb").unwrap();
    assert!(store.directory_exists("/combinations/mouse").unwrap());
    assert!(store.list("/missing").unwrap().is_empty());
}

#[test]
fn interrupted_replacements_preserve_the_old_or_new_record() {
    // Inline and block-backed contents, growing and shrinking.
    for (old, new) in [(7000, 11000), (11000, 300), (100, 200), (300, 5000)] {
        let (flash, mut store) = provisioned();
        let old = vec![13; old];
        let new = vec![27; new];
        store.save("/adapter.pb", &old).unwrap();
        drop(store);
        let mut completed = false;
        for cut in 0..100 {
            let flash = flash.copy();
            let mut store = debonair_storage::Storage::<_, 30>::open(
                CutFlash::new(flash.clone(), cut),
                RANGE,
                IDENTITY,
            )
            .unwrap();
            let result = store.save("/adapter.pb", &new);
            drop(store);
            let mut reopened = Storage::open(flash, RANGE, IDENTITY).unwrap();
            let data = reopened.load("/adapter.pb").unwrap().unwrap();
            assert!(data == old || data == new, "cut {cut}");
            if result.is_ok() {
                assert_eq!(data, new);
                completed = true;
                break;
            }
        }
        assert!(completed);
    }
}

#[test]
fn interrupted_creations_leave_no_record_or_the_complete_record() {
    let path = "/combinations/mouse/bonds/2/bond.pb";
    for size in [200, 6000] {
        let (flash, mut store) = provisioned();
        store
            .save("/combinations/mouse/bonds/1/bond.pb", b"saved")
            .unwrap();
        drop(store);
        let new = vec![27; size];
        let mut completed = false;
        let mut empty = false;
        for cut in 0..100 {
            let flash = flash.copy();
            let mut store = debonair_storage::Storage::<_, 30>::open(
                CutFlash::new(flash.clone(), cut),
                RANGE,
                IDENTITY,
            )
            .unwrap();
            let result = store.save(path, &new);
            drop(store);
            let mut reopened = Storage::open(flash, RANGE, IDENTITY).unwrap();
            empty |= reopened.file_size(path).unwrap() == Some(0);
            match reopened.load(path).unwrap() {
                None => {}
                Some(bytes) => assert_eq!(bytes, new, "cut {cut}"),
            }
            assert_eq!(
                reopened
                    .load("/combinations/mouse/bonds/1/bond.pb")
                    .unwrap()
                    .unwrap(),
                b"saved"
            );
            if result.is_ok() {
                assert!(reopened.load(path).unwrap().is_some());
                completed = true;
                break;
            }
        }
        assert!(completed);
        assert!(empty, "some interruption leaves the created file empty");
    }
}

#[test]
fn interrupted_removals_never_restore_a_removed_record() {
    let (flash, mut store) = provisioned();
    let path = "/combinations/mouse/bonds/1/bond.pb";
    store.save(path, b"bond").unwrap();
    drop(store);
    let mut completed = false;
    for cut in 0..40 {
        let flash = flash.copy();
        let mut store = debonair_storage::Storage::<_, 30>::open(
            CutFlash::new(flash.clone(), cut),
            RANGE,
            IDENTITY,
        )
        .unwrap();
        let result = store.remove(path);
        drop(store);
        let mut reopened = Storage::open(flash, RANGE, IDENTITY).unwrap();
        let size = reopened.file_size(path).unwrap();
        assert!(size == Some(4) || size.is_none(), "cut {cut}");
        if result.is_ok() {
            assert!(size.is_none());
            completed = true;
            break;
        }
    }
    assert!(completed);
}

/// Flash whose one mutation numbered `fail_at` fails, and every other access succeeds.
struct OneFailure {
    flash: Flash,
    mutations: usize,
    fail_at: usize,
}
impl ErrorType for OneFailure {
    type Error = NorFlashErrorKind;
}
impl ReadNorFlash for OneFailure {
    const READ_SIZE: usize = 1;
    fn capacity(&self) -> usize {
        self.flash.capacity()
    }
    async fn read(&mut self, offset: u32, bytes: &mut [u8]) -> Result<(), Self::Error> {
        self.flash.read(offset, bytes).await
    }
}
impl OneFailure {
    fn mutation(&mut self) -> Result<(), NorFlashErrorKind> {
        self.mutations += 1;
        if self.mutations - 1 == self.fail_at {
            Err(NorFlashErrorKind::Other)
        } else {
            Ok(())
        }
    }
}
impl NorFlash for OneFailure {
    const WRITE_SIZE: usize = 4;
    const ERASE_SIZE: usize = 4096;
    async fn write(&mut self, off: u32, bytes: &[u8]) -> Result<(), Self::Error> {
        self.mutation()?;
        self.flash.write(off, bytes).await
    }
    async fn erase(&mut self, from: u32, to: u32) -> Result<(), Self::Error> {
        self.mutation()?;
        self.flash.erase(from, to).await
    }
}

#[test]
fn a_failed_creation_reclaims_its_empty_file_and_directory() {
    let (flash, mut store) = provisioned();
    store
        .save("/combinations/mouse/bonds/1/bond.pb", b"saved")
        .unwrap();
    drop(store);
    let path = "/combinations/mouse/bonds/2/bond.pb";
    let mut completed = false;
    for fail_at in 0..100 {
        let flash = flash.copy();
        let mut store = debonair_storage::Storage::<_, 30>::open(
            OneFailure {
                flash: flash.clone(),
                mutations: 0,
                fail_at,
            },
            RANGE,
            IDENTITY,
        )
        .unwrap();
        let result = store.save(path, &[27; 6000]);
        drop(store);
        let mut left = Storage::open(flash, RANGE, IDENTITY).unwrap();
        assert_eq!(
            left.load("/combinations/mouse/bonds/1/bond.pb")
                .unwrap()
                .unwrap(),
            b"saved"
        );
        if result.is_ok() {
            assert_eq!(left.load(path).unwrap().unwrap(), [27; 6000]);
            completed = true;
            break;
        }
        // Nothing is left of the record, not even its directory.
        assert_eq!(left.file_size(path), Ok(None), "fail at {fail_at}");
        assert_eq!(
            left.list("/combinations/mouse/bonds").unwrap(),
            ["1"],
            "fail at {fail_at}"
        );
    }
    assert!(completed);
}

#[test]
fn a_failed_save_is_resolved_by_reading_the_record_back() {
    let flash = Flash::blank();
    Storage::provision_blank(flash.clone(), RANGE, IDENTITY).unwrap();
    let probe = Probe::new(flash);
    let mut store =
        debonair_storage::Storage::<_, 30>::open(probe.clone(), RANGE, IDENTITY).unwrap();
    store.save("/adapter.pb", b"first").unwrap();
    // A write that fails leaves the old record, read back from a fresh mount.
    probe.fail_writes.set(true);
    assert_eq!(store.save("/adapter.pb", b"second"), Err(Error::Io));
    // A record that cannot be read back either has an unknown outcome.
    probe.fail.set(true);
    assert!(matches!(
        store.remove("/adapter.pb"),
        Err(Error::Io | Error::Unknown)
    ));
    probe.fail.set(false);
    probe.fail_writes.set(false);
    assert_eq!(store.load("/adapter.pb").unwrap().unwrap(), b"first");
    store.save("/adapter.pb", b"second").unwrap();
    assert_eq!(store.load("/adapter.pb").unwrap().unwrap(), b"second");
}

#[test]
fn operations_reuse_the_mount_and_remount_after_a_failure() {
    let flash = Flash::blank();
    let mut store = Storage::provision_blank(flash.clone(), RANGE, IDENTITY).unwrap();
    for id in 1..=8 {
        store
            .save(&format!("/combinations/mouse/bonds/{id}/bond.pb"), b"bond")
            .unwrap();
    }
    drop(store);
    let probe = Probe::new(flash);
    let mut store =
        debonair_storage::Storage::<_, 30>::open(probe.clone(), RANGE, IDENTITY).unwrap();
    let load = |store: &mut debonair_storage::Storage<Probe, 30>| {
        probe.reads.set(0);
        let result = store.load("/combinations/mouse/bonds/5/bond.pb");
        (result, probe.reads.get())
    };
    let (result, mounted) = load(&mut store);
    assert_eq!(result.unwrap().unwrap(), b"bond");
    probe.fail.set(true);
    assert!(load(&mut store).0.is_err());
    probe.fail.set(false);
    // A failed read leaves the next operation to mount again, which reads every metadata pair.
    let (result, remounted) = load(&mut store);
    assert_eq!(result.unwrap().unwrap(), b"bond");
    assert!(remounted > 2 * mounted, "{remounted} and {mounted} reads");
    let (_, again) = load(&mut store);
    assert!(again <= mounted, "{again} and {mounted} reads");
}

#[test]
fn a_full_filesystem_keeps_the_old_record() {
    let (_, mut store) = provisioned();
    store.save("/adapter.pb", &[7; 8000]).unwrap();
    let mut full = None;
    for i in 0..100 {
        let name = format!("/fill{i}");
        if let Err(e) = store.save(&name, &[3; 4096]) {
            assert_eq!(e, Error::Full);
            full = Some(name);
            break;
        }
    }
    // A failed creation leaves no file behind.
    assert_eq!(store.file_size(&full.unwrap()).unwrap(), None);
    assert!(store.save("/adapter.pb", &[8; 120 * 1024]).is_err());
    assert_eq!(store.load("/adapter.pb").unwrap().unwrap(), [7; 8000]);
    store.remove("/adapter.pb").unwrap();
    assert_eq!(store.load("/adapter.pb").unwrap(), None);
}

#[test]
fn records_and_bonds_survive_a_remount() {
    use debonair_core::{
        bonds::{self, Bond, Identity, Table},
        hid::Devices,
    };
    let devices = Devices {
        keyboard: true,
        mouse: false,
    };
    let (flash, mut store) = provisioned();
    store::open(&mut store).unwrap();
    let identity = Identity::load_or_create(&mut store, devices, &mut || 42).unwrap();
    let bond = Bond {
        address: [1, 2, 3, 4, 5, 6],
        key_size: 16,
        ..Bond::default()
    };
    let mut table = Table::new(devices, 1);
    table.added(0, 0);
    table
        .save(&mut store, 0, false, |slot| {
            (slot == 0).then(|| bond.clone()).into()
        })
        .unwrap();
    drop(store);
    let mut store = Storage::open(flash, RANGE, IDENTITY).unwrap();
    store::open(&mut store).unwrap();
    assert_eq!(
        Identity::load_or_create(&mut store, devices, &mut || 7).unwrap(),
        identity
    );
    assert_eq!(bonds::load(&mut store, devices).unwrap().bonds, [(1, bond)]);
}
