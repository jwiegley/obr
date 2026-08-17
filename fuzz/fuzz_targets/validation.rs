#![no_main]

use libfuzzer_sys::fuzz_target;
use std::str::FromStr;

fuzz_target!(|data: &str| {
    // Fuzz the string-based parsers and validators at the CLI input boundary.
    let _ = obr::validation::is_valid_id_format(data);
    let _ = obr::validation::LabelValidator::validate(data);
    let _ = obr::model::Status::from_str(data);
    let _ = obr::model::Priority::from_str(data);
    let _ = obr::model::IssueType::from_str(data);
});
