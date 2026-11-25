use std::ffi::OsString;

use rmcp::model::{Tool, ToolAnnotations};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct GetClass {
    /// Local APK path; with Harmony enabled, getclass/listclass/getmanifest/findrefs/export also accept HAP or ABC.
    #[serde(alias = "input_path")]
    apk_path: String,
    /// Single dotted class name or Dalvik descriptor.
    class_name: Option<String>,
    /// Optional list of dotted class names or Dalvik descriptors for batch decompilation.
    classes: Option<Vec<String>>,
    /// Optional file containing class names, one per line.
    classes_file: Option<String>,
    /// Emit structured JSON with source and member reference metadata.
    #[serde(default)]
    json: bool,
    /// Positive worker count. Defaults to available CPU count.
    thread: Option<usize>,
    /// Optional source output file. Existing files are overwritten. Content is also returned.
    output: Option<String>,
    /// Harmony record output mode: source (default) or asm.
    #[cfg(feature = "harmony")]
    abc_output: Option<String>,
    /// Resource directory or HAP for a bare ABC file.
    #[cfg(feature = "harmony")]
    resources: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ListClass {
    /// Local APK path; with Harmony enabled, getclass/listclass/getmanifest/findrefs/export also accept HAP or ABC.
    #[serde(alias = "input_path")]
    apk_path: String,
    /// Optional case sensitive class or package prefix.
    prefix: Option<String>,
    /// Emit class descriptors as a JSON array.
    #[serde(default)]
    json: bool,
    /// Positive worker count. Defaults to available CPU count.
    thread: Option<usize>,
    /// Optional class list output file. When set, stdout is empty. Existing files are overwritten.
    output: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ListMethod {
    /// Local APK path; with Harmony enabled, getclass/listclass/getmanifest/findrefs/export also accept HAP or ABC.
    #[serde(alias = "input_path")]
    apk_path: String,
    /// Only list methods defined by this class. Accepts a dotted name or Dalvik descriptor.
    class_name: Option<String>,
    /// Optional case insensitive substring filter over method names.
    filter: Option<String>,
    /// Emit method descriptors as a JSON array.
    #[serde(default)]
    json: bool,
    /// Positive worker count. Defaults to available CPU count.
    thread: Option<usize>,
    /// Optional method list output file. When set, stdout is empty. Existing files are overwritten.
    output: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ListString {
    /// Local APK path; with Harmony enabled, getclass/listclass/getmanifest/findrefs/export also accept HAP or ABC.
    #[serde(alias = "input_path")]
    apk_path: String,
    /// Optional case insensitive substring filter over string values.
    filter: Option<String>,
    /// Deduplicate string values across DEX files in first occurrence order.
    #[serde(default)]
    unique: bool,
    /// Emit string values as a JSON array.
    #[serde(default)]
    json: bool,
    /// Positive worker count. Defaults to available CPU count.
    thread: Option<usize>,
    /// Optional string list output file. When set, stdout is empty. Existing files are overwritten.
    output: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct GetManifest {
    /// Local APK path; with Harmony enabled, getclass/listclass/getmanifest/findrefs/export also accept HAP or ABC.
    #[serde(alias = "input_path")]
    apk_path: String,
    /// Optional XML output file. Existing files are overwritten. Content is also returned.
    output: Option<String>,
    /// Emit Harmony manifest files as a JSON object.
    #[cfg(feature = "harmony")]
    #[serde(default)]
    json: bool,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct FindRefs {
    /// Local APK path; with Harmony enabled, getclass/listclass/getmanifest/findrefs/export also accept HAP or ABC.
    #[serde(alias = "input_path")]
    apk_path: String,
    /// Reference target and match mode.
    query: RefQuery,
    /// Emit structured JSON reference records.
    #[serde(default)]
    json: bool,
    /// Positive worker count. Defaults to available CPU count.
    thread: Option<usize>,
    /// Optional reference output file. Existing files are overwritten. Content is also returned.
    output: Option<String>,
    /// Resource directory or HAP for a bare ABC file.
    #[cfg(feature = "harmony")]
    resources: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum RefQuery {
    /// Match Ark literal constants by regular expression (ABC/HAP only).
    #[cfg(feature = "harmony")]
    Constant {
        /// Regular expression over Ark literal values.
        value: String
    },
    /// Match an exact signed decimal or hexadecimal integer (ABC/HAP only).
    #[cfg(feature = "harmony")]
    Integer {
        /// Signed decimal or hexadecimal integer.
        value: String
    },
    /// Match a resolved Ark instruction by regular expression (ABC/HAP only).
    #[cfg(feature = "harmony")]
    Code {
        /// Regular expression over a resolved instruction.
        value: String
    },
    /// Find string constant references.
    String {
        /// Case sensitive string constant substring.
        value: String,
        /// Optional maximum character length for displayed string constants.
        max_match: Option<usize>,
        /// Omit matched string constant values from output.
        #[serde(default)]
        no_match: bool,
    },
    /// Find type references.
    Type {
        /// Case sensitive Dalvik type descriptor substring.
        value: String,
    },
    /// Find direct subclasses.
    Extends {
        /// Dotted class name or Dalvik descriptor.
        value: String,
    },
    /// Find direct interface implementors.
    Impl {
        /// Dotted class name or Dalvik descriptor.
        value: String,
    },
    /// Find method call references.
    Method {
        /// Case sensitive method name substring. Supply name, class_name, or both.
        name: Option<String>,
        /// Dotted name or Dalvik descriptor of the declaring class.
        class_name: Option<String>,
        /// Match class_name as a case sensitive descriptor substring.
        #[serde(default)]
        fuzzy_class: bool,
    },
    /// Find field access references.
    Field {
        /// Case sensitive field name substring. Supply name, class_name, or both.
        name: Option<String>,
        /// Dotted name or Dalvik descriptor of the declaring class.
        class_name: Option<String>,
        /// Match class_name as a case sensitive descriptor substring.
        #[serde(default)]
        fuzzy_class: bool,
    },
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct Export {
    /// Local APK path; with Harmony enabled, getclass/listclass/getmanifest/findrefs/export also accept HAP or ABC.
    #[serde(alias = "input_path")]
    apk_path: String,
    /// Directory where exported Java pseudocode files are written. Content is also returned.
    output: String,
    /// Optional case sensitive class or package prefix.
    prefix: Option<String>,
    /// Optional file containing class names, one per line.
    classes_file: Option<String>,
    /// Positive worker count. Defaults to available CPU count.
    thread: Option<usize>,
    /// Harmony record output mode: source (default) or asm.
    #[cfg(feature = "harmony")]
    abc_output: Option<String>,
    /// Resource directory or HAP for a bare ABC file.
    #[cfg(feature = "harmony")]
    resources: Option<String>,
}

fn tool<I: JsonSchema + 'static>(name: &'static str, description: &'static str) -> Tool {
    Tool::new(name, description, serde_json::Map::new())
        .with_input_schema::<I>()
        .with_annotations(
            ToolAnnotations::new()
                .read_only(false)
                .destructive(true)
                .idempotent(true)
                .open_world(false),
        )
}

pub(super) fn tools() -> Vec<Tool> {
    vec![
        tool::<GetClass>("getclass", "Decompile APK classes or Harmony records to Java, JavaScript, or Ark assembly."),
        tool::<ListClass>(
            "listclass",
            "List classes from an APK, HAP, or bare ABC when the backend is enabled.",
        ),
        tool::<ListMethod>(
            "listmethod",
            "List defined method descriptors from every DEX in an APK.",
        ),
        tool::<ListString>(
            "liststring",
            "List string pool values from every DEX in an APK.",
        ),
        tool::<GetManifest>(
            "getmanifest",
            "Decode an APK manifest or Harmony module configuration.",
        ),
        tool::<FindRefs>(
            "findrefs",
            "Find Android or Ark references, constants, and instructions.",
        ),
        tool::<Export>(
            "export",
            "Export selected Android classes or Harmony records into an output directory.",
        ),
    ]
}

pub(super) struct Query {
    args: Vec<OsString>,
}

impl Query {
    pub(super) fn parse(name: &str, args: Value) -> Result<Self, String> {
        let args = match name {
            "getclass" => getclass(serde_json::from_value(args).map_err(invalid_args)?)?,
            "listclass" => listclass(serde_json::from_value(args).map_err(invalid_args)?),
            "listmethod" => listmethod(serde_json::from_value(args).map_err(invalid_args)?),
            "liststring" => liststring(serde_json::from_value(args).map_err(invalid_args)?),
            "getmanifest" => getmanifest(serde_json::from_value(args).map_err(invalid_args)?),
            "findrefs" => findrefs(serde_json::from_value(args).map_err(invalid_args)?),
            "export" => export(serde_json::from_value(args).map_err(invalid_args)?),
            _ => return Err("unknown tool".to_owned()),
        };
        Ok(Self { args })
    }
}

fn invalid_args(error: serde_json::Error) -> String {
    format!("invalid arguments: {error}")
}

fn push_thread(args: &mut Vec<OsString>, thread: Option<usize>) {
    if let Some(thread) = thread {
        args.push("--thread".into());
        args.push(thread.to_string().into());
    }
}

fn getclass(input: GetClass) -> Result<Vec<OsString>, String> {
    if input.class_name.is_none()
        && input.classes.as_ref().is_none_or(Vec::is_empty)
        && input.classes_file.is_none()
    {
        return Err("invalid arguments: missing field `class_name`".to_owned());
    }
    let mut args = vec!["getclass".into()];
    push_thread(&mut args, input.thread);
    if input.json {
        args.push("--json".into());
    }
    if let Some(classes_file) = input.classes_file {
        args.push("--classes-file".into());
        args.push(classes_file.into());
    }
    #[cfg(feature = "harmony")]
    push_harmony(&mut args, input.abc_output, input.resources);
    push_output(&mut args, input.output);
    args.push(input.apk_path.into());
    if let Some(class_name) = input.class_name {
        args.push(class_name.into());
    }
    if let Some(classes) = input.classes {
        for cls in classes {
            args.push(cls.into());
        }
    }
    Ok(args)
}

fn listclass(input: ListClass) -> Vec<OsString> {
    let mut args = vec!["listclass".into()];
    push_thread(&mut args, input.thread);
    if input.json {
        args.push("--json".into());
    }
    if let Some(prefix) = input.prefix {
        args.push("--prefix".into());
        args.push(prefix.into());
    }
    args.push(input.apk_path.into());
    push_output(&mut args, input.output);
    args
}

fn listmethod(input: ListMethod) -> Vec<OsString> {
    let mut args = vec!["listmethod".into()];
    push_thread(&mut args, input.thread);
    if input.json {
        args.push("--json".into());
    }
    if let Some(class) = input.class_name {
        args.push(format!("--class={class}").into());
    }
    if let Some(filter) = input.filter {
        args.push(format!("--filter={filter}").into());
    }
    push_output(&mut args, input.output);
    args.push("--".into());
    args.push(input.apk_path.into());
    args
}

fn liststring(input: ListString) -> Vec<OsString> {
    let mut args = vec!["liststring".into()];
    push_thread(&mut args, input.thread);
    if input.unique {
        args.push("--unique".into());
    }
    if input.json {
        args.push("--json".into());
    }
    if let Some(filter) = input.filter {
        args.push(format!("--filter={filter}").into());
    }
    push_output(&mut args, input.output);
    args.push("--".into());
    args.push(input.apk_path.into());
    args
}

fn getmanifest(input: GetManifest) -> Vec<OsString> {
    let mut args = vec!["getmanifest".into(), input.apk_path.into()];
    #[cfg(feature = "harmony")]
    if input.json { args.push("--json".into()); }
    push_output(&mut args, input.output);
    args
}

fn findrefs(input: FindRefs) -> Vec<OsString> {
    let mut args = vec!["findrefs".into()];
    push_thread(&mut args, input.thread);
    if input.json {
        args.push("--json".into());
    }
    #[cfg(feature = "harmony")]
    push_harmony(&mut args, None, input.resources);
    args.push(input.apk_path.into());
    match input.query {
        #[cfg(feature = "harmony")]
        RefQuery::Constant { value } => { args.push("constant".into()); args.push(value.into()); }
        #[cfg(feature = "harmony")]
        RefQuery::Integer { value } => { args.push("integer".into()); args.push(value.into()); }
        #[cfg(feature = "harmony")]
        RefQuery::Code { value } => { args.push("code".into()); args.push(value.into()); }
        RefQuery::String {
            value,
            max_match,
            no_match,
        } => {
            args.push("string".into());
            if let Some(max_match) = max_match {
                args.push("--max-match".into());
                args.push(max_match.to_string().into());
            }
            if no_match {
                args.push("--no-match".into());
            }
            args.push(value.into());
        }
        RefQuery::Type { value } => {
            args.push("type".into());
            args.push(value.into());
        }
        RefQuery::Extends { value } => {
            args.push("extends".into());
            args.push(value.into());
        }
        RefQuery::Impl { value } => {
            args.push("impl".into());
            args.push(value.into());
        }
        RefQuery::Method {
            name,
            class_name,
            fuzzy_class,
        } => push_member(&mut args, "method", name, class_name, fuzzy_class),
        RefQuery::Field {
            name,
            class_name,
            fuzzy_class,
        } => push_member(&mut args, "field", name, class_name, fuzzy_class),
    }
    push_output(&mut args, input.output);
    args
}

fn export(input: Export) -> Vec<OsString> {
    let mut args = vec!["export".into()];
    push_thread(&mut args, input.thread);
    if let Some(prefix) = input.prefix {
        args.push("--prefix".into());
        args.push(prefix.into());
    }
    if let Some(classes_file) = input.classes_file {
        args.push("--classes-file".into());
        args.push(classes_file.into());
    }
    #[cfg(feature = "harmony")]
    push_harmony(&mut args, input.abc_output, input.resources);
    args.push(input.apk_path.into());
    args.push(input.output.into());
    args
}

fn push_output(args: &mut Vec<OsString>, output: Option<String>) {
    if let Some(output) = output {
        args.push("-o".into());
        args.push(output.into());
    }
}

fn push_member(
    args: &mut Vec<OsString>,
    kind: &str,
    name: Option<String>,
    class_name: Option<String>,
    fuzzy_class: bool,
) {
    args.push(kind.into());
    if let Some(name) = name {
        args.push(name.into());
    }
    if let Some(class_name) = class_name {
        args.push("--class".into());
        args.push(class_name.into());
    }
    if fuzzy_class {
        args.push("--fuzzy-class".into());
    }
}

pub(super) fn execute(query: Query) -> Result<String, String> {
    let mut cli_args = Vec::with_capacity(query.args.len() + 1);
    cli_args.push(OsString::from("rust_ASC"));
    cli_args.extend(query.args);
    let mut out = Vec::new();
    if let Err(error) = crate::cli::run_from_into(cli_args, &mut out) {
        let partial = String::from_utf8_lossy(&out);
        return Err(if partial.is_empty() { format!("Error: {error}") } else { format!("{partial}\nError: {error}") });
    }
    Ok(String::from_utf8_lossy(&out).into_owned())
}

#[cfg(feature = "harmony")]
fn push_harmony(args: &mut Vec<OsString>, mode: Option<String>, resources: Option<String>) {
    if let Some(mode) = mode { args.push("--abc-output".into()); args.push(mode.into()); }
    if let Some(resources) = resources { args.push("--resources".into()); args.push(resources.into()); }
}
