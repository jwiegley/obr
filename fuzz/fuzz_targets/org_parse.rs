#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &str| {
    // Fuzz the Org-mode text -> issues parser: the primary input boundary
    // for the default export format's import path.
    let _ = obr::sync::org_bridge::org_text_to_issues(data);
    // The light heading-id scan used by export verification and doctor.
    let _ = obr::sync::org_bridge::org_heading_ids(data);
    // The failure-collecting parse used by validation summaries.
    let _ = obr::sync::org_bridge::parse_issues_collecting_failures(data);
});
