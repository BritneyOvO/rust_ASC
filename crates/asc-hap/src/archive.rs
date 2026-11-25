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

    pub fn get_classes(&self, queries: &[String], mode: OutputMode) -> Result<Vec<Option<ClassOutput>>> {
        let mut results: Vec<Option<ClassOutput>> = (0..queries.len()).map(|_| None).collect();
        for entry in hap::ranked_code(&self.entries) {
            if results.iter().all(Option::is_some) { break; }
            let bytes = hap::inflate(&self.map, entry)?;
            let metadata = asc_abc::Abc::parse(&bytes)?;
            let mut decompiler = None;
            for (query, result) in queries.iter().zip(&mut results) {
                if result.is_some() || metadata.class_offset(query)?.is_none() { continue; }
                let parser = match &decompiler {
                    Some(parser) => parser,
                    None => decompiler.insert(Decompiler::new(&bytes)?),
                };
                let output = parser.render_class(query, mode).map(|text| {
                    if mode == OutputMode::Assembly { text } else { self.resources().annotate(&text).into_owned() }
                }).map_err(Error::from);
                *result = Some(ClassOutput { class: query.clone(), member: entry.name.clone(), member_index: entry.index, output });
            }
        }
        Ok(results)
    }

    /// Export all matching records, including same-named records from different members.
    pub fn visit_classes(&self, prefix: Option<&str>, mode: OutputMode, mut visit: impl FnMut(ClassOutput) -> Result<()>) -> Result<()> {
        for entry in hap::ranked_code(&self.entries) {
            let bytes = hap::inflate(&self.map, entry)?;
            let names = asc_abc::class_names(&bytes, prefix)?;
            let decompiler = Decompiler::new(&bytes)?;
            for class in names {
                let output = decompiler.render_class(&class, mode).map(|text| {
                    if mode == OutputMode::Assembly { text } else { self.resources().annotate(&text).into_owned() }
                }).map_err(Error::from);
                visit(ClassOutput { class, member: entry.name.clone(), member_index: entry.index, output })?;
            }
        }
        Ok(())
    }
}
