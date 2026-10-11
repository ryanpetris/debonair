//! Records saved on the Dongle's flash. Each record is one protobuf message from
//! `proto/storage.proto` in a file at a fixed path; `docs/storage-format.md` describes the layout.
use crate::protocol::storage as saved;
use alloc::{string::String, vec::Vec};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    /// The storage is in use, or memory for a record could not be reserved.
    Unavailable,
    /// The flash holds no storage layout yet.
    Unprovisioned,
    /// The flash holds another storage layout, or a root record of another format.
    Layout,
    /// A path or configuration outside the storage's bounds.
    Bounds,
    /// The record is too large.
    TooLarge,
    /// Not enough free space.
    Full,
    Io,
    /// A record does not decode as its message or breaks its rules.
    Corrupt,
    /// A failed change whose outcome could not be confirmed by reading the record back.
    Unknown,
    /// A directory in the path does not exist.
    Missing,
}

/// Records at fixed paths. A record is a nonempty file: an empty file holds no record.
pub trait Store {
    /// Record `path` whole, in a buffer of exactly its length. None when the file is missing or
    /// empty.
    fn load(&mut self, path: &str) -> Result<Option<Vec<u8>>, Error>;
    /// Saves `bytes`, which are not empty, as the whole record `path`, creating its directories.
    /// A save that fails leaves the old or the new record. Saving the bytes the record already
    /// holds writes nothing.
    fn save(&mut self, path: &str, bytes: &[u8]) -> Result<(), Error>;
    /// Removes record `path`, and its directory when that is a record directory, named by an ID,
    /// and left empty. Removing a missing record succeeds.
    fn remove(&mut self, path: &str) -> Result<(), Error>;
    /// The names in `directory`, in no particular order. Empty when the directory is missing.
    fn list(&mut self, directory: &str) -> Result<Vec<String>, Error>;
    /// The error the storage failed to open with, which every access reports. Ok for storage that
    /// opened, whatever its later accesses report.
    fn opening(&mut self) -> Result<(), Error> {
        Ok(())
    }
}

/// One store used by several owners on the same task. An access while another is in progress
/// fails as Unavailable.
pub struct Shared<S>(core::cell::RefCell<S>);
impl<S> Shared<S> {
    pub const fn new(store: S) -> Self {
        Self(core::cell::RefCell::new(store))
    }
    /// The store itself, while no access is in progress.
    pub fn borrow(&self) -> core::cell::Ref<'_, S> {
        self.0.borrow()
    }
}
impl<S: Store> Store for &Shared<S> {
    fn load(&mut self, path: &str) -> Result<Option<Vec<u8>>, Error> {
        let mut store = self.0.try_borrow_mut().map_err(|_| Error::Unavailable)?;
        store.load(path)
    }
    fn save(&mut self, path: &str, bytes: &[u8]) -> Result<(), Error> {
        let mut store = self.0.try_borrow_mut().map_err(|_| Error::Unavailable)?;
        store.save(path, bytes)
    }
    fn remove(&mut self, path: &str) -> Result<(), Error> {
        let mut store = self.0.try_borrow_mut().map_err(|_| Error::Unavailable)?;
        store.remove(path)
    }
    fn list(&mut self, directory: &str) -> Result<Vec<String>, Error> {
        let mut store = self.0.try_borrow_mut().map_err(|_| Error::Unavailable)?;
        store.list(directory)
    }
    /// Storage in use by another owner has opened.
    fn opening(&mut self) -> Result<(), Error> {
        self.0.try_borrow_mut().map_or(Ok(()), |mut s| s.opening())
    }
}

/// Storage that failed to open reports that error for every access until the board restarts.
impl<S: Store> Store for Result<S, Error> {
    fn load(&mut self, path: &str) -> Result<Option<Vec<u8>>, Error> {
        self.as_mut().map_err(|e| *e)?.load(path)
    }
    fn save(&mut self, path: &str, bytes: &[u8]) -> Result<(), Error> {
        self.as_mut().map_err(|e| *e)?.save(path, bytes)
    }
    fn remove(&mut self, path: &str) -> Result<(), Error> {
        self.as_mut().map_err(|e| *e)?.remove(path)
    }
    fn list(&mut self, directory: &str) -> Result<Vec<String>, Error> {
        self.as_mut().map_err(|e| *e)?.list(directory)
    }
    fn opening(&mut self) -> Result<(), Error> {
        self.as_mut().map_err(|e| *e)?.opening()
    }
}

/// The ID a record directory is named after: a positive decimal number of at most 32 bits,
/// without a prefix or padding.
pub fn parse_id(value: &str) -> Option<u64> {
    if value.is_empty() || value.starts_with('0') || !value.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    value.parse().ok().filter(|id| *id <= u64::from(u32::MAX))
}

/// The bytes of a saved record: `message` encoded into a buffer of exactly its length.
pub fn encode<M: prost::Message>(message: &M) -> Result<Vec<u8>, Error> {
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(message.encoded_len())
        .map_err(|_| Error::Unavailable)?;
    message.encode(&mut bytes).map_err(|_| Error::TooLarge)?;
    Ok(bytes)
}

/// The saved record in `bytes`. `Corrupt` when they do not decode as `M`.
pub fn decode<M: prost::Message + Default>(bytes: &[u8]) -> Result<M, Error> {
    M::decode(bytes).map_err(|_| Error::Corrupt)
}

/// A fixed-length byte field of a saved record. `Corrupt` when it has another length.
pub fn array<const N: usize>(bytes: &[u8]) -> Result<[u8; N], Error> {
    bytes.try_into().map_err(|_| Error::Corrupt)
}

const FORMAT: &str = "/format.pb";
/// The value of `Format.format`.
const FORMAT_VALUE: u32 = 1;

/// Checks the filesystem's root record, writing it into a filesystem that holds nothing else.
/// `Layout` when the root record is not this firmware's or is missing beside other files, so
/// storage of another format is never changed.
pub fn open<S: Store>(store: &mut S) -> Result<(), Error> {
    match store.load(FORMAT)? {
        Some(bytes) => {
            let format: saved::Format = decode(&bytes).map_err(|_| Error::Layout)?;
            if format.format == FORMAT_VALUE {
                Ok(())
            } else {
                Err(Error::Layout)
            }
        }
        // An empty root file is a creation that did not complete.
        None if store.list("/")?.iter().all(|name| name == "format.pb") => store.save(
            FORMAT,
            &encode(&saved::Format {
                format: FORMAT_VALUE,
            })?,
        ),
        None => Err(Error::Layout),
    }
}

/// Opens the records of a filesystem that was opened as `store`. Storage that failed to open stays
/// unavailable: `Unprovisioned` or `Layout` when the flash holds no layout or another one, and
/// `Unavailable` for any other failure, such as a filesystem that cannot be mounted.
pub fn opened<S: Store>(store: Result<S, Error>) -> Result<S, Error> {
    let unavailable = |e| match e {
        Error::Unprovisioned | Error::Layout => e,
        _ => Error::Unavailable,
    };
    let mut store = store.map_err(unavailable)?;
    open(&mut store).map_err(unavailable)?;
    Ok(store)
}

#[cfg(test)]
pub mod memory {
    use super::*;
    use alloc::collections::BTreeMap;

    /// A store kept in RAM, for tests. Directories are implied by the files' paths.
    #[derive(Default)]
    pub struct Memory {
        pub files: BTreeMap<String, Vec<u8>>,
        pub fail: bool,
        /// Makes every save fail as `Unavailable`, as when memory runs out.
        pub unavailable: bool,
        /// Successful saves that changed a file.
        pub writes: usize,
    }
    impl Store for Memory {
        fn load(&mut self, path: &str) -> Result<Option<Vec<u8>>, Error> {
            if self.fail {
                return Err(Error::Io);
            }
            Ok(self.files.get(path).filter(|f| !f.is_empty()).cloned())
        }
        fn save(&mut self, path: &str, bytes: &[u8]) -> Result<(), Error> {
            if self.fail {
                return Err(Error::Io);
            }
            if self.unavailable {
                return Err(Error::Unavailable);
            }
            if self.files.get(path).map(Vec::as_slice) != Some(bytes) {
                self.files.insert(path.into(), bytes.into());
                self.writes += 1;
            }
            Ok(())
        }
        fn remove(&mut self, path: &str) -> Result<(), Error> {
            if self.fail {
                return Err(Error::Io);
            }
            self.files.remove(path);
            Ok(())
        }
        fn list(&mut self, directory: &str) -> Result<Vec<String>, Error> {
            if self.fail {
                return Err(Error::Io);
            }
            let prefix = if directory.ends_with('/') {
                directory.into()
            } else {
                alloc::format!("{directory}/")
            };
            let mut names: Vec<String> = self
                .files
                .keys()
                .filter_map(|path| path.strip_prefix(prefix.as_str()))
                .map(|rest| rest.split('/').next().unwrap().into())
                .collect();
            names.dedup();
            Ok(names)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{memory::Memory, *};

    #[test]
    fn parses_record_ids() {
        assert_eq!(parse_id("1"), Some(1));
        assert_eq!(parse_id("4294967295"), Some(u64::from(u32::MAX)));
        for name in ["", "0", "01", "+1", "1a", "4294967296", "bond.pb"] {
            assert_eq!(parse_id(name), None, "{name}");
        }
    }

    #[test]
    fn opening_writes_the_root_record_only_into_an_empty_filesystem() {
        let mut store = Memory::default();
        open(&mut store).unwrap();
        let format: saved::Format = decode(&store.files[FORMAT]).unwrap();
        assert_eq!(format.format, 1);
        open(&mut store).unwrap();
        assert_eq!(store.writes, 1);

        // An interrupted creation left an empty root file.
        let mut store = Memory::default();
        store.files.insert(FORMAT.into(), Vec::new());
        open(&mut store).unwrap();
        assert!(!store.files[FORMAT].is_empty());

        let mut store = Memory::default();
        store
            .files
            .insert("/adapter.pb".into(), alloc::vec![0x10, 1]);
        assert_eq!(open(&mut store), Err(Error::Layout));
        assert!(!store.files.contains_key(FORMAT));

        let mut store = Memory::default();
        store
            .files
            .insert(FORMAT.into(), encode(&saved::Format { format: 2 }).unwrap());
        assert_eq!(open(&mut store), Err(Error::Layout));
        store.files.insert(FORMAT.into(), alloc::vec![0xff]);
        assert_eq!(open(&mut store), Err(Error::Layout));
        assert_eq!(opened(Ok(store)).err(), Some(Error::Layout));
        assert_eq!(
            opened::<Memory>(Err(Error::Unprovisioned)).err(),
            Some(Error::Unprovisioned)
        );
        // A filesystem that cannot be mounted, or whose root record cannot be read, is unavailable.
        assert_eq!(
            opened::<Memory>(Err(Error::Io)).err(),
            Some(Error::Unavailable)
        );
        let failing = Memory {
            fail: true,
            ..Memory::default()
        };
        assert_eq!(opened(Ok(failing)).err(), Some(Error::Unavailable));
    }
}
