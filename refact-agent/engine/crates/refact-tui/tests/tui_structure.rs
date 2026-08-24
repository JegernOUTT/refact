use std::fs;
use std::path::Path;

const APP_RS_PRODUCTION_LINE_CAP: usize = 1_510;
const STATE_RS_PRODUCTION_LINE_CAP: usize = 2_000;
const RATCHET_SCHEDULE: &str = "1,510 app.rs production lines → 2,000 state.rs production lines";
const INLINE_TEST_BOUNDARY: &str = "\n#[cfg(test)]\nmod tests {";

#[test]
fn app_rs_stays_within_the_current_ratchet_cap() {
    assert_production_line_cap("src/app.rs", APP_RS_PRODUCTION_LINE_CAP);
}

#[test]
fn state_rs_stays_within_the_current_ratchet_cap() {
    assert_production_line_cap("src/app/state.rs", STATE_RS_PRODUCTION_LINE_CAP);
}

fn assert_production_line_cap(relative_path: &str, cap: usize) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(relative_path);
    let source = fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()));
    let production = source
        .split_once(INLINE_TEST_BOUNDARY)
        .map(|(production, _)| production)
        .unwrap_or(&source);
    let current_line_count = production.lines().count();

    assert!(
        current_line_count <= cap,
        "{relative_path} has {current_line_count} production lines; cap is {cap}. Ratchet schedule: {RATCHET_SCHEDULE}. The cap may only decrease."
    );
}
