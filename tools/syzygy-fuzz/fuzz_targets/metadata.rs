#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|input: &[u8]| {
    pyrrhic_rs::fuzz_support::parse_metadata(input);
});
