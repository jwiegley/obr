#![no_main]

use libfuzzer_sys::fuzz_target;
use std::str::FromStr;

fuzz_target!(|data: &str| {
    // Fuzz all string-based parsers and validators.
    let _ = beads_rust::validation::is_valid_id_format(data);
    let _ = beads_rust::validation::LabelValidator::validate(data);
    let _ = beads_rust::model::Status::from_str(data);
    let _ = beads_rust::model::Priority::from_str(data);
    let _ = beads_rust::model::IssueType::from_str(data);
});
