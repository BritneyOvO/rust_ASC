use std::collections::{BTreeMap, BTreeSet};

use super::flow::{EdgeKind, Flow, Target};
use super::index::{IdKind, Index, MethodInfo, Reference, Resolved};
use super::{find_refs, RefQuery, References};
use crate::file::{Class, Code, File, Method, Module};

const CLASS: &[u8] = include_bytes!("../../tests/fixtures/class.abc");
const HX360E: &[u8] = include_bytes!("../../tests/fixtures/hx360e.abc");

#[test]
fn integers_are_exact_signed_and_arbitrary_precision() {
    for (input, output) in [
        ("+00042", "42"),
        ("-0x2a", "-42"),
        ("0xffffffffffffffff", "18446744073709551615"),
        ("0x10000000000000000n", "18446744073709551616"),
        ("-0", "0"),
    ] {
        assert_eq!(super::constants::integer(input).as_deref(), Some(output));
    }
    for input in ["", "0x", "3.14", "--2", "12 junk"] {
        assert!(super::constants::integer(input).is_none());
    }
}

#[test]
fn real_string_and_numeric_hits_include_method_and_pc() {
    let mut refs = References::new(CLASS).unwrap();
    let strings = refs.find("class.abc", RefQuery::String("123")).unwrap();
    assert!(
        strings.iter().any(|s| s.contains("pc=")
            && s.contains("opcode=lda.str")
            && s.contains("matched=(123)")),
        "{strings:?}"
    );
    let integers = refs.find("class.abc", RefQuery::Integer("0xa")).unwrap();
    assert!(
        integers
            .iter()
            .any(|s| s.contains("pc=") && s.contains("kind=integer") && s.contains("matched=(10)")),
        "{integers:?}"
    );
    assert!(refs
        .find("class.abc", RefQuery::String("does-not-exist"))
        .unwrap()
        .is_empty());
    assert!(refs.find("class.abc", RefQuery::Code("[")).is_err());
}

#[test]
fn real_class_calls_are_distinct_from_function_definitions() {
    let methods = find_refs(
        CLASS,
        "class.abc",
        RefQuery::Method {
            name: Some(".*"),
            class_name: Some("Test"),
        },
    )
    .unwrap();
    assert!(
        methods.iter().any(|s| s.contains("kind=construct")
            && s.contains("resolution=resolved")
            && s.contains("pc=")),
        "{methods:?}"
    );
    assert!(
        methods.iter().any(|s| s.contains("kind=definition")),
        "{methods:?}"
    );
    let external = find_refs(
        CLASS,
        "class.abc",
        RefQuery::Method {
            name: Some("log"),
            class_name: Some("console"),
        },
    )
    .unwrap();
    assert!(
        external
            .iter()
            .any(|s| s.contains("kind=call") && s.contains("matched=(console->log)")),
        "{external:?}"
    );
}

#[test]
fn modern_literal_buffers_and_cross_record_calls_are_searchable() {
    let mut refs = References::new(HX360E).unwrap();
    let strings = refs
        .find("hx360e.abc", RefQuery::String("app.prev.log"))
        .unwrap();
    assert!(
        strings
            .iter()
            .any(|s| s.contains("AppLog") && s.contains("pc=")),
        "{strings:?}"
    );
    let calls = refs
        .find(
            "hx360e.abc",
            RefQuery::Method {
                name: Some("i"),
                class_name: Some("AppLog"),
            },
        )
        .unwrap();
    assert!(
        calls.iter().any(|s| s.contains("GamePage")
            && s.contains("kind=call")
            && s.contains("resolution=resolved")),
        "{calls:?}"
    );
    let nested = refs
        .find(
            "hx360e.abc",
            RefQuery::Method {
                name: Some("append"),
                class_name: Some("AppLog"),
            },
        )
        .unwrap();
    assert!(
        nested.iter().any(|s| s.contains("->i@")
            && s.contains("kind=call")
            && s.contains("resolution=resolved")),
        "{nested:?}"
    );
}

fn blank_data() -> Vec<u8> {
    let mut data = vec![0; 64];
    data[..8].copy_from_slice(b"PANDA\0\0\0");
    data[16..20].copy_from_slice(&64u32.to_le_bytes());
    data
}

fn fake_index<'a>(data: &'a [u8], bodies: &[(u32, &[u8])]) -> Index<'a> {
    let mut index = Index {
        file: File::parse(data).unwrap(),
        records: vec![Class {
            name: "LTest;".into(),
            methods: Vec::new(),
            module: Module::default(),
        }],
        methods: BTreeMap::new(),
        strings: BTreeMap::new(),
        literals: BTreeMap::new(),
        classes: BTreeMap::new(),
        owners: BTreeMap::new(),
        constants: Vec::new(),
        fields: Vec::new(),
    };
    for (offset, bytes) in bodies {
        let instructions = crate::decode(bytes)
            .unwrap()
            .into_iter()
            .map(|ins| {
                let references = ins
                    .operands
                    .iter()
                    .enumerate()
                    .filter_map(|(operand, op)| {
                        if let crate::Operand::Id(id) = op {
                            Some(Reference {
                                operand,
                                kind: if ins.opcode.flags & 2 != 0 {
                                    IdKind::Method
                                } else {
                                    IdKind::String
                                },
                                offset: *id,
                            })
                        } else {
                            None
                        }
                    })
                    .collect();
                Resolved { ins, references }
            })
            .collect();
        index.methods.insert(
            *offset,
            MethodInfo {
                record: 0,
                method: Method {
                    offset: *offset,
                    name: format!("f{offset}"),
                    code: Some(Code {
                        registers: 4,
                        arguments: 3,
                        bytes: &[],
                        tries: Vec::new(),
                    }),
                },
                instructions,
            },
        );
    }
    index
}

#[test]
fn branches_preserve_both_possible_callees_and_overwrites_kill_old_values() {
    let data = blank_data();
    // Branch on acc; each arm assigns a different function, then joins at callarg0.
    let branch = [
        79, 11, 51, 0, 2, 0, 0, 97, 0, 77, 9, 51, 0, 3, 0, 0, 97, 0, 96, 0, 41, 0, 101,
    ];
    let targets = [101];
    let index = fake_index(&data, &[(1, &branch), (2, &targets), (3, &targets)]);
    let edges = Flow::new(&index).run().unwrap();
    let calls: BTreeSet<_> = edges
        .iter()
        .filter(|e| e.caller == 1 && e.kind == EdgeKind::Call)
        .map(|e| e.target.clone())
        .collect();
    assert_eq!(
        calls,
        BTreeSet::from([Target::Method(2), Target::Method(3)])
    );

    let overwrite = [
        51, 0, 2, 0, 0, 97, 0, 51, 0, 3, 0, 0, 97, 0, 96, 0, 41, 0, 101,
    ];
    let index = fake_index(&data, &[(1, &overwrite), (2, &targets), (3, &targets)]);
    let edges = Flow::new(&index).run().unwrap();
    let calls: BTreeSet<_> = edges
        .iter()
        .filter(|e| e.caller == 1 && e.kind == EdgeKind::Call)
        .map(|e| e.target.clone())
        .collect();
    assert_eq!(calls, BTreeSet::from([Target::Method(3)]));
}

#[test]
fn closure_scopes_do_not_mix_same_numbered_slots() {
    let data = blank_data();
    // Each parent owns env.slot[0], and creates a closure that calls that slot.
    let parent_a = [9, 1, 51, 0, 4, 0, 0, 61, 0, 51, 0, 2, 0, 0, 41, 0, 101];
    let parent_b = [9, 1, 51, 0, 5, 0, 0, 61, 0, 51, 0, 3, 0, 0, 41, 0, 101];
    let child = [60, 0, 41, 0, 101];
    let target = [101];
    let index = fake_index(
        &data,
        &[
            (1, &parent_a),
            (6, &parent_b),
            (2, &child),
            (3, &child),
            (4, &target),
            (5, &target),
        ],
    );
    let edges = Flow::new(&index).run().unwrap();
    for (caller, target) in [(2, 4), (3, 5)] {
        let calls: BTreeSet<_> = edges
            .iter()
            .filter(|e| {
                e.caller == caller
                    && e.kind == EdgeKind::Call
                    && matches!(e.target, Target::Method(_))
            })
            .map(|e| e.target.clone())
            .collect();
        assert_eq!(calls, BTreeSet::from([Target::Method(target)]));
    }
}

#[test]
fn module_resolution_preserves_normalized_paths_and_rejects_ambiguity() {
    let mut index = Index::new(HX360E).unwrap();
    let source = index
        .records
        .iter()
        .position(|r| r.name.contains("GamePage"))
        .unwrap();
    let target = index
        .records
        .iter()
        .position(|r| r.name.contains("AppLog"))
        .unwrap();
    let path = "@normalized:N&&&entry/src/main/ets/common/AppLog&";
    assert_eq!(Flow::new(&index).record(source, path), Some(target));
    assert_eq!(
        Flow::new(&index).record(source, "../common/AppLog.ets"),
        Some(target)
    );
    assert_eq!(
        Flow::new(&index).record(source, "@normalized:N&&&missing/AppLog&"),
        None
    );
    let duplicate = Class {
        name: index.records[target].name.clone(),
        methods: Vec::new(),
        module: Module::default(),
    };
    index.records.push(duplicate);
    assert_eq!(Flow::new(&index).record(source, path), None);
}

#[test]
fn structured_references_preserve_literal_text_without_parsing_display_lines() {
    let mut refs = References::new(CLASS).unwrap();
    let hits = refs
        .find_records("input | special.abc", RefQuery::Code("lda.str"))
        .unwrap();
    assert!(!hits.is_empty());
    assert!(hits.iter().all(|hit| hit.member == "input | special.abc"));
    assert!(hits
        .iter()
        .all(|hit| hit.pc.is_some() && hit.opcode.is_some()));
    let text = refs
        .find("input | special.abc", RefQuery::Code("lda.str"))
        .unwrap();
    assert_eq!(
        text,
        hits.iter().map(ToString::to_string).collect::<Vec<_>>()
    );
}
