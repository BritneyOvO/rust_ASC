#![cfg(feature = "harmony")]
use std::process::Command;
use serde_json::Value;
use tempfile::tempdir;
const CLASS: &[u8] = include_bytes!("../crates/asc-abc-dec/tests/fixtures/class.abc");

#[test]
fn bare_abc_is_detected_by_magic_and_renders_source_and_assembly() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("misleading.apk");
    std::fs::write(&path, CLASS).unwrap();
    let input = path.to_str().unwrap();
    let mut listed = Vec::new();
    rust_ASC::cli::run_from_into(["rust_ASC", "listclass", input, "--json"], &mut listed).unwrap();
    let names: Vec<String> = serde_json::from_slice(&listed).unwrap();
    assert!(!names.is_empty());
    let name = &names[0];
    for mode in ["source", "asm"] {
        let output = Command::new(env!("CARGO_BIN_EXE_rust_ASC"))
            .args(["getclass", input, name, "--abc-output", mode, "--json"]).output().unwrap();
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        let rows: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(rows[0]["status"], "ok");
        assert_eq!(rows[0]["backend"], "harmony");
        assert!(!rows[0]["source"].as_str().unwrap().is_empty());
        assert!(rows[0].get("dex").is_none());
    }
    let mut out = Vec::new();
    assert!(rust_ASC::cli::run_from_into(["rust_ASC", "listmethod", input], &mut out).unwrap_err().contains("not supported"));
}

mod zip_fixture;
use zip_fixture::{write_apk, CompressionMethod};

#[test]
fn batch_preserves_successes_and_manifest_resources_and_export_origins() {
    let dir = tempdir().unwrap();
    let hap = dir.path().join("input");
    let names = asc_abc::class_names(CLASS, None).unwrap();
    let manifest = br#"{"module":{"name":"entry","description":"$string:title"}}"#;
    let strings = br#"{"string":[{"name":"title","value":"Test name"}]}"#;
    write_apk(&hap, &[("ets/modules.abc", CLASS), ("ets/other.abc", CLASS), ("module.json", manifest),
        ("resources/base/element/string.json", strings)], CompressionMethod::Deflated);
    let input = hap.to_str().unwrap();
    let mut out = Vec::new();
    let failure = rust_ASC::cli::run_from_into(["rust_ASC", "getclass", input, &names[0], "missing.Record", "--json"], &mut out);
    assert!(failure.is_err());
    let rows: Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(rows[0]["status"], "ok");
    assert_eq!(rows[0]["member"], "ets/modules.abc");
    assert_eq!(rows[1]["status"], "not_found");
    out.clear();
    rust_ASC::cli::run_from_into(["rust_ASC", "getmanifest", input, "--json"], &mut out).unwrap();
    let manifest: Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(manifest["module.json"]["module"]["description"], "Test name");
    let destination = dir.path().join("export");
    rust_ASC::cli::run_from_into(["rust_ASC", "export", input, destination.to_str().unwrap(), "--abc-output", "asm"], &mut out).unwrap();
    assert!(destination.join("member-00000000").is_dir());
    assert!(destination.join("member-00000001").is_dir());
    let classes = dir.path().join("classes.txt");
    std::fs::write(&classes, format!("# selected\n{}\n", names[0])).unwrap();
    out.clear();
    rust_ASC::cli::run_from_into(["rust_ASC", "getclass", input, "--classes-file", classes.to_str().unwrap(), "--json"], &mut out).unwrap();
    assert_eq!(serde_json::from_slice::<Value>(&out).unwrap().as_array().unwrap().len(), 1);
}
