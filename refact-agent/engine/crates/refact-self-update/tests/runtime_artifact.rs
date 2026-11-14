use refact_self_update::runtime_artifact::{verify_runtime_artifact, ArtifactLimits};
use refact_self_update::runtime_manifest::RuntimeManifest;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::io::{Cursor, Write};

const LINUX: &str = "x86_64-unknown-linux-gnu";
const WINDOWS: &str = "x86_64-pc-windows-msvc";
const BODY: &[u8] = b"fake runtime bytes, never executed";
fn hash(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}
fn limits() -> ArtifactLimits {
    ArtifactLimits {
        archive_bytes: 65536,
        runtime_bytes: 1024,
    }
}
fn wire(archive: &[u8], target: &str) -> Value {
    json!({
        "schema_version":1,
        "repository":{"owner":"fixture","name":"runtime","id":1},
        "source_sha":"a".repeat(40),
        "workflow":{"path":".github/workflows/build.yml","id":2},
        "run":{"id":3,"attempt":1},
        "build":{"version":"1.2.3","channel":"nightly"},
        "build_inputs":{"rust_toolchain":"1.90.0","node_version":"22.0.0","profile":"release",
            "cargo_lock_sha256":"b".repeat(64),"javascript_locks_sha256":"c".repeat(64),"config_sha256":"d".repeat(64)},
        "component_versions":{"runtime":"1.2.3","gui":"1.2.3","vscode":"1.2.3","jetbrains":"1.2.3"},
        "compatibility":{"daemon_api":{"min":1,"max":1},"worker_api":{"min":1,"max":1},"host_bridge":{"min":1,"max":1}},
        "artifacts":[{"component":"runtime","target":target,"filename":"runtime.bin",
            "format":if target == WINDOWS {"zip"} else {"tar_gz"},"size":archive.len(),"sha256":hash(archive),
            "extracted_files":[{"path":if target == WINDOWS {"refact.exe"} else {"refact"},"size":BODY.len(),"sha256":hash(BODY)}]}],
        "tests":"not_run"
    })
}
fn manifest(wire: &Value) -> RuntimeManifest {
    RuntimeManifest::from_json(&serde_json::to_vec(wire).unwrap()).unwrap()
}
fn tar(entries: &[(&str, u8, u32)]) -> Vec<u8> {
    let encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    let mut builder = tar::Builder::new(encoder);
    for (name, kind, mode) in entries {
        let mut header = tar::Header::new_gnu();
        header.set_path(name).unwrap();
        header.set_entry_type(tar::EntryType::new(*kind));
        header.set_mode(*mode);
        header.set_size(BODY.len() as u64);
        if *kind == b'1' || *kind == b'2' {
            header.set_link_name("refact").unwrap();
        }
        header.set_cksum();
        builder.append(&header, Cursor::new(BODY)).unwrap();
    }
    builder.into_inner().unwrap().finish().unwrap()
}
fn zip(names: &[&str]) -> Vec<u8> {
    zip_method(names, zip::CompressionMethod::Deflated)
}
fn zip_method(names: &[&str], method: zip::CompressionMethod) -> Vec<u8> {
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for name in names {
        writer
            .start_file(
                *name,
                zip::write::SimpleFileOptions::default()
                    .unix_permissions(0o644)
                    .compression_method(method),
            )
            .unwrap();
        writer.write_all(BODY).unwrap();
    }
    writer.finish().unwrap().into_inner()
}
#[test]
fn correct_tar_and_zip_return_exact_bytes_and_manifest_identity() {
    for (target, archive) in [
        (LINUX, tar(&[("refact", b'0', 0o755)])),
        (WINDOWS, zip(&["refact.exe"])),
    ] {
        let m = manifest(&wire(&archive, target));
        let verified = verify_runtime_artifact(&m, target, &archive, limits()).unwrap();
        assert_eq!(verified.bytes(), BODY);
        assert_eq!(verified.target(), target);
        assert_eq!(verified.manifest_digest(), m.digest());
        assert_eq!(
            verified.identity().component_versions(),
            m.component_versions()
        );
    }
}
#[test]
fn compressed_and_extracted_metadata_are_independently_bound() {
    let archive = tar(&[("refact", b'0', 0o755)]);
    for (field, value, expected) in [
        ("size", json!(archive.len() + 1), "archive size mismatch"),
        ("sha256", json!("0".repeat(64)), "archive checksum mismatch"),
    ] {
        let mut w = wire(&archive, LINUX);
        w["artifacts"][0][field] = value;
        assert_eq!(
            verify_runtime_artifact(&manifest(&w), LINUX, &archive, limits()).unwrap_err(),
            expected
        );
    }
    for (field, value, expected) in [
        ("size", json!(BODY.len() + 1), "runtime size mismatch"),
        ("sha256", json!("0".repeat(64)), "runtime checksum mismatch"),
    ] {
        let mut w = wire(&archive, LINUX);
        w["artifacts"][0]["extracted_files"][0][field] = value;
        assert_eq!(
            verify_runtime_artifact(&manifest(&w), LINUX, &archive, limits()).unwrap_err(),
            expected
        );
    }
}
#[test]
fn declared_sizes_and_caller_budgets_fail_before_large_allocation() {
    let archive = tar(&[("refact", b'0', 0o755)]);
    let mut w = wire(&archive, LINUX);
    w["artifacts"][0]["extracted_files"][0]["size"] = json!(u64::MAX);
    assert_eq!(
        verify_runtime_artifact(&manifest(&w), LINUX, &archive, limits()).unwrap_err(),
        "runtime byte limit exceeded"
    );
    let m = manifest(&wire(&archive, LINUX));
    let mut small = limits();
    small.archive_bytes = archive.len() as u64 - 1;
    assert_eq!(
        verify_runtime_artifact(&m, LINUX, &archive, small).unwrap_err(),
        "archive byte limit exceeded"
    );
    small = limits();
    small.runtime_bytes = BODY.len() as u64 - 1;
    assert_eq!(
        verify_runtime_artifact(&m, LINUX, &archive, small).unwrap_err(),
        "runtime byte limit exceeded"
    );
}
#[test]
fn tar_rejects_links_directories_extensions_wrong_mode_paths_and_extra_members() {
    let valid = tar(&[("refact", b'0', 0o755)]);
    assert!(
        verify_runtime_artifact(&manifest(&wire(&valid, LINUX)), LINUX, &valid, limits()).is_ok()
    );
    for entries in [
        vec![("refact", b'1', 0o755)],
        vec![("refact", b'2', 0o755)],
        vec![("refact", b'5', 0o755)],
        vec![("refact", b'x', 0o755)],
        vec![("refact", b'0', 0o644)],
        vec![("nested/refact", b'0', 0o755)],
        vec![("Refact", b'0', 0o755)],
        vec![],
        vec![("refact", b'0', 0o755), ("refact", b'0', 0o755)],
        vec![("refact", b'0', 0o755), ("extra", b'0', 0o755)],
    ] {
        let archive = tar(&entries);
        assert!(
            verify_runtime_artifact(&manifest(&wire(&archive, LINUX)), LINUX, &archive, limits())
                .is_err(),
            "accepted {entries:?}"
        );
    }
}
#[test]
fn zip_rejects_traversal_absolute_nested_extra_missing_and_case_aliases() {
    for names in [
        vec!["../refact.exe"],
        vec!["/refact.exe"],
        vec!["nested/refact.exe"],
        vec!["refact.exe", "extra"],
        vec![],
        vec!["REFACT.EXE"],
        vec!["refact.exe/"],
    ] {
        let archive = zip(&names);
        assert!(
            verify_runtime_artifact(
                &manifest(&wire(&archive, WINDOWS)),
                WINDOWS,
                &archive,
                limits()
            )
            .is_err(),
            "accepted {names:?}"
        );
    }
}
#[test]
fn selected_target_and_manifest_file_contract_are_exact() {
    let archive = tar(&[("refact", b'0', 0o755)]);
    let w = wire(&archive, LINUX);
    assert_eq!(
        verify_runtime_artifact(&manifest(&w), WINDOWS, &archive, limits()).unwrap_err(),
        "runtime target absent"
    );
    for files in [
        json!([]),
        json!([{"path":"nested/refact","size":BODY.len(),"sha256":hash(BODY)}]),
    ] {
        let mut changed = w.clone();
        changed["artifacts"][0]["extracted_files"] = files;
        assert_eq!(
            verify_runtime_artifact(&manifest(&changed), LINUX, &archive, limits()).unwrap_err(),
            "manifest must bind exactly the root runtime file"
        );
    }
    let mut duplicate = w.clone();
    let mut extra = duplicate["artifacts"][0].clone();
    extra["filename"] = json!("second.bin");
    duplicate["artifacts"].as_array_mut().unwrap().push(extra);
    assert!(RuntimeManifest::from_json(&serde_json::to_vec(&duplicate).unwrap()).is_err());
}

fn zip_offsets(bytes: &[u8]) -> (usize, usize) {
    let end = bytes.windows(4).rposition(|b| b == b"PK\x05\x06").unwrap();
    let central = u32::from_le_bytes(bytes[end + 16..end + 20].try_into().unwrap()) as usize;
    (central, end)
}
fn zip_attributes(bytes: &mut [u8], creator: u8, attributes: u32) {
    let (central, _) = zip_offsets(bytes);
    bytes[central + 5] = creator;
    bytes[central + 38..central + 42].copy_from_slice(&attributes.to_le_bytes());
}
#[test]
fn dos_stored_and_deflated_files_accept_native_and_python_attributes() {
    for method in [
        zip::CompressionMethod::Stored,
        zip::CompressionMethod::Deflated,
    ] {
        for attributes in [
            0,
            0x20,
            0x21,
            0x27,
            (0o100666 << 16) | 0x20,
            (0o100444 << 16) | 1,
        ] {
            let mut archive = zip_method(&["refact.exe"], method);
            zip_attributes(&mut archive, 0, attributes);
            let verified = verify_runtime_artifact(
                &manifest(&wire(&archive, WINDOWS)),
                WINDOWS,
                &archive,
                limits(),
            )
            .unwrap();
            assert_eq!(verified.bytes(), BODY);
        }
    }
}
#[test]
fn zip_rejects_dos_directory_volume_reserved_and_contradictory_types() {
    for (creator, attributes) in [
        (0, 0x10),
        (0, 0x08),
        (0, 0x40),
        (0, 0x100),
        (0, (0o120777 << 16) | 0x20),
        (0, (0o040755 << 16) | 0x20),
        (0, (0o104755 << 16) | 0x20),
        (3, 0o120777 << 16),
        (3, 0o100666 << 16),
        (3, 0o104755 << 16),
        (3, 0),
        (10, 0x20),
    ] {
        let mut archive = zip(&["refact.exe"]);
        zip_attributes(&mut archive, creator, attributes);
        assert!(
            verify_runtime_artifact(
                &manifest(&wire(&archive, WINDOWS)),
                WINDOWS,
                &archive,
                limits()
            )
            .is_err(),
            "accepted creator {creator} attributes {attributes:o}"
        );
    }
}
#[test]
fn multichunk_runtime_preserves_exact_bytes_with_bounded_growth() {
    let body: Vec<u8> = (0..300_000).map(|i| (i % 251) as u8).collect();
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    writer
        .start_file(
            "refact.exe",
            zip::write::SimpleFileOptions::default().unix_permissions(0o644),
        )
        .unwrap();
    writer.write_all(&body).unwrap();
    let archive = writer.finish().unwrap().into_inner();
    let mut w = wire(&archive, WINDOWS);
    w["artifacts"][0]["extracted_files"][0]["size"] = json!(body.len());
    w["artifacts"][0]["extracted_files"][0]["sha256"] = json!(hash(&body));
    let limits = ArtifactLimits {
        archive_bytes: archive.len() as u64,
        runtime_bytes: body.len() as u64,
    };
    assert_eq!(
        verify_runtime_artifact(&manifest(&w), WINDOWS, &archive, limits)
            .unwrap()
            .bytes(),
        body
    );
    let smaller = ArtifactLimits {
        runtime_bytes: body.len() as u64 - 1,
        ..limits
    };
    assert_eq!(
        verify_runtime_artifact(&manifest(&w), WINDOWS, &archive, smaller).unwrap_err(),
        "runtime byte limit exceeded"
    );
}
fn set_u32(bytes: &mut [u8], offset: usize, value: usize) {
    bytes[offset..offset + 4].copy_from_slice(&(value as u32).to_le_bytes());
}
fn zip_error(bytes: &[u8]) -> String {
    verify_runtime_artifact(&manifest(&wire(bytes, WINDOWS)), WINDOWS, bytes, limits()).unwrap_err()
}
#[test]
fn zip_rejects_validly_hashed_unreferenced_local_record() {
    let normal = zip(&["refact.exe"]);
    let (central, end) = zip_offsets(&normal);
    let extra = zip(&["extra.bin"]);
    let (extra_central, _) = zip_offsets(&extra);
    let hidden_local = &extra[..extra_central];
    let mut malicious = normal.clone();
    malicious.splice(central..central, hidden_local.iter().copied());
    set_u32(
        &mut malicious,
        end + hidden_local.len() + 16,
        central + hidden_local.len(),
    );
    // A generic central-directory reader still sees one valid file with the expected payload.
    let mut generic = zip::ZipArchive::new(Cursor::new(&malicious)).unwrap();
    assert_eq!(generic.len(), 1);
    let mut body = Vec::new();
    std::io::Read::read_to_end(&mut generic.by_index(0).unwrap(), &mut body).unwrap();
    assert_eq!(body, BODY);
    assert_eq!(zip_error(&malicious), "zip local extent rejected");
}
#[test]
fn zip_requires_exact_central_extent_and_one_complete_record() {
    let normal = zip(&["refact.exe"]);
    let (central, end) = zip_offsets(&normal);
    let mut junk = normal.clone();
    junk.splice(end..end, [0x42]);
    assert_eq!(zip_error(&junk), "zip central extent rejected");
    let mut hidden_central = normal.clone();
    hidden_central.splice(end..end, normal[central..end].iter().copied());
    set_u32(
        &mut hidden_central,
        end + (end - central) + 12,
        2 * (end - central),
    );
    assert_eq!(zip_error(&hidden_central), "zip central record rejected");
}
#[test]
fn zip_rejects_descriptors_zip64_split_and_local_central_disagreement() {
    let normal = zip(&["refact.exe"]);
    let (central, end) = zip_offsets(&normal);
    let mut descriptor = normal.clone();
    descriptor[6] |= 8;
    descriptor[central + 8] |= 8;
    assert_eq!(zip_error(&descriptor), "zip flags/method rejected");
    let mut zip64 = normal.clone();
    zip64[4] = 45;
    zip64[central + 6] = 45;
    assert_eq!(zip_error(&zip64), "zip central record rejected");
    let mut split = normal.clone();
    split[end + 4] = 1;
    assert_eq!(zip_error(&split), "zip must contain exactly one member");
    for offset in [6, 8, 14, 18, 22] {
        let mut mismatch = normal.clone();
        mismatch[offset] ^= 1;
        assert_eq!(zip_error(&mismatch), "zip local/central mismatch");
    }
    let mut name = normal.clone();
    name[30] = b'X';
    assert_eq!(zip_error(&name), "zip member path/type/encryption rejected");
}
