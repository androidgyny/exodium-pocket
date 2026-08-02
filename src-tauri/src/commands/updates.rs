use std::collections::HashMap;

use serde::{Deserialize, Serialize};

// ── Manifest schema (v2) ─────────────────────────────────────────────────────

/// A downloadable content pack. Two source kinds:
///   - HTTP tar.gz (url + sha256): externally hosted release asset
///   - Torrent-sourced ZIP (torrent_file_path): a file inside the collection's
///     existing torrent. librqbit handles piece-level integrity, so sha256 is
///     redundant and left empty.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ContentPackInfo {
    pub display_name: String,
    pub description: String,
    pub url: String,
    pub sha256: String,
    pub size_bytes: u64,
    pub version: u32,
    /// Relative path under data_dir where the pack extracts to.
    pub install_path: String,
    /// Pack IDs this pack replaces (e.g. media supersedes posters).
    #[serde(default)]
    pub supersedes: Vec<String>,
    /// Oldest installed version still usable with this app build. An installed
    /// pack BELOW it is deleted on startup; anything at or above it stays and
    /// is merely offered as an update. Without this, every content change
    /// silently wiped the user's art and left them to notice - the poster pack
    /// grew by 34 covers and would have cost everyone their 376 MB.
    #[serde(default)]
    pub min_compatible_version: u32,
    /// If set, install via torrent selective-download instead of HTTP. Value
    /// is the file path inside the collection's torrent
    /// (e.g. "Content/XODOSMetadata.zip"). The extractor expects a .zip.
    #[serde(default)]
    pub torrent_file_path: Option<String>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct CollectionManifest {
    pub torrent_infohash: String,
    pub game_count: u32,
    /// Available content packs keyed by pack ID (e.g. "posters", "media").
    #[serde(default)]
    pub content_packs: HashMap<String, ContentPackInfo>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct Manifest {
    pub schema_version: u32,
    pub generated_at: String,
    pub collections: HashMap<String, CollectionManifest>,
}

// ── Response types ────────────────────────────────────────────────────────────

// ── Manifest loading ──────────────────────────────────────────────────────────

/// Load the manifest from the best available source.
/// Dev mode reads from the project root. Production reads the bundled copy
/// from resource_dir (shipped via bundle.resources). HTTP fetch from a remote
/// manifest_url is a future improvement (v0.2+).
pub(crate) fn load_manifest() -> Result<Manifest, String> {
    // Dev: read from the project root next to Cargo.toml
    let dev_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(|p| p.join("manifest.json"))
        .unwrap_or_default();
    if dev_path.exists() {
        let content = std::fs::read_to_string(&dev_path)
            .map_err(|e| format!("cannot read manifest.json: {}", e))?;
        return serde_json::from_str(&content)
            .map_err(|e| format!("cannot parse manifest.json: {}", e));
    }

    // Production: read the bundled copy from resource_dir.
    if let Some(res_dir) = super::setup::RESOURCE_DIR.get() {
        let bundled = res_dir.join("manifest.json");
        if bundled.exists() {
            let content = std::fs::read_to_string(&bundled)
                .map_err(|e| format!("cannot read bundled manifest.json: {}", e))?;
            return serde_json::from_str(&content)
                .map_err(|e| format!("cannot parse bundled manifest.json: {}", e));
        }
    }

    // TODO (v0.2): HTTP fetch from manifest_url as final fallback.
    Err("manifest.json not found (dev path or resource_dir)".to_string())
}

#[cfg(test)]
mod manifest_load_tests {
    #[test]
    fn manifest_parses_with_packs() {
        let m = super::load_manifest().expect("load_manifest failed");
        let ex = m.collections.get("eXoDOS").expect("no eXoDOS collection");
        assert!(!ex.content_packs.is_empty(), "eXoDOS has no content packs");
        println!("packs: {:?}", ex.content_packs.keys().collect::<Vec<_>>());
    }
}
