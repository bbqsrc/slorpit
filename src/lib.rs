use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct FileEntry {
    pub path: String,
    pub size: u64,
    pub modified: Option<u64>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ArchiveCatalog {
    pub files: Vec<FileEntry>,
    pub version: String,
}

impl ArchiveCatalog {
    pub fn new() -> Self {
        Self {
            files: Vec::new(),
            version: "1.0".to_string(),
        }
    }
}

pub const CATALOG_KEY: &str = "SlorpitCatalog";
