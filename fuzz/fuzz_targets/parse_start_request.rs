#![no_main]

use std::path::PathBuf;

use gtc::start_stop_parsing::parse_start_request;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let tail = data
        .split(|byte| *byte == 0)
        .map(|part| String::from_utf8_lossy(part).into_owned())
        .take(32)
        .collect::<Vec<_>>();
    let _ = parse_start_request(&tail, PathBuf::from("/tmp/fuzz-bundle"));
});
