extern crate blazesym;

use std::env;
use blazesym::{BlazeSymbolizer, SymbolFileCfg, SymbolizedResult};

fn show_usage() {
    let args: Vec<String> = env::args().collect();
    println!("Usage: {} <kernel> <binary> <address>", args[0]);
}

fn main() {
    let args: Vec<String> = env::args().collect();

    if args.len() != 4 {
	show_usage();
	return;
    }

    let kern_name = args[1].clone();
    let bin_name = args[2].clone();
    let mut addr_str = &args[3][..];

    if &addr_str[0..2] == "0x" {
	// Remove prefixed 0x
	addr_str = &addr_str[2..];
    }
    let addr = u64::from_str_radix(addr_str, 16).unwrap();

    let sym_files = [SymbolFileCfg::Elf { file_name: bin_name, loaded_address: 0 },
		     SymbolFileCfg::Kernel { kallsyms: String::from("/proc/kallsyms"),
						  kernel_image: kern_name }];
    let resolver = BlazeSymbolizer::new().unwrap();
    let symlist = resolver.symbolize(&sym_files, &[addr]);
    if let Some(SymbolizedResult {symbol, start_address, path, line_no, column}) = &symlist[0] {
	println!("0x{:x} {}@0x{:x} {}:{}:{}", addr, symbol, start_address, path, line_no, column);
    } else {
	println!("0x{:x} is not found", addr);
    }
}
