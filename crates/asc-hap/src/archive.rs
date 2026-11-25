use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::collections::VecDeque;
use std::time::SystemTime;

use memmap2::Mmap;
use asc_abc_dec::{Decompiler, OutputMode};
use crate::hap::{self, ZipEntry};
use crate::{Error, HapResources, Result};

const MAX_CACHED_ARCHIVES: usize = 8;
const MAX_CACHED_BYTES: u64 = 512 * 1024 * 1024;

struct CachedArchive {
    path: PathBuf,
    length: u64,
    modified: Option<SystemTime>,
    archive: Arc<Archive>,
}
static CACHE: OnceLock<Mutex<VecDeque<CachedArchive>>> = OnceLock::new();

/// Owns the mapped archive; borrowed ABC parsers are confined to each operation.
pub struct Archive {
    map: Mmap,
    entries: Vec<ZipEntry>,
    resources: OnceLock<HapResources>,
}

pub struct BatchOutput {
    pub classes: Vec<Option<ClassOutput>>,
    pub errors: Vec<String>,
}

pub struct ClassOutput {
    pub class: String,
    pub member: String,
    pub member_index: u32,
    pub output: Result<String>,
}

impl Archive {
    pub fn open(path: &Path) -> Result<Arc<Self>> {
        let path = path.canonicalize()?;
        let file = std::fs::File::open(&path)?;
        let metadata = file.metadata()?;
        let length = metadata.len();
        let modified = metadata.modified().ok();
        let cache = CACHE.get_or_init(|| Mutex::new(VecDeque::new()));
        if let Ok(mut cache) = cache.lock() {
            if let Some(position) = cache.iter().position(|item| item.path == path && item.length == length && item.modified == modified) {
                let hit = cache.remove(position).expect("position came from cache");
                let archive = Arc::clone(&hit.archive);
                cache.push_front(hit);
                return Ok(archive);
            }
            cache.retain(|item| item.path != path);
        }
        // SAFETY: the file is mapped read-only; callers must keep the input unmodified during analysis.
        let map = unsafe { Mmap::map(&file) }?;
        let entries = hap::zip_entries(&map)?;
        let archive = Arc::new(Self { map, entries, resources: OnceLock::new() });
        if length <= MAX_CACHED_BYTES {
            if let Ok(mut cache) = cache.lock() {
                while !cache.is_empty() && (cache.len() >= MAX_CACHED_ARCHIVES || cache.iter().map(|item| item.length).sum::<u64>() + length > MAX_CACHED_BYTES) {
                    cache.pop_back();
                }
                cache.push_front(CachedArchive { path, length, modified, archive: Arc::clone(&archive) });
            }
        }
        Ok(archive)
    }

    pub fn is_hap(&self) -> bool {
        self.entries.iter().any(|entry| hap::is_hap_marker(&entry.name))
    }

    pub fn resources(&self) -> &HapResources {
        self.resources.get_or_init(|| HapResources::index(&self.map, &self.entries))
    }

    pub fn manifest_files(&self) -> Result<Vec<(String, String)>> {
        hap::package_texts(&self.map, &self.entries, self.resources())
    }

    /// Inflate each selected member once and keep its parser borrowed within the callback.
    pub fn for_each_abc(&self, mut visit: impl FnMut(&str, u32, &[u8]) -> Result<()>) -> Result<()> {
        for entry in hap::ranked_code(&self.entries) {
            let bytes = hap::inflate(&self.map, entry)?;
            visit(&entry.name, entry.index, &bytes)?;
        }
        Ok(())
    }

    pub fn list_classes(&self, prefix: Option<&str>) -> Result<Vec<String>> {
        let mut classes = Vec::new();
        self.for_each_abc(|_, _, bytes| {
            classes.extend(asc_abc::class_names(bytes, prefix)?);
            Ok(())
        })?;
        Ok(classes)
    }

    pub fn get_classes(&self, queries: &[String], mode: OutputMode, threads: usize) -> Result<BatchOutput> {
        if threads == 0 { return Err(Error::new("threads must be >= 1")); }
        let mut results: Vec<Option<ClassOutput>> = (0..queries.len()).map(|_| None).collect();
        let mut errors = Vec::new();
        for entry in hap::ranked_code(&self.entries) {
            if results.iter().all(Option::is_some) { break; }
            let bytes = match hap::inflate(&self.map, entry) {
                Ok(bytes) => bytes,
                Err(error) => { errors.push(format!("{}: {error}", entry.name)); continue; }
            };
            let metadata = match asc_abc::Abc::parse(&bytes) {
                Ok(metadata) => metadata,
                Err(error) => { errors.push(format!("{}: {error}", entry.name)); continue; }
            };
            let selected: Vec<_> = queries.iter().enumerate().filter_map(|(index, query)| {
                if results[index].is_some() { return None; }
                match metadata.class_offset(query) {
                    Ok(Some(_)) => Some((index, query)),
                    Ok(None) => None,
                    Err(error) => { errors.push(format!("{}: {error}", entry.name)); None }
                }
            }).collect();
            if selected.is_empty() { continue; }
            let parser = Decompiler::new(&bytes)?;
            let rendered = hap::run_indexed(selected.len(), threads, |index| {
                let (_, query) = selected[index];
                let output = parser.render_class(query, mode).map(|text| {
                    if mode == OutputMode::Assembly { text } else { self.resources().annotate(&text).into_owned() }
                }).map_err(Error::from);
                Ok(ClassOutput { class: query.clone(), member: entry.name.clone(), member_index: entry.index, output })
            })?;
            for ((index, _), output) in selected.into_iter().zip(rendered) { results[index] = Some(output); }
        }
        Ok(BatchOutput { classes: results, errors })
    }

    /// Filter before rendering, reuse each parser, and bound retained source by worker count.
    pub fn visit_selected_classes(&self, prefix: Option<&str>, queries: Option<&[String]>, mode: OutputMode, threads: usize, mut visit: impl FnMut(ClassOutput) -> Result<()>) -> Result<()> {
        if threads == 0 { return Err(Error::new("threads must be >= 1")); }
        for entry in hap::ranked_code(&self.entries) {
            let prepare = || -> Result<_> {
                let bytes = hap::inflate(&self.map, entry)?;
                let metadata = asc_abc::Abc::parse(&bytes)?;
                let mut names = metadata.class_names(prefix)?;
                if let Some(queries) = queries {
                    let offsets = queries.iter().map(|query| metadata.class_offset(query)).collect::<std::result::Result<Vec<_>, _>>()?;
                    let offsets: std::collections::BTreeSet<_> = offsets.into_iter().flatten().collect();
                    names.retain(|name| metadata.class_offset(name).ok().flatten().is_some_and(|offset| offsets.contains(&offset)));
                }
                Ok((bytes, names))
            };
            let (bytes, names) = match prepare() {
                Ok(prepared) => prepared,
                Err(error) => {
                    visit(ClassOutput { class: String::new(), member: entry.name.clone(), member_index: entry.index, output: Err(error) })?;
                    continue;
                }
            };
            if names.is_empty() { continue; }
            let decompiler = Decompiler::new(&bytes)?;
            for names in names.chunks(threads) {
                let outputs = hap::run_indexed(names.len(), threads, |index| {
                    let class = &names[index];
                    let output = decompiler.render_class(class, mode).map(|text| {
                        if mode == OutputMode::Assembly { text } else { self.resources().annotate(&text).into_owned() }
                    }).map_err(Error::from);
                    Ok(ClassOutput { class: class.clone(), member: entry.name.clone(), member_index: entry.index, output })
                })?;
                for output in outputs { visit(output)?; }
            }
        }
        Ok(())
    }
}
