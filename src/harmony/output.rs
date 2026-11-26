use std::fs;
use std::io::Write;

use serde::Serialize;

#[derive(Serialize)]
pub(super) struct ClassOutput {
    pub class: String,
    pub status: &'static str,
    pub backend: &'static str,
    pub language: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub member: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

pub(super) fn emit(
    text: &str,
    output: Option<&String>,
    also_stdout: bool,
    out: &mut dyn Write,
) -> Result<(), String> {
    if let Some(path) = output {
        fs::write(path, text).map_err(|error| format!("cannot write {path}: {error}"))?;
    }
    if output.is_none() || also_stdout {
        out.write_all(text.as_bytes())
            .map_err(|error| format!("cannot write output: {error}"))?;
    }
    Ok(())
}

pub(super) fn json(value: &impl Serialize) -> Result<String, String> {
    serde_json::to_string_pretty(value)
        .map(|text| text + "\n")
        .map_err(|error| format!("cannot serialize JSON: {error}"))
}

#[derive(Serialize)]
pub(super) struct ReferenceOutput<'a> {
    backend: &'static str,
    member: &'a str,
    caller: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pc: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    opcode: Option<&'a str>,
    kind: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    resolution: Option<&'a str>,
    matched: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    literal: Option<(u32, usize)>,
    #[serde(skip_serializing_if = "Option::is_none")]
    metadata: Option<u32>,
}

impl<'a> From<&'a asc_abc_dec::ReferenceHit> for ReferenceOutput<'a> {
    fn from(hit: &'a asc_abc_dec::ReferenceHit) -> Self {
        Self {
            backend: "harmony",
            member: &hit.member,
            caller: &hit.caller,
            pc: hit.pc,
            opcode: hit.opcode,
            kind: hit.kind,
            resolution: hit.resolution,
            matched: &hit.matched,
            literal: hit.literal,
            metadata: hit.metadata,
        }
    }
}
