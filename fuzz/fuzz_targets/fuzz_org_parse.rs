#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &str| {
    // Fuzz the Org-mode text to issues parser.
    // This is the primary input boundary for Org-mode import.
    let _ = beads_rust::sync::org_bridge::org_text_to_issues(data);
});
