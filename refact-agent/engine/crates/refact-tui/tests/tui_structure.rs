use std::fs;
use std::path::Path;

const APP_RS_LINE_CAP: usize = 11_530;
const RATCHET_SCHEDULE: &str = "11,530 → 9,800 → 7,000 → 5,000 → 2,000";

#[test]
fn app_rs_stays_within_the_current_ratchet_cap() {
    let app_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/app.rs");
    let source = fs::read_to_string(&app_path)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", app_path.display()));
    let current_line_count = source.lines().count();

    assert!(
        current_line_count <= APP_RS_LINE_CAP,
        "src/app.rs has {current_line_count} lines; cap is {APP_RS_LINE_CAP}. Ratchet schedule: {RATCHET_SCHEDULE}. The cap may only decrease."
    );
}
