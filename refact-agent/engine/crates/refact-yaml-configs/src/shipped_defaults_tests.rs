use crate::project_configs_bootstrap::{
    compute_checksum, embedded_defaults, global_configs_try_create_all,
};
use serde::Deserialize;
use std::collections::HashMap;
use tokio::fs;

const BOOTSTRAP_KINDS: [&str; 5] = [
    "modes",
    "subagents",
    "toolbox_commands",
    "code_lens",
    "review_stages",
];

type ShippedDefaults = HashMap<String, Vec<(u32, String)>>;

#[derive(Deserialize)]
struct SchemaVersion {
    schema_version: u32,
}

fn shipped_defaults() -> ShippedDefaults {
    serde_json::from_str(include_str!("shipped_defaults.json"))
        .expect("shipped defaults manifest should be valid JSON")
}

#[test]
fn every_embedded_default_is_recorded_in_the_manifest() {
    let manifest = shipped_defaults();

    for kind in BOOTSTRAP_KINDS {
        for (filename, content) in embedded_defaults(kind) {
            let key = format!("{kind}/{filename}");
            let version = serde_yaml::from_str::<SchemaVersion>(&content)
                .expect("embedded default should contain a valid schema_version")
                .schema_version;
            let checksum = compute_checksum(&content);
            let entries = manifest.get(&key).map(Vec::as_slice).unwrap_or(&[]);
            let max_recorded = entries
                .iter()
                .map(|(version, _)| *version)
                .max()
                .unwrap_or(0);
            let recorded = entries
                .iter()
                .any(|entry| entry == &(version, checksum.clone()));

            assert!(
                recorded,
                "{key} changed: bump schema_version above {max_recorded} and append [{version}, {checksum}] to shipped_defaults.json"
            );
        }
    }
}

#[test]
fn manifest_versions_are_unique_and_increasing() {
    for (key, entries) in shipped_defaults() {
        let mut previous = None;
        for (version, _) in entries {
            assert!(
                previous.map_or(true, |previous| version > previous),
                "{key} manifest schema versions must be unique and strictly increasing"
            );
            previous = Some(version);
        }
    }
}

#[tokio::test]
async fn bumped_default_upgrades_an_unmodified_install() {
    let manifest = shipped_defaults();
    let main_ask = embedded_defaults("modes")
        .into_iter()
        .find(|(filename, _)| filename == "ask.yaml")
        .map(|(_, content)| {
            content
                .replace("schema_version: 14\n", "schema_version: 13\n")
                .replace("  - agent_mailbox\n  - agent_peer_grant\n", "")
                .replace("    - match: \"agent_peer_grant\"\n      action: ask\n", "")
        })
        .expect("embedded mode ask.yaml should exist");
    let expected_baseline = manifest
        .get("modes/ask.yaml")
        .and_then(|entries| entries.iter().find(|(version, _)| *version == 13))
        .map(|(_, checksum)| checksum)
        .expect("manifest should contain main's ask.yaml baseline");
    assert_eq!(compute_checksum(&main_ask), *expected_baseline);

    let temp = tempfile::tempdir().expect("temporary config directory should be created");
    let config_dir = temp.path();
    let ask_path = config_dir.join("modes/ask.yaml");
    fs::create_dir_all(ask_path.parent().expect("ask.yaml has a parent"))
        .await
        .expect("modes directory should be created");
    fs::write(&ask_path, &main_ask)
        .await
        .expect("main's ask.yaml should be installed");
    let checksums = HashMap::from([("modes/ask.yaml".to_string(), expected_baseline.clone())]);
    fs::write(
        config_dir.join("default-checksums.yaml"),
        serde_yaml::to_string(&checksums).expect("baseline should serialize"),
    )
    .await
    .expect("main's checksum baseline should be installed");

    global_configs_try_create_all(config_dir)
        .await
        .expect("bootstrap should upgrade the installed default");

    let installed = fs::read_to_string(ask_path)
        .await
        .expect("upgraded ask.yaml should be readable");
    let embedded_ask = embedded_defaults("modes")
        .into_iter()
        .find(|(filename, _)| filename == "ask.yaml")
        .map(|(_, content)| content)
        .expect("embedded mode ask.yaml should exist");
    assert_eq!(installed, embedded_ask);
    assert!(installed.contains("  - agent_mailbox"));
}
