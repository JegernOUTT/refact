use std::fs;
use std::path::Path;

const APP_RS_PRODUCTION_LINE_CAP: usize = 1_850;
const RATCHET_SCHEDULE: &str = "1,850 production lines → semantic extraction caps → 2,000";
const INLINE_TEST_BOUNDARY: &str = "\n#[cfg(test)]\nmod tests {";

#[test]
fn app_rs_stays_within_the_current_ratchet_cap() {
    let app_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/app.rs");
    let source = fs::read_to_string(&app_path)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", app_path.display()));
    let production = source
        .split_once(INLINE_TEST_BOUNDARY)
        .map(|(production, _)| production)
        .unwrap_or(&source);
    let current_line_count = production.lines().count();

    assert!(
        current_line_count <= APP_RS_PRODUCTION_LINE_CAP,
        "src/app.rs has {current_line_count} production lines; cap is {APP_RS_PRODUCTION_LINE_CAP}. Ratchet schedule: {RATCHET_SCHEDULE}. The cap may only decrease."
    );
}
