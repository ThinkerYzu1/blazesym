use std::collections::HashMap;
use std::fs::File;
use std::io::Error;
use std::io::ErrorKind;
use std::io::Result;
use std::path::Path;
use std::path::PathBuf;

use crate::elf;
use crate::elf::types::Elf64_Nhdr;
use crate::elf::ElfParser;
use crate::log::warn;
use crate::maps;
use crate::maps::MapsEntry;
use crate::maps::Pid;
use crate::util::ReadRaw as _;
use crate::Addr;

use super::meta::Binary;
use super::meta::Unknown;
use super::meta::UserAddrMeta;


/// A type capturing normalized addresses along with captured meta data.
///
/// This type enables "remote" symbolization. That is to say, it represents the
/// input necessary for addresses to be symbolized on a system other than where
/// they were recorded.
#[derive(Clone, Debug)]
pub struct NormalizedAddrs<M> {
    /// Normalized addresses along with an index into `meta` for retrieval of
    /// the corresponding [`AddrMeta`] information.
    ///
    /// A normalized address is one as it would appear in a binary or debug
    /// symbol file, i.e., one excluding any relocations.
    pub addrs: Vec<(Addr, usize)>,
    /// Meta information about the normalized addresses.
    pub meta: Vec<M>,
}

/// A type representing normalized user addresses.
pub type NormalizedUserAddrs = NormalizedAddrs<UserAddrMeta>;


/// A type representing a build ID note.
///
/// In the ELF file, this header is typically followed by the variable sized
/// build ID.
#[repr(C)]
struct BuildIdNote {
    /// ELF note header.
    header: Elf64_Nhdr,
    /// NUL terminated string representing the name.
    name: [u8; 4],
}

// SAFETY: `BuildIdNote` is valid for any bit pattern.
unsafe impl crate::util::Pod for BuildIdNote {}


/// Attempt to read an ELF binary's build ID.
// TODO: Currently look up is always performed based on section name, but there
//       is also the possibility of iterating notes and checking checking
//       Elf64_Nhdr.n_type for NT_GNU_BUILD_ID, specifically.
fn read_build_id(path: &Path) -> Result<Option<Vec<u8>>> {
    let build_id_section = ".note.gnu.build-id";
    let file = File::open(path)?;
    let parser = ElfParser::open_file(file)?;

    // The build ID is contained in the `.note.gnu.build-id` section. See
    // elf(5).
    if let Ok(idx) = parser.find_section(build_id_section) {
        // SANITY: We just found the index so the section should always be
        //         found.
        let shdr = parser.section_headers()?.get(idx).unwrap();
        if shdr.sh_type != elf::types::SHT_NOTE {
            warn!(
                "build ID section {build_id_section} of {} is of unsupported type ({})",
                path.display(),
                shdr.sh_type
            );
            return Ok(None)
        }

        // SANITY: We just found the index so the section should always be
        //         found.
        let mut bytes = parser.section_data(idx).unwrap();
        let header = bytes.read_pod_ref::<BuildIdNote>().ok_or_else(|| {
            Error::new(
                ErrorKind::InvalidData,
                "failed to read build ID section header",
            )
        })?;
        if &header.name != b"GNU\0" {
            warn!(
                "encountered unsupported build ID type {:?}; ignoring",
                header.name
            );
            Ok(None)
        } else {
            // Every byte following the header is part of the build ID.
            let build_id = bytes.to_vec();
            Ok(Some(build_id))
        }
    } else {
        Ok(None)
    }
}


/// Normalize a virtual address belonging to an ELF file represented by the
/// provided [`MapsEntry`].
fn normalize_elf_addr(virt_addr: Addr, entry: &MapsEntry) -> Result<Addr> {
    let file_off = virt_addr - entry.range.start + entry.offset as usize;
    let parser = ElfParser::open(&entry.path)?;
    let phdrs = parser.program_headers()?;
    let addr = phdrs
        .iter()
        .find_map(|phdr| {
            if phdr.p_type == elf::types::PT_LOAD {
                if (phdr.p_offset..phdr.p_offset + phdr.p_memsz).contains(&(file_off as u64)) {
                    return Some(file_off - phdr.p_offset as usize + phdr.p_vaddr as usize)
                }
            }
            None
        })
        .ok_or_else(|| {
            Error::new(
                ErrorKind::InvalidInput,
                format!(
                    "failed to find ELF segment in {} that contains file offset 0x{:x}",
                    entry.path.display(),
                    entry.offset,
                ),
            )
        })?;

    Ok(addr)
}


impl NormalizedUserAddrs {
    /// Add an unknown (non-normalizable) address to this object.
    ///
    /// This function accepts `unknown_idx` which, if not `None`, should
    /// contain the index into [`Self::meta`] at which an [`Unknown`]
    /// without any build ID resides.
    ///
    /// It returns the index of the inserted [`Unknown`] variant. The
    /// return type is an `Option` only for convenience of callers.
    /// Returned is always a `Some`.
    fn add_unknown_addr(&mut self, addr: Addr, unknown_idx: Option<usize>) -> Option<usize> {
        let unknown_idx = if let Some(unknown_idx) = unknown_idx {
            debug_assert_eq!(self.meta[unknown_idx], Unknown::default().into());
            unknown_idx
        } else {
            let unknown_idx = self.meta.len();
            let unknown = Unknown::default();
            let () = self.meta.push(UserAddrMeta::Unknown(unknown));
            unknown_idx
        };

        let () = self.addrs.push((addr, unknown_idx));
        Some(unknown_idx)
    }
}


/// Normalize `addresses` belonging to a process.
///
/// Normalize all `addrs` in a given process. The `addrs` array has to
/// be sorted in ascending order or an error will be returned.
///
/// Unknown addresses are not normalized. They are reported as
/// [`Unknown`] meta entries in the returned [`NormalizedUserAddrs`]
/// object. The cause of an address to be unknown (and, hence, not
/// normalized), could have a few reasons, including, but not limited
/// to:
/// - user error (if a bogus address was provided)
/// - they belonged to an ELF object that has been unmapped since the
///   address was captured
///
/// The process' ID should be provided in `pid`. To normalize addresses of the
/// calling processes, `0` can be provided as a sentinel for the current
/// process' ID.
///
/// Normalized addresses are reported in the exact same order in which the
/// non-normalized ones were provided.
pub fn normalize_user_addrs(addrs: &[Addr], pid: u32) -> Result<NormalizedUserAddrs> {
    let pid = Pid::from(pid);

    let mut entries = maps::parse(pid)?.filter_map(|result| {
        if let Ok(entry) = result {
            maps::is_symbolization_relevant(&entry).then(|| Ok(entry))
        } else {
            Some(result)
        }
    });
    let mut entry = entries.next().ok_or_else(|| {
        Error::new(
            ErrorKind::UnexpectedEof,
            format!("proc maps for {pid} does not contain relevant entries"),
        )
    })??;

    // Lookup table from path (as used in each proc maps entry) to index into
    // `normalized.meta`.
    let mut meta_lookup = HashMap::<PathBuf, usize>::new();
    let mut normalized = NormalizedUserAddrs {
        addrs: Vec::with_capacity(addrs.len()),
        meta: Vec::new(),
    };
    // The index of the Unknown entry without any build ID information,
    // used for all unknown addresses.
    let mut unknown_idx = None;

    let mut prev_addr = addrs.first().copied().unwrap_or_default();
    // We effectively do a single pass over `addrs`, advancing to the next
    // proc maps entry whenever the current address is not (or no longer)
    // contained in the current entry's range.
    'main: for addr in addrs.iter().copied() {
        if addr < prev_addr {
            return Err(Error::new(
                ErrorKind::InvalidInput,
                "addresses to normalize are not sorted",
            ))
        }
        prev_addr = addr;

        // proc maps entries are always sorted by start address. If the
        // current address lies before the start address at this point,
        // that means that we cannot find a suitable entry. This could
        // happen, for example, if an ELF object was unmapped between
        // address capture and normalization.
        if addr < entry.range.start {
            unknown_idx = normalized.add_unknown_addr(addr, unknown_idx);
            continue 'main
        }

        while addr >= entry.range.end {
            entry = if let Some(entry) = entries.next() {
                entry?
            } else {
                // If there are no proc maps entries left to check, we
                // cannot normalize. We have to assume that addresses
                // were valid and the ELF object was just unmapped,
                // similar to above.
                unknown_idx = normalized.add_unknown_addr(addr, unknown_idx);
                continue 'main
            };
        }

        let meta_idx = if let Some(meta_idx) = meta_lookup.get(&entry.path) {
            *meta_idx
        } else {
            let binary = Binary {
                path: entry.path.to_path_buf(),
                build_id: read_build_id(&entry.path)?,
                _non_exhaustive: (),
            };

            let meta_idx = normalized.meta.len();
            let () = normalized.meta.push(UserAddrMeta::Binary(binary));
            let _ref = meta_lookup.insert(entry.path.to_path_buf(), meta_idx);
            meta_idx
        };

        let normalized_addr = normalize_elf_addr(addr, &entry)?;
        let () = normalized.addrs.push((normalized_addr, meta_idx));
    }

    Ok(normalized)
}


#[cfg(test)]
mod tests {
    use super::*;

    use std::mem::transmute;
    use std::rc::Rc;

    use crate::mmap::Mmap;
    use crate::FindAddrOpts;
    use crate::SymbolType;


    /// Check that we can read a binary's build ID.
    #[test]
    fn build_id_reading() {
        let elf = Path::new(&env!("CARGO_MANIFEST_DIR"))
            .join("data")
            .join("libtest-so.so");

        let build_id = read_build_id(&elf).unwrap().unwrap();
        // The file contains a sha1 build ID, which is always 40 hex digits.
        assert_eq!(build_id.len(), 20, "'{build_id:?}'");

        // The shared object is explicitly built without build ID.
        let elf = Path::new(&env!("CARGO_MANIFEST_DIR"))
            .join("data")
            .join("test-no-debug.bin");
        let build_id = read_build_id(&elf).unwrap();
        assert_eq!(build_id, None);
    }

    /// Check that we detect unsorted input addresses.
    #[test]
    fn user_address_normalization_unsorted() {
        let mut addrs = [
            libc::__errno_location as Addr,
            libc::dlopen as Addr,
            libc::fopen as Addr,
        ];
        let () = addrs.sort();
        let () = addrs.swap(0, 1);

        let err = normalize_user_addrs(addrs.as_slice(), 0).unwrap_err();
        assert!(err.to_string().contains("are not sorted"), "{err}");
    }

    /// Check that we handle unknown addresses as expected.
    #[test]
    fn user_address_normalization_unknown() {
        // The very first page of the address space should never be
        // mapped, so use addresses from there.
        let addrs = [0x500 as Addr, 0x600 as Addr];

        let norm_addrs = normalize_user_addrs(addrs.as_slice(), 0).unwrap();
        assert_eq!(norm_addrs.addrs.len(), 2);
        assert_eq!(norm_addrs.meta.len(), 1);
        assert_eq!(norm_addrs.meta[0], Unknown::default().into());
        assert_eq!(norm_addrs.addrs[0].1, 0);
        assert_eq!(norm_addrs.addrs[1].1, 0);
    }

    /// Check that we can normalize user addresses.
    #[test]
    fn user_address_normalization() {
        let mut addrs = [
            libc::__errno_location as Addr,
            libc::dlopen as Addr,
            libc::fopen as Addr,
            build_id_reading as Addr,
            user_address_normalization as Addr,
            Mmap::map as Addr,
        ];
        let () = addrs.sort();

        let (errno_idx, _) = addrs
            .iter()
            .enumerate()
            .find(|(_idx, addr)| **addr == libc::__errno_location as Addr)
            .unwrap();

        let norm_addrs = normalize_user_addrs(addrs.as_slice(), 0).unwrap();
        assert_eq!(norm_addrs.addrs.len(), 6);

        let addrs = &norm_addrs.addrs;
        let meta = &norm_addrs.meta;
        assert_eq!(meta.len(), 2);

        let fopen_meta_idx = addrs[errno_idx].1;
        assert!(meta[fopen_meta_idx]
            .binary()
            .unwrap()
            .path
            .file_name()
            .unwrap()
            .to_string_lossy()
            .contains("libc.so"));
    }

    /// Check that we can normalize user addresses in our own shared object.
    #[test]
    fn user_address_normalization_custom_so() {
        let test_so = Path::new(&env!("CARGO_MANIFEST_DIR"))
            .join("data")
            .join("libtest-so.so");

        let mmap = Mmap::builder().exec().open(test_so).unwrap();
        let mmap = Rc::new(mmap);

        // Look up the address of the `the_answer` function inside of the shared
        // object.
        let elf_parser = ElfParser::from_mmap(mmap.clone(), 0).unwrap();
        let opts = FindAddrOpts {
            sym_type: SymbolType::Function,
            ..Default::default()
        };
        let symbols = elf_parser.find_address("the_answer", &opts).unwrap();
        // There is only one symbol with this address in there.
        assert_eq!(symbols.len(), 1);
        let symbol = symbols.first().unwrap();

        let the_answer_addr = unsafe { mmap.as_ptr().add(symbol.address) };
        // Now just double check that everything worked out and the function
        // is actually where it was meant to be.
        let the_answer_fn =
            unsafe { transmute::<_, extern "C" fn() -> libc::c_int>(the_answer_addr) };
        let answer = the_answer_fn();
        assert_eq!(answer, 42);

        let norm_addrs = normalize_user_addrs([the_answer_addr as Addr].as_slice(), 0).unwrap();
        assert_eq!(norm_addrs.addrs.len(), 1);
        assert_eq!(norm_addrs.meta.len(), 1);

        let norm_addr = norm_addrs.addrs[0];
        assert_eq!(norm_addr.0, symbol.address);
        let meta = &norm_addrs.meta[norm_addr.1];
        let so_path = Path::new(&env!("CARGO_MANIFEST_DIR"))
            .join("data")
            .join("libtest-so.so");
        let expected_binary = Binary {
            build_id: Some(read_build_id(&so_path).unwrap().unwrap()),
            path: so_path,
            _non_exhaustive: (),
        };
        assert_eq!(meta, &UserAddrMeta::Binary(expected_binary));
    }
}
