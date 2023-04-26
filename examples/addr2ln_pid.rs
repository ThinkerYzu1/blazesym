extern crate blazesym;

use blazesym::cfg;
use blazesym::Addr;
use blazesym::BlazeSymbolizer;
use blazesym::SymbolSrcCfg;
use blazesym::SymbolizedResult;
use std::env;

fn show_usage() {
    let args: Vec<String> = env::args().collect();
    println!("Usage: {} <pid> <address>", args[0]);
    println!("Resolve an address in the process of the given pid, and");
    println!("print its symbol, the file name of the source, and the line number.");
}

fn main() {
    let args: Vec<String> = env::args().collect();

    if args.len() != 3 {
        show_usage();
        return
    }

    let pid = args[1].parse::<u32>().unwrap();
    let mut addr_str = &args[2][..];
    println!("PID: {pid}");

    if addr_str.len() > 2 && &addr_str[0..2] == "0x" {
        // Remove prefixed 0x
        addr_str = &addr_str[2..];
    }
    let addr = Addr::from_str_radix(addr_str, 16).unwrap();

    let cfg = SymbolSrcCfg::Process(cfg::Process { pid: pid.into() });
    let resolver = BlazeSymbolizer::new().unwrap();
    let symlist = resolver.symbolize(&cfg, &[addr]).unwrap();
    if !symlist[0].is_empty() {
        let SymbolizedResult {
            symbol,
            start_address,
            path,
            line_no,
            column: _,
        } = &symlist[0][0];
        println!(
            "0x{:x} {}@0x{:x}+{} {}:{}",
            addr,
            symbol,
            start_address,
            addr - start_address,
            path.display(),
            line_no
        );
    } else {
        println!("0x{addr:x} is not found");
    }
}
