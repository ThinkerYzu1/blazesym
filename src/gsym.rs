use std::fs::File;
use std::io::{Error, Read};
use std::mem;
use std::path::PathBuf;

use super::{AddressLineInfo, FindAddrOpts, SymResolver, SymbolInfo};

mod parser;
mod types;

use parser::{find_address, GsymContext};

/// The symbol resolver for the GSYM format.
pub struct GsymResolver {
    file_name: PathBuf,
    ctx: GsymContext<'static>,
    #[allow(dead_code)]
    data: Vec<u8>,
    loaded_address: u64,
}

impl GsymResolver {
    pub fn new(file_name: PathBuf, loaded_address: u64) -> Result<GsymResolver, Error> {
        let mut fo = File::open(&file_name)?;
        let mut data = vec![];
        fo.read_to_end(&mut data)?;
        let ctx = GsymContext::parse_header(&data)?;

        Ok(GsymResolver {
            file_name,
            ctx: unsafe { mem::transmute(ctx) },
            data,
            loaded_address,
        })
    }
}

impl SymResolver for GsymResolver {
    fn get_address_range(&self) -> (u64, u64) {
        let sz = self.ctx.num_addresses();
        if sz == 0 {
            return (0, 0);
        }

        let start = self.ctx.addr_at(0) + self.loaded_address;
        let end =
            self.ctx.addr_at(sz - 1) + self.ctx.addr_info(sz - 1).size as u64 + self.loaded_address;
        (start, end)
    }

    fn find_symbol(&self, addr: u64) -> Option<(&str, u64)> {
        let addr = addr - self.loaded_address;
        let idx = find_address(&self.ctx, addr);
        let found = self.ctx.addr_at(idx);
        if addr < found {
            return None;
        }

        let info = self.ctx.addr_info(idx);
        let name = self.ctx.get_str(info.name as usize);
        Some((name, found + self.loaded_address))
    }

    fn find_address(&self, _name: &str, _opts: &FindAddrOpts) -> Option<Vec<SymbolInfo>> {
        // It is inefficient to find the address of a symbol with
        // GSYM.  We may support it in the future if needed.
        None
    }

    fn find_address_regex(&self, _pattern: &str, _opts: &FindAddrOpts) -> Option<Vec<SymbolInfo>> {
        None
    }

    fn addr_file_off(&self, _addr: u64) -> Option<u64> {
        // Unavailable
        None
    }

    fn get_obj_file_name(&self) -> String {
        self.file_name.to_str().unwrap().to_string()
    }

    fn find_line_info(&self, _addr: u64) -> Option<AddressLineInfo> {
        None
    }

    fn repr(&self) -> String {
        format!("GSYM {:?}", self.file_name)
    }
}
