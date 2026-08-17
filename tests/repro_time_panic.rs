#[test]
fn test_parse_flexible_timestamp_panic_on_multibyte() {
    let input = "+10🚀";
    let result = obr::util::time::parse_flexible_timestamp(input, "test");
    // Should return Err, not panic
    assert!(result.is_err());
}

#[test]
fn test_parse_relative_time_panic_on_multibyte() {
    let input = "+10🚀";
    let result = obr::util::time::parse_relative_time(input);
    // Should return None, not panic
    assert!(result.is_none());
}
