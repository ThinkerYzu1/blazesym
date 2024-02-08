use std::fs::File;
use std::os::unix::io::AsRawFd;
use std::path::Path;
use std::path::PathBuf;

use crate::insert_map::InsertMap;
use crate::once::OnceCell;
use crate::util::fstat;
use crate::ErrorExt as _;
use crate::Result;


#[derive(Debug, Eq, Hash, PartialEq)]
// `libc` has deprecated `time_t` usage on `musl`. See
// https://github.com/rust-lang/libc/issues/1848
#[cfg_attr(target_env = "musl", allow(deprecated))]
struct FileMeta {
    dev: libc::dev_t,
    inode: libc::ino_t,
    size: libc::off_t,
    mtime_sec: libc::time_t,
    mtime_nsec: i64,
}

impl From<&libc::stat> for FileMeta {
    fn from(other: &libc::stat) -> Self {
        // Casts are necessary because on Android some libc types do not
        // use proper typedefs. https://github.com/rust-lang/libc/issues/3285
        Self {
            dev: other.st_dev as _,
            inode: other.st_ino as _,
            size: other.st_size as _,
            mtime_sec: other.st_mtime,
            mtime_nsec: other.st_mtime_nsec as _,
        }
    }
}


#[derive(Debug, Eq, Hash, PartialEq)]
struct EntryMeta {
    path: PathBuf,
    meta: Option<FileMeta>,
}

impl EntryMeta {
    /// Create a new [`EntryMeta`] object. If `stat` is [`None`] file
    /// modification times and other meta data are effectively ignored.
    fn new(path: PathBuf, stat: Option<&libc::stat>) -> Self {
        Self {
            path,
            meta: stat.map(FileMeta::from),
        }
    }
}


#[derive(Debug)]
struct Entry<T> {
    file: File,
    value: OnceCell<T>,
}

impl<T> Entry<T> {
    fn new(file: File) -> Self {
        Self {
            file,
            value: OnceCell::new(),
        }
    }
}


/// A lookup cache for data associated with a file, looked up by path.
///
/// The cache transparently checks whether the file contents have
/// changed based on file system meta data and creates and hands out a
/// new entry if so.
/// Note that stale/old entries are never evicted.
#[derive(Debug)]
pub(crate) struct FileCache<T> {
    /// The map we use for associating file meta data with user-defined
    /// data.
    cache: InsertMap<EntryMeta, Entry<T>>,
}

impl<T> FileCache<T> {
    /// Create a new [`FileCache`] object.
    pub fn new() -> Self {
        Self {
            cache: InsertMap::new(),
        }
    }

    /// Retrieve an entry for the file at the given `path`.
    pub fn entry(&self, path: &Path) -> Result<(&File, &OnceCell<T>)> {
        let file =
            File::open(path).with_context(|| format!("failed to open file {}", path.display()))?;
        let stat = fstat(file.as_raw_fd())?;
        let meta = EntryMeta::new(path.to_path_buf(), Some(&stat));

        let entry = self.cache.get_or_insert(meta, || Entry::new(file));
        Ok((&entry.file, &entry.value))
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    use std::io::Read as _;
    use std::io::Write as _;
    use std::thread::sleep;
    use std::time::Duration;

    use tempfile::tempfile;
    use tempfile::NamedTempFile;


    /// Exercise the `Debug` representation of various types.
    #[test]
    fn debug_repr() {
        let cache = FileCache::<()>::new();
        assert_ne!(format!("{cache:?}"), "");

        let tmpfile = tempfile().unwrap();
        let entry = Entry::<usize>::new(tmpfile);
        assert_ne!(format!("{entry:?}"), "");
    }

    /// Check that we can associate data with a file.
    #[test]
    fn lookup() {
        let cache = FileCache::<usize>::new();
        let tmpfile = NamedTempFile::new().unwrap();

        {
            let (_file, cell) = cache.entry(tmpfile.path()).unwrap();
            assert_eq!(cell.get(), None);

            let () = cell.set(42).unwrap();
        }

        {
            let (_file, cell) = cache.entry(tmpfile.path()).unwrap();
            assert_eq!(cell.get(), Some(&42));
        }
    }

    /// Make sure that a changed file purges the cache entry.
    #[test]
    fn outdated() {
        let cache = FileCache::<usize>::new();
        let tmpfile = NamedTempFile::new().unwrap();
        let modified = {
            let (file, cell) = cache.entry(tmpfile.path()).unwrap();
            assert_eq!(cell.get(), None);

            let () = cell.set(42).unwrap();
            file.metadata().unwrap().modified().unwrap()
        };

        // Sleep briefly to make sure that file times will end up being
        // different.
        let () = sleep(Duration::from_millis(10));

        let mut file = File::create(tmpfile.path()).unwrap();
        let () = file.write_all(b"foobar").unwrap();

        {
            let (mut file, entry) = cache.entry(tmpfile.path()).unwrap();
            assert_eq!(entry.get(), None);
            assert_ne!(file.metadata().unwrap().modified().unwrap(), modified);

            let mut content = Vec::new();
            let _count = file.read_to_end(&mut content);
            assert_eq!(content, b"foobar");
        }
    }
}
