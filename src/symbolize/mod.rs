//! Functionality for symbolizing addresses.
//!
//! This module contains functionality for symbolizing addresses, i.e., finding
//! symbol names and other information based on "raw" addresses.
//!
//! For example, here we symbolize the backtrace captured via `libc`'s
//! `backtrace` function:
//! ```no_run
//! # use std::cmp::min;
//! # use std::mem::size_of;
//! # use std::mem::transmute;
//! # use std::ptr;
//! use blazesym::symbolize::Source;
//! use blazesym::symbolize::Process;
//! use blazesym::symbolize::Sym;
//! use blazesym::symbolize::Symbolizer;
//! use blazesym::Addr;
//! use blazesym::Pid;
//!
//! # assert_eq!(size_of::<*mut libc::c_void>(), size_of::<Addr>());
//! // Retrieve up to 64 stack frames of the calling thread.
//! const MAX_CNT: usize = 64;
//!
//! let mut bt_buf = [ptr::null_mut::<libc::c_void>(); MAX_CNT];
//! let bt_cnt = unsafe { libc::backtrace(bt_buf.as_mut_ptr(), MAX_CNT as _) } as usize;
//! let bt = &bt_buf[0..min(bt_cnt, MAX_CNT)];
//! # let bt = unsafe { transmute::<&[*mut libc::c_void], &[Addr]>(bt) };
//!
//! // Symbolize the addresses for the current process, as that's where
//! // they were captured.
//! let src = Source::Process(Process::new(Pid::Slf));
//! let symbolizer = Symbolizer::new();
//!
//! let bt_syms = symbolizer.symbolize(&src, bt).unwrap();
//! for (addr, syms) in addrs.into_iter().zip(syms) {
//!     let mut addr_fmt = format!("0x{addr:016x}:");
//!     if syms.is_empty() {
//!         println!("{addr_fmt} <no-symbol>")
//!     } else {
//!         for (i, sym) in syms.into_iter().enumerate() {
//!             if i == 1 {
//!                 addr_fmt = addr_fmt.replace(|_c| true, " ");
//!             }
//!
//!             let Sym {
//!                 name,
//!                 addr: sym_addr,
//!                 path,
//!                 line,
//!                 ..
//!             } = sym;
//!
//!             println!(
//!                 "{addr_fmt} {name} @ 0x{sym_addr:x}+0x{:x} {}:{line}",
//!                 addr - sym_addr,
//!                 path.display(),
//!             );
//!         }
//!     }
//! }
//! ```

mod source;
mod symbolizer;

use std::path::PathBuf;

pub use source::Elf;
pub use source::Gsym;
pub use source::GsymData;
pub use source::GsymFile;
pub use source::Kernel;
pub use source::Process;
pub use source::Source;
pub use symbolizer::Builder;
pub use symbolizer::Sym;
pub use symbolizer::Symbolizer;


pub(crate) struct AddrLineInfo {
    pub path: PathBuf,
    pub line: u32,
    pub column: u16,
}
