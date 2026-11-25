use std::fs;
use std::io::Write;
use std::path::Path;

use asc_abc_dec::{Decompiler, OutputMode};
use asc_hap::HapResources;
use asc_abc_dec::{RefQuery as Query, References};
use clap::ArgMatches;

use crate::input::ArtifactKind;
use super::output::{self, ClassOutput};

pub(crate) fn dispatch(
    command: &str,
    args: &ArgMatches,
    out: &mut dyn Write,
) -> Option<Result<(), String>> {
    let path = args.get_one::<String>("apk_path")?;
    let kind = match ArtifactKind::detect(Path::new(path)) {
        Ok(ArtifactKind::Android) => return None,
        Ok(kind) => kind,
        Err(error) => return Some(Err(error)),
    };
    Some(execute(command, args, Path::new(path), kind, out))
}

fn execute(command: &str, args: &ArgMatches, path: &Path, kind: ArtifactKind, out: &mut dyn Write) -> Result<(), String> {
    let threads = args.try_get_one::<usize>("thread").ok().flatten().copied()
        .unwrap_or_else(|| std::thread::available_parallelism().map_or(1, usize::from));
    if threads == 0 { return Err("Worker count must be greater than zero".into()); }

    let output = args.try_get_one::<String>("output").ok().flatten();
    let json = args.try_get_one::<bool>("json").ok().flatten().copied().unwrap_or(false);
    let bytes = if kind == ArtifactKind::Abc {
        Some(fs::read(path).map_err(|error| format!("cannot read {}: {error}", path.display()))?)
    } else { None };
    let resources = args.try_get_one::<String>("resources").ok().flatten();
    if kind == ArtifactKind::Hap && resources.is_some() {
        return Err("HAP uses its own resources; --resources is only for bare ABC".into());
    }
    let table = resources.map(|p| asc_hap::load_resources(Path::new(p)))
        .transpose().map_err(|e| e.to_string())?.unwrap_or_default();
    match command {
        "getclass" => {
            let targets = class_queries(args)?;
            let mode = output_mode(args);
            let language = if mode == OutputMode::Source { "javascript" } else { "ark-assembly" };
            let mut archive_errors = Vec::new();
            let results = if let Some(bytes) = &bytes {
                let metadata = asc_abc::Abc::parse(bytes).map_err(|e| e.to_string())?;
                let decompiler = Decompiler::new(bytes).map_err(|e| e.to_string())?;
                targets.iter().map(|class| {
                    let result = if metadata.class_offset(class).map_err(|e| e.to_string())?.is_none() {
                        None
                    } else {
                        Some(asc_hap::ClassOutput {
                            class: class.clone(), member: path.display().to_string(), member_index: 0,
                            output: decompiler.render_class(class, mode).map(|source| {
                                if mode == OutputMode::Source { table.annotate(&source).into_owned() } else { source }
                            }).map_err(asc_hap::Error::from),
                        })
                    };
                    Ok(result)
                }).collect::<Result<Vec<_>, String>>()?
            } else {
                let batch = asc_hap::Archive::open(path).and_then(|a| a.get_classes(&targets, mode, threads)).map_err(|e| e.to_string())?;
                archive_errors = batch.errors;
                batch.classes
            };
            let mut failed = archive_errors.len();
            let rows: Vec<_> = targets.into_iter().zip(results).map(|(class, result)| {
                match result {
                    Some(item) => match item.output {
                        Ok(source) => ClassOutput { class, status: "ok", backend: "harmony", language,
                            member: Some(item.member), source: Some(source), error: None },
                        Err(error) => { failed += 1; ClassOutput { class, status: "error", backend: "harmony", language,
                            member: Some(item.member), source: None, error: Some(error.to_string()) } }
                    },
                    None => { failed += 1; ClassOutput { class: class.clone(), status: if archive_errors.is_empty() { "not_found" } else { "error" }, backend: "harmony", language,
                        member: None, source: None, error: Some(if archive_errors.is_empty() { format!("record {class} not found") } else { archive_errors.join("\n") }) } },
                }
            }).collect();
            let text = if json { output::json(&rows)? } else {
                rows.iter().filter_map(|item| item.source.as_ref().map(|source| {
                    if rows.len() == 1 { source.clone() + "\n" }
                    else { format!("// Record: {} ({})\n{source}\n", item.class, item.member.as_deref().unwrap_or("")) }
                })).collect::<String>()
            };
            output::emit(&text, output, true, out)?;
            if failed == 0 { Ok(()) } else { Err(format!("{failed} Harmony record(s) or members could not be read: {}", archive_errors.join("; "))) }
        }
        "listclass" => {
            let prefix = args.get_one::<String>("prefix").map(String::as_str);
            let names = if let Some(bytes) = &bytes { asc_abc::class_names(bytes, prefix).map_err(|e| e.to_string())? }
                else { asc_hap::Archive::open(path).and_then(|a| a.list_classes(prefix)).map_err(|e| e.to_string())? };
            let text = if json { output::json(&names)? } else if names.is_empty() { String::new() } else { names.join("\n") + "\n" };
            output::emit(&text, output, false, out)
        }
        "getmanifest" if kind == ArtifactKind::Hap => {
            let files = asc_hap::Archive::open(path).and_then(|a| a.manifest_files()).map_err(|e| e.to_string())?;
            let text = if json {
                let files: Result<std::collections::BTreeMap<_, serde_json::Value>, _> = files.iter()
                    .map(|(name, text)| serde_json::from_str(text).map(|value| (name, value))).collect();
                output::json(&files.map_err(|e| format!("invalid manifest JSON: {e}"))?)?
            } else { files.iter().map(|(name, text)| format!("===== {name} =====\n{text}")).collect() };
            output::emit(&text, output, true, out)
        }
        "export" => export(args, path, bytes.as_deref(), &table, threads, out),
        "findrefs" => {
            let (name, query) = args.subcommand().ok_or("missing reference query")?;
            let value = query.try_get_one::<String>("value").ok().flatten().map(String::as_str);
            let class = query.try_get_one::<String>("class_name").ok().flatten().map(String::as_str);
            if query.try_get_one::<bool>("fuzzy_class").ok().flatten() == Some(&true) { return Err("--fuzzy-class is not supported for ABC or HAP".into()); }
            let pattern = query.try_get_one::<String>("name").ok().flatten().map(String::as_str);
            let q = match name {
                "string" => Query::String(value.ok_or("missing string pattern")?),
                "constant" => Query::Constant(value.ok_or("missing constant pattern")?),
                "integer" => Query::Integer(value.ok_or("missing integer")?),
                "code" => Query::Code(value.ok_or("missing instruction pattern")?),
                "type" => Query::Type(value.ok_or("missing type pattern")?),
                "method" => Query::Method { name: pattern, class_name: class },
                "field" => Query::Field { name: pattern, class_name: class },
                _ => return Err(format!("{name} is not supported for ABC or HAP")),
            };
            let mut hits = Vec::new();
            if let Some(bytes) = &bytes {
                hits = References::new(bytes).and_then(|mut r| r.find_records_resolved(&path.display().to_string(), q, &|raw| table.resolve_literal(raw))).map_err(|e| e.to_string())?;
            } else {
                let archive = asc_hap::Archive::open(path).map_err(|e| e.to_string())?;
                archive.for_each_abc(|member, _, bytes| {
                    hits.extend(References::new(bytes)?.find_records_resolved(member, q, &|raw| archive.resources().resolve_literal(raw))?);
                    Ok(())
                }).map_err(|e| e.to_string())?;
            }
            let text = if json { output::json(&hits.iter().map(output::ReferenceOutput::from).collect::<Vec<_>>())? }
                else {
                    let maximum = query.try_get_one::<usize>("max_match").ok().flatten().copied();
                    let no_match = query.try_get_one::<bool>("no_match").ok().flatten() == Some(&true);
                    hits.iter().map(|hit| {
                        let mut hit = hit.clone();
                        if let Some(maximum) = maximum {
                            let truncated: String = hit.matched.chars().take(maximum).collect();
                            if truncated.len() < hit.matched.len() { hit.matched = truncated + "..."; }
                        }
                        let mut text = hit.to_string();
                        if no_match {
                            text = text.rsplit_once(" | matched=(").map_or_else(|| text.clone(), |(head, _)| head.to_owned());
                        }
                        text + "\n"
                    }).collect()
                };
            output::emit(&text, query.get_one::<String>("output"), true, out)
        }
        _ => Err(format!("{command} is not supported for ABC or HAP")),
    }
}

fn output_mode(args: &ArgMatches) -> OutputMode {
    if args.get_one::<String>("abc_output").is_some_and(|s| s == "asm") { OutputMode::Assembly } else { OutputMode::Source }
}

fn class_queries(args: &ArgMatches) -> Result<Vec<String>, String> {
    let mut targets: Vec<String> = args.get_many::<String>("dalvik_class").into_iter().flatten().cloned().collect();
    if let Some(path) = args.get_one::<String>("classes_file") {
        targets.extend(read_classes(path)?);
    }
    let mut seen = std::collections::HashSet::new();
    targets.retain(|s| seen.insert(s.clone()));
    if targets.is_empty() { Err("At least one record or --classes-file is required".into()) } else { Ok(targets) }
}

fn read_classes(path: &str) -> Result<Vec<String>, String> {
    let text = fs::read_to_string(path).map_err(|e| format!("cannot read classes file {path}: {e}"))?;
    Ok(text.lines().map(|s| s.trim().trim_start_matches('\u{feff}')).filter(|s| !s.is_empty() && !s.starts_with('#')).map(str::to_owned).collect())
}

fn export(args: &ArgMatches, path: &Path, bytes: Option<&[u8]>, resources: &HapResources, threads: usize, out: &mut dyn Write) -> Result<(), String> {
    let destination = Path::new(args.get_one::<String>("output_directory").ok_or("missing output directory")?);
    let prefix = args.get_one::<String>("prefix").map(String::as_str);
    let requested = args.get_one::<String>("classes_file").map(|p| read_classes(p)).transpose()?;
    let mode = output_mode(args);
    let mut remaining: std::collections::BTreeSet<_> = requested.as_ref().into_iter().flatten().cloned().collect();
    let mut used = std::collections::HashSet::new();
    let mut count = 0usize;
    let mut errors = Vec::new();
    let mut visit = |item: asc_hap::ClassOutput| -> asc_hap::Result<()> {
        if !item.class.is_empty() && requested.as_ref().is_some_and(|queries| !queries.iter().any(|query| equivalent_record(query, &item.class))) { return Ok(()); }
        remaining.retain(|query| !equivalent_record(query, &item.class));
        match item.output {
            Ok(text) => {
                let base = if bytes.is_some() { destination.to_path_buf() }
                    else { destination.join(format!("member-{:08x}", item.member_index)) };
                let mut dest = asc_hap::output_path(&base, &item.class, mode.extension());
                let original = dest.clone();
                let mut index = 0;
                while !used.insert(dest.to_string_lossy().to_lowercase()) {
                    index += 1;
                    dest = original.with_file_name(format!("{}~{index}.{}", original.file_stem().unwrap_or_default().to_string_lossy(), mode.extension()));
                }
                if let Some(parent) = dest.parent() { fs::create_dir_all(parent)?; }
                fs::write(&dest, text)?;
                count += 1;
            }
            Err(error) => errors.push(format!("{}: {error}", item.class)),
        }
        Ok(())
    };
    if let Some(bytes) = bytes {
        let names = asc_abc::class_names(bytes, prefix).map_err(|e| e.to_string())?;
        let parser = Decompiler::new(bytes).map_err(|e| e.to_string())?;
        for class in names {
            if requested.as_ref().is_some_and(|queries| !queries.iter().any(|query| equivalent_record(query, &class))) { continue; }
            let output = parser.render_class(&class, mode).map(|s| {
                if mode == OutputMode::Source { resources.annotate(&s).into_owned() } else { s }
            }).map_err(asc_hap::Error::from);
            visit(asc_hap::ClassOutput { class, member: path.display().to_string(), member_index: 0, output }).map_err(|e| e.to_string())?;
        }
    } else {
        let archive = asc_hap::Archive::open(path).map_err(|e| e.to_string())?;
        archive.visit_selected_classes(prefix, requested.as_deref(), mode, threads, visit).map_err(|e| e.to_string())?;
        for (name, text) in archive.manifest_files().map_err(|e| e.to_string())? {
            let filename = Path::new(&name).file_name().ok_or("manifest has no filename")?;
            fs::create_dir_all(destination).map_err(|e| e.to_string())?;
            fs::write(destination.join(filename), text).map_err(|e| e.to_string())?;
        }
    }
    writeln!(out, "exported {count} Harmony records ({} errors) -> {}", errors.len(), destination.display()).map_err(|e| e.to_string())?;
    if !remaining.is_empty() { return Err(format!("records not found: {}", remaining.into_iter().collect::<Vec<_>>().join(", "))); }
    if count == 0 && errors.is_empty() { return Err("No classes matched the export filter.".into()); }
    if errors.is_empty() { Ok(()) } else { Err(errors.join("\n")) }
}

fn equivalent_record(left: &str, right: &str) -> bool {
    let normalize = |s: &str| s.strip_prefix('L').unwrap_or(s).trim_end_matches(';').replace('.', "/");
    left == right || normalize(left) == normalize(right)
}
