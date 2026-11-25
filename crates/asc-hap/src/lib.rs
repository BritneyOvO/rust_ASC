//! HarmonyOS HAP access.
//!
//! `.abc` members are discovered from the ZIP central directory and inflated
//! only when a command needs them.

#[macro_use]
mod error;
mod hap;
mod resources;
mod archive;
pub use archive::{Archive, ClassOutput};

pub use error::{Error, Result};
pub use hap::{
    dump_abc, dump_abc_with_mode, dump_hap, dump_hap_with_mode, find_refs, get_class_output,
    get_class_source, is_hap, list_classes, load_resources, module_json, DumpStats, Options, Query,
};
pub use resources::HapResources;

/// Map a record to an escaped package path without traversing outside the output directory.
pub fn output_path(root: &std::path::Path, record: &str, extension: &str) -> std::path::PathBuf {
    hap::class_output_path_with_extension(root, record, extension)
}
