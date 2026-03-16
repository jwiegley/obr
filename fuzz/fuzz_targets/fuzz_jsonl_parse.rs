#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &str| {
    // Fuzz the JSONL line parser: each line should be a JSON Issue.
    // This exercises serde_json deserialization of arbitrary input.
    let _ = serde_json::from_str::<beads_rust::model::Issue>(data);
});
