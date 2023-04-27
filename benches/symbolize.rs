use blazesym::c_api;
use blazesym::cfg;
use blazesym::Addr;
use blazesym::BlazeSymbolizer;
use blazesym::SymbolSrcCfg;

use criterion::measurement::Measurement;
use criterion::BenchmarkGroup;


/// Symbolize addresses in the current process.
fn symbolize_process() {
    let cfg = SymbolSrcCfg::Process(cfg::Process { pid: None });
    let addrs = [
        libc::__errno_location as Addr,
        libc::dlopen as Addr,
        libc::fopen as Addr,
        symbolize_process as Addr,
        c_api::blazesym_find_addresses as Addr,
    ];

    let symbolizer = BlazeSymbolizer::new().unwrap();
    let results = symbolizer.symbolize(&cfg, &addrs).unwrap();
    assert_eq!(results.len(), addrs.len());
}

pub fn benchmark<M>(group: &mut BenchmarkGroup<'_, M>)
where
    M: Measurement,
{
    group.bench_function(stringify!(symbolize::symbolize_process), |b| {
        b.iter(symbolize_process)
    });
}
