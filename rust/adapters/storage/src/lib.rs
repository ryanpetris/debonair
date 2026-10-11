//! Records in LittleFS on the Dongle's flash, as a [`Store`].
//!
//! The storage range starts with one erase block, the guard, whose first 32 bytes hold the
//! storage layout's identity; LittleFS fills the rest. Storage of another layout, and a filesystem
//! that cannot be mounted, are never formatted. Only flash that is entirely erased is provisioned.
#![no_std]
extern crate alloc;

use alloc::{boxed::Box, string::String, vec::Vec};
use core::{ops::Range, ptr::NonNull};
use debonair_core::store::{Error, Store, parse_id};
use embassy_futures::block_on;
use embedded_storage_async::nor_flash::NorFlash;
use littlefs2::{
    driver,
    fs::{Allocation, Filesystem},
    io::{Error as LfsError, Write},
    path::{Path, PathBuf},
};

pub const MARKER_BYTES: usize = 32;
pub const BLOCK_BYTES: usize = 4096;
/// LittleFS's smallest read. Directory and metadata reads ask for a few bytes at a time, and on
/// the RP2040 and RP2350 each flash read is a separate DMA transfer with a fixed setup cost, so
/// reading whole 128-byte units takes far fewer reads for about the same number of bytes.
const READ_BYTES: usize = 128;
/// LittleFS's read and program cache, and the unit records are compared in.
const CACHE_BYTES: usize = 512;

/// LittleFS in a flash range. The owner serializes all access; operations complete their flash
/// writes before returning.
///
/// The filesystem stays mounted between operations, since a LittleFS mount reads every metadata
/// pair.
pub struct Storage<F: NorFlash + 'static, const BLOCKS: usize> {
    mount: Mount<F, BLOCKS>,
}

/// LittleFS state, the driver it reads through and the filesystem mounted over them. LittleFS
/// keeps pointers to the allocation and driver while mounted, so they stay at one heap address for
/// the mount's life.
struct Volume<F: NorFlash + 'static, const BLOCKS: usize> {
    alloc: Allocation<Flash<F, BLOCKS>>,
    driver: Flash<F, BLOCKS>,
    fs: Option<Filesystem<'static, Flash<F, BLOCKS>>>,
}

/// A filesystem mounted on demand over a `Volume` this value owns.
///
/// Safety: `volume` comes from a leaked `Box` and is freed only by `Drop` or `into_driver`, after
/// `fs` is cleared. Every access goes through the raw pointer. `fs` holds the only references to
/// `alloc` and `driver`, created as disjoint field borrows when it is mounted, and it never leaves
/// the heap, so moving or dropping a `Mount` moves only the pointer. The `'static` lifetime is
/// never exposed: `with` lends the filesystem for one call, and no closure returns a borrow of it.
struct Mount<F: NorFlash + 'static, const BLOCKS: usize> {
    volume: NonNull<Volume<F, BLOCKS>>,
}
impl<F: NorFlash + 'static, const BLOCKS: usize> Mount<F, BLOCKS> {
    fn new(driver: Flash<F, BLOCKS>) -> Self {
        let volume = Box::new(Volume {
            alloc: Allocation::new(),
            driver,
            fs: None,
        });
        Self {
            volume: NonNull::from(Box::leak(volume)),
        }
    }
    /// Runs `f` on the mounted filesystem, mounting it first if needed. An error that may leave
    /// the in-memory state behind the flash unmounts it, so the next call reads the flash again.
    fn with<R>(
        &mut self,
        f: impl FnOnce(&Filesystem<'static, Flash<F, BLOCKS>>) -> littlefs2::io::Result<R>,
    ) -> littlefs2::io::Result<R> {
        let volume = self.volume.as_ptr();
        // SAFETY: see `Mount`. `fs` is a field disjoint from `alloc` and `driver`.
        let fs = match unsafe { &mut (*volume).fs } {
            Some(fs) => fs,
            empty => {
                // SAFETY: see `Mount`. No reference to `alloc` or `driver` exists while `fs` is
                // empty.
                let (alloc, driver) = unsafe { (&mut (*volume).alloc, &mut (*volume).driver) };
                // A mount reinitializes all LittleFS state in the allocation.
                empty.insert(Filesystem::mount(alloc, driver)?)
            }
        };
        let result = f(fs);
        if let Err(e) = result
            && !matches!(
                e,
                LfsError::NO_SUCH_ENTRY
                    | LfsError::PATH_NOT_DIR
                    | LfsError::PATH_IS_DIR
                    | LfsError::FILENAME_TOO_LONG
            )
        {
            self.unmount();
        }
        result
    }
    fn unmount(&mut self) {
        // LittleFS buffers are caller-provided, so there is nothing to release.
        // SAFETY: see `Mount`. No borrow of `fs` outlives a `with` call.
        unsafe { (*self.volume.as_ptr()).fs = None };
    }
    fn into_driver(mut self) -> Flash<F, BLOCKS> {
        self.unmount();
        // SAFETY: see `Mount`. The filesystem's borrows ended above, and forgetting `self` keeps
        // `Drop` from freeing the volume again.
        let volume = unsafe { Box::from_raw(self.volume.as_ptr()) };
        core::mem::forget(self);
        volume.driver
    }
}
impl<F: NorFlash + 'static, const BLOCKS: usize> Drop for Mount<F, BLOCKS> {
    fn drop(&mut self) {
        self.unmount();
        // SAFETY: see `Mount`. The filesystem's borrows ended above.
        drop(unsafe { Box::from_raw(self.volume.as_ptr()) });
    }
}

struct Flash<F, const BLOCKS: usize> {
    flash: F,
    start: u32,
}
impl<F: NorFlash, const BLOCKS: usize> driver::Storage for Flash<F, BLOCKS> {
    const READ_SIZE: usize = READ_BYTES;
    const WRITE_SIZE: usize = F::WRITE_SIZE;
    const BLOCK_SIZE: usize = BLOCK_BYTES;
    const BLOCK_COUNT: usize = BLOCKS;
    const BLOCK_CYCLES: isize = 500;
    type CACHE_SIZE = littlefs2::consts::U512;
    type LOOKAHEAD_SIZE = littlefs2::consts::U16;
    fn read(&mut self, off: usize, buf: &mut [u8]) -> littlefs2::io::Result<usize> {
        self.bounds(off, buf.len())?;
        block_on(self.flash.read(self.start + off as u32, buf)).map_err(|_| LfsError::IO)?;
        Ok(buf.len())
    }
    fn write(&mut self, off: usize, buf: &[u8]) -> littlefs2::io::Result<usize> {
        self.bounds(off, buf.len())?;
        block_on(self.flash.write(self.start + off as u32, buf)).map_err(|_| LfsError::IO)?;
        Ok(buf.len())
    }
    fn erase(&mut self, off: usize, len: usize) -> littlefs2::io::Result<usize> {
        self.bounds(off, len)?;
        block_on(
            self.flash
                .erase(self.start + off as u32, self.start + (off + len) as u32),
        )
        .map_err(|_| LfsError::IO)?;
        Ok(len)
    }
}
impl<F, const BLOCKS: usize> Flash<F, BLOCKS> {
    fn bounds(&self, off: usize, len: usize) -> littlefs2::io::Result<()> {
        if off
            .checked_add(len)
            .is_none_or(|end| end > BLOCKS * BLOCK_BYTES)
        {
            Err(LfsError::INVALID)
        } else {
            Ok(())
        }
    }
}

fn error(e: LfsError) -> Error {
    match e {
        LfsError::NO_SPACE => Error::Full,
        LfsError::NO_MEMORY => Error::Unavailable,
        LfsError::INVALID | LfsError::FILENAME_TOO_LONG => Error::Bounds,
        LfsError::FILE_TOO_BIG => Error::TooLarge,
        LfsError::NO_SUCH_ENTRY | LfsError::PATH_NOT_DIR => Error::Missing,
        // A failed metadata checksum is a read error: the data may read back on a later try.
        _ => Error::Io,
    }
}

fn checked_path(value: &str) -> Result<PathBuf, Error> {
    if !value.starts_with('/') || value.split('/').any(|part| part == ".." || part == ".") {
        return Err(Error::Bounds);
    }
    PathBuf::try_from(value).map_err(|_| Error::Bounds)
}

/// The directory of record `path` when it is a record directory, named by an ID. Such a directory
/// holds nothing once its record is gone, so it is removed with the record.
fn record_directory(path: &Path) -> Option<PathBuf> {
    let parent = path.parent()?;
    parse_id(parent.file_name()?.as_str())?;
    Some(parent)
}

/// Removes record directory `directory` when it is empty.
fn remove_empty<F: driver::Storage>(
    fs: &Filesystem<'_, F>,
    directory: &Path,
) -> littlefs2::io::Result<()> {
    match fs.remove_dir(directory) {
        Ok(()) | Err(LfsError::DIR_NOT_EMPTY | LfsError::NO_SUCH_ENTRY) => Ok(()),
        Err(e) => Err(e),
    }
}

/// Whether file `path` holds exactly `bytes`. A failure to look the file up is the outer error; a
/// failure to read the contents of a file of the same length is the inner one.
fn compare<F: driver::Storage>(
    fs: &Filesystem<'_, F>,
    path: &Path,
    bytes: &[u8],
) -> littlefs2::io::Result<littlefs2::io::Result<bool>> {
    match fs.metadata(path) {
        Err(LfsError::NO_SUCH_ENTRY) => return Ok(Ok(false)),
        Err(e) => return Err(e),
        Ok(m) if m.len() != bytes.len() || !m.is_file() => return Ok(Ok(false)),
        _ => (),
    }
    Ok(fs.open_file_and_then(path, |file| {
        let mut buffer = [0; CACHE_BYTES];
        let mut at = 0;
        while at < bytes.len() {
            let len = buffer.len().min(bytes.len() - at);
            let n = file.read(&mut buffer[..len])?;
            if n == 0 || buffer[..n] != bytes[at..at + n] {
                return Ok(false);
            }
            at += n;
        }
        Ok(true)
    }))
}

impl<F: NorFlash + 'static, const BLOCKS: usize> Storage<F, BLOCKS> {
    /// Checks that `range` is the guard block followed by exactly `BLOCKS` LittleFS blocks.
    fn config(flash: &F, range: &Range<u32>) -> Result<(), Error> {
        if BLOCKS < 2
            || F::ERASE_SIZE != BLOCK_BYTES
            || range.end as usize > flash.capacity()
            || range.end.checked_sub(range.start).map(|n| n as usize)
                != Some((BLOCKS + 1) * BLOCK_BYTES)
            || !(range.start as usize).is_multiple_of(BLOCK_BYTES)
            || F::READ_SIZE == 0
            || F::WRITE_SIZE == 0
            || !READ_BYTES.is_multiple_of(F::READ_SIZE)
            || !CACHE_BYTES.is_multiple_of(F::WRITE_SIZE)
        {
            return Err(Error::Bounds);
        }
        Ok(())
    }

    fn marker(flash: &mut F, range: &Range<u32>) -> Result<[u8; MARKER_BYTES], Error> {
        let mut marker = [0; MARKER_BYTES];
        block_on(flash.read(range.start, &mut marker)).map_err(|_| Error::Io)?;
        Ok(marker)
    }

    /// Opens the storage in `range`, provisioning it when the guard is blank.
    pub fn open_or_provision_blank(
        mut flash: F,
        range: Range<u32>,
        identity: [u8; MARKER_BYTES],
    ) -> Result<Self, Error> {
        Self::config(&flash, &range)?;
        if Self::marker(&mut flash, &range)? == [0xff; MARKER_BYTES] {
            Self::provision_blank(flash, range, identity)
        } else {
            Self::open(flash, range, identity)
        }
    }

    /// Opens provisioned storage whose guard holds `identity`. `Unprovisioned` for a blank guard
    /// and `Layout` for another identity; a filesystem that cannot be mounted fails without being
    /// formatted.
    pub fn open(
        mut flash: F,
        range: Range<u32>,
        identity: [u8; MARKER_BYTES],
    ) -> Result<Self, Error> {
        Self::config(&flash, &range)?;
        let marker = Self::marker(&mut flash, &range)?;
        if marker == [0xff; MARKER_BYTES] {
            return Err(Error::Unprovisioned);
        }
        if marker != identity {
            return Err(Error::Layout);
        }
        let mut store = Self {
            mount: Mount::new(Flash {
                flash,
                start: range.start + BLOCK_BYTES as u32,
            }),
        };
        // Never format on mount failure. An interrupted provisioning needs explicit recovery,
        // just like unknown or damaged storage.
        store.mount.with(|_| Ok(())).map_err(error)?;
        Ok(store)
    }

    /// Provisions `range`, which must be entirely erased: writes and verifies the guard, then
    /// formats LittleFS. `Layout` when any byte is not erased.
    pub fn provision_blank(
        mut flash: F,
        range: Range<u32>,
        identity: [u8; MARKER_BYTES],
    ) -> Result<Self, Error> {
        Self::config(&flash, &range)?;
        if identity == [0xff; MARKER_BYTES] {
            return Err(Error::Layout);
        }
        let mut page = [0; CACHE_BYTES];
        for at in (range.start..range.end).step_by(page.len()) {
            block_on(flash.read(at, &mut page)).map_err(|_| Error::Io)?;
            if page.iter().any(|&v| v != 0xff) {
                return Err(Error::Layout);
            }
        }
        page.fill(0xff);
        page[..MARKER_BYTES].copy_from_slice(&identity);
        // Claim the layout before formatting. Any interruption leaves either a mountable
        // filesystem or a failure, never permission to format again.
        block_on(flash.write(range.start, &page)).map_err(|_| Error::Io)?;
        if Self::marker(&mut flash, &range)? != identity {
            return Err(Error::Io);
        }
        let mut driver = Flash {
            flash,
            start: range.start + BLOCK_BYTES as u32,
        };
        Filesystem::format(&mut driver).map_err(error)?;
        let mut store = Self {
            mount: Mount::new(driver),
        };
        store.mount.with(|_| Ok(())).map_err(error)?;
        Ok(store)
    }

    pub fn into_flash(self) -> F {
        self.mount.into_driver().flash
    }

    /// The length of file `path`, or None when it does not exist.
    pub fn file_size(&mut self, path: &str) -> Result<Option<usize>, Error> {
        let path = checked_path(path)?;
        self.mount
            .with(|fs| match fs.metadata(&path) {
                Ok(m) if m.is_file() => Ok(Some(m.len())),
                Ok(_) => Err(LfsError::PATH_IS_DIR),
                Err(LfsError::NO_SUCH_ENTRY) => Ok(None),
                Err(e) => Err(e),
            })
            .map_err(error)
    }

    /// Whether directory `path` exists.
    pub fn directory_exists(&mut self, path: &str) -> Result<bool, Error> {
        let path = checked_path(path)?;
        self.mount
            .with(|fs| match fs.metadata(&path) {
                Ok(m) => Ok(m.is_dir()),
                Err(LfsError::NO_SUCH_ENTRY) => Ok(false),
                Err(e) => Err(e),
            })
            .map_err(error)
    }

    /// Writes `bytes` as the whole file `name`, unless it already holds them. Old contents that
    /// cannot be read are written over.
    ///
    /// The file is opened truncated, written and closed. LittleFS keeps an existing file's old
    /// contents until the close commits the new ones in one metadata commit, so an interrupted
    /// write leaves the old or the new contents. Creating a missing file commits it empty when it
    /// is opened, so an interrupted creation can leave an empty file, which holds no record.
    pub fn replace_file(&mut self, name: &str, bytes: &[u8]) -> Result<(), Error> {
        let path = checked_path(name)?;
        match self.mount.with(|fs| compare(fs, &path, bytes)) {
            Ok(Ok(true)) => return Ok(()),
            Ok(Ok(false)) => {}
            // Old contents that cannot be read are written over, from a fresh mount.
            Ok(Err(_)) => self.mount.unmount(),
            Err(e) => return Err(error(e)),
        }
        let mut started = false;
        let mut unwritten = false;
        let result = self
            .mount
            .with(|fs| {
                started = true;
                if let Some(parent) = path.parent() {
                    fs.create_dir_all(&parent)?;
                }
                // A failed write marks the file errored, so closing it does not commit.
                fs.create_file_and_then(&path, |file| {
                    file.write_all(bytes).inspect_err(|_| unwritten = true)
                })
            })
            // LittleFS reports a creation whose commit finds no room as a name too long. A checked
            // path is within LittleFS's 255-byte name limit, so the filesystem is full.
            .map_err(|e| match e {
                LfsError::FILENAME_TOO_LONG => LfsError::NO_SPACE,
                e => e,
            });
        match result {
            Ok(()) => Ok(()),
            // A filesystem that cannot be mounted was not changed.
            Err(e) if !started => Err(error(e)),
            Err(e) => {
                // Resolve against a fresh mount, never the mutation's caches.
                self.mount.unmount();
                match self.mount.with(|fs| compare(fs, &path, bytes)?) {
                    Ok(true) => Ok(()),
                    Ok(false) => {
                        // The old contents are authoritative. An empty file left by a failed
                        // creation holds no record and is reclaimed now when possible, with its
                        // record directory when nothing else is in it.
                        let _ = self.mount.with(|fs| {
                            match fs.metadata(&path) {
                                Ok(meta) if meta.is_file() && meta.is_empty() => {
                                    fs.remove(&path)?
                                }
                                Err(LfsError::NO_SUCH_ENTRY) => (),
                                Ok(_) => return Ok(()),
                                Err(e) => return Err(e),
                            }
                            match record_directory(&path) {
                                Some(directory) => remove_empty(fs, &directory),
                                None => Ok(()),
                            }
                        });
                        Err(error(e))
                    }
                    // A write that failed before closing committed nothing of the new contents,
                    // whether or not the old ones can be read.
                    Err(_) if unwritten => Err(error(e)),
                    Err(_) => Err(Error::Unknown),
                }
            }
        }
    }

    /// Removes file `name`, and its record directory when that is left empty.
    pub fn remove_file(&mut self, name: &str) -> Result<(), Error> {
        let path = checked_path(name)?;
        let result =
            self.mount.with(|fs| {
                match fs.remove(&path) {
                    Ok(()) | Err(LfsError::NO_SUCH_ENTRY) => (),
                    Err(e) => return Err(e),
                }
                // Empty record directories take a metadata pair. Their removal is cleanup, after the
                // authoritative file deletion.
                Ok(record_directory(&path)
                    .is_none_or(|directory| remove_empty(fs, &directory).is_ok()))
            });
        match result {
            Ok(true) => Ok(()),
            Ok(false) => {
                // A failed cleanup commit leaves the next operation a fresh mount.
                self.mount.unmount();
                Ok(())
            }
            Err(LfsError::NO_SUCH_ENTRY | LfsError::PATH_NOT_DIR) => Ok(()),
            Err(e) => {
                // Resolve against a fresh mount, never the mutation's caches.
                self.mount.unmount();
                match self.file_size(name) {
                    Ok(None) => Ok(()),
                    Ok(Some(_)) => Err(error(e)),
                    Err(_) => Err(Error::Unknown),
                }
            }
        }
    }
}

impl<F: NorFlash + 'static, const BLOCKS: usize> Store for Storage<F, BLOCKS> {
    fn load(&mut self, name: &str) -> Result<Option<Vec<u8>>, Error> {
        let path = checked_path(name)?;
        let read = self.mount.with(|fs| {
            fs.open_file_and_then(&path, |file| {
                let n = file.len()?;
                if n == 0 {
                    return Ok(Ok(None));
                }
                let mut bytes = Vec::new();
                if bytes.try_reserve_exact(n).is_err() {
                    return Ok(Err(Error::Unavailable));
                }
                bytes.resize(n, 0);
                Ok(if file.read(&mut bytes)? == n {
                    Ok(Some(bytes))
                } else {
                    Err(Error::Io)
                })
            })
        });
        match read {
            Ok(result) => result,
            Err(LfsError::NO_SUCH_ENTRY) => Ok(None),
            // A directory in the record's place.
            Err(LfsError::PATH_IS_DIR) => Err(Error::Io),
            Err(e) => Err(error(e)),
        }
    }
    fn save(&mut self, name: &str, bytes: &[u8]) -> Result<(), Error> {
        // An empty file holds no record.
        if bytes.is_empty() {
            return Err(Error::Bounds);
        }
        self.replace_file(name, bytes)
    }
    fn remove(&mut self, name: &str) -> Result<(), Error> {
        self.remove_file(name)
    }
    fn list(&mut self, directory: &str) -> Result<Vec<String>, Error> {
        let path = checked_path(directory)?;
        let listed = self.mount.with(|fs| {
            fs.read_dir_and_then(&path, |entries| {
                let mut names = Vec::new();
                for entry in entries {
                    let entry = entry?;
                    let name = entry.file_name().as_str();
                    if matches!(name, "." | "..") {
                        continue;
                    }
                    if names.try_reserve(1).is_err() {
                        return Ok(Err(Error::Unavailable));
                    }
                    names.push(String::from(name));
                }
                Ok(Ok(names))
            })
        });
        match listed {
            Ok(result) => result,
            Err(LfsError::NO_SUCH_ENTRY) => Ok(Vec::new()),
            Err(e) => Err(error(e)),
        }
    }
}
