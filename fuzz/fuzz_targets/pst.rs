//! A PST file, opened and read through: every folder and every message.
//!
//! Give it a real file to begin from, or it never gets past the first
//! bytes: `cargo run -p brolga-pst --example make_big -- fuzz/corpus/pst/seed.pst 0 0 0`.
#![no_main]

use std::io::Write;

use brolga_pst::Pst;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|file: &[u8]| {
    let path = std::env::temp_dir().join(format!("brolga-fuzz-{}.pst", std::process::id()));
    let written = std::fs::File::create(&path).and_then(|mut f| f.write_all(file));
    if written.is_err() {
        return;
    }

    if let Ok(pst) = Pst::open(&path)
        && let Ok((folders, _)) = pst.folders()
    {
        for message in folders.iter().flat_map(|f| &f.messages).take(200) {
            let _ = pst.message(*message);
        }
    }
});
