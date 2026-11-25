use std::fs::File;
use std::io::Read;
use std::path::Path;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ArtifactKind {
    Android,
    Abc,
    Hap,
}

impl ArtifactKind {
    pub(crate) fn detect(path: &Path) -> Result<Self, String> {
        let mut file = File::open(path)
            .map_err(|error| format!("cannot open APK/input {}: {error}", path.display()))?;
        let mut magic = [0; 8];
        let count = file.read(&mut magic)
            .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
        if count == 8 && asc_abc::is_abc(&magic) {
            return Ok(Self::Abc);
        }
        if magic.starts_with(b"dex\n") {
            return Ok(Self::Android);
        }
        if asc_hap::Archive::open(path)
            .map_err(|error| format!("invalid ZIP archive {}: {error}", path.display()))?.is_hap() {
            Ok(Self::Hap)
        } else {
            Ok(Self::Android)
        }
    }
}
