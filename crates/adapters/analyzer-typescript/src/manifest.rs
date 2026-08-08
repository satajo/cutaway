//! package.json reading: which packages exist, where they live, and which
//! file each of them offers as its entry.
//!
//! Dependency fields stay unread - the sources alone witness what a package
//! depends on.

use cutaway_inspection::ports::source_analyzer::SourceAnalysisError;
use cutaway_inspection::ports::source_tree::SourceFile;
use serde_json::Value;

use crate::modules::{is_vendored, join};

/// One package a package.json declares.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveredPackage {
    pub name: String,
    /// Directory of the manifest, `""` for the repository root, no trailing
    /// slash otherwise.
    pub dir: String,
    /// Repository-relative paths the package may entered through, most
    /// authoritative first. Only the source tree tells which of them exists,
    /// so the module catalog picks the winner.
    pub entry_candidates: Vec<String>,
}

/// Finds every `package.json` that names a package. A manifest without a
/// `name` is legal - workspace roots are written that way - and declares no
/// package.
pub fn discover_packages(
    files: &[SourceFile],
) -> Result<Vec<DiscoveredPackage>, SourceAnalysisError> {
    let mut packages = Vec::new();
    for file in files {
        let path = file.path.as_str();
        if !(path == "package.json" || path.ends_with("/package.json")) {
            continue;
        }
        if is_vendored(path) {
            continue;
        }
        let text =
            std::str::from_utf8(&file.contents).map_err(|_| SourceAnalysisError::NonUtf8Text {
                path: file.path.clone(),
            })?;
        let manifest: Value =
            serde_json::from_str(text).map_err(|error| SourceAnalysisError::Unparseable {
                path: file.path.clone(),
                reason: error.to_string(),
            })?;
        let Some(name) = manifest.get("name").and_then(Value::as_str) else {
            continue;
        };
        let dir = path
            .strip_suffix("package.json")
            .map_or("", |dir| dir.trim_end_matches('/'))
            .to_owned();
        packages.push(DiscoveredPackage {
            name: name.to_owned(),
            entry_candidates: entry_candidates(&manifest, &dir),
            dir,
        });
    }
    packages.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(packages)
}

/// Everything the package may be entered through, in decreasing authority:
/// what the manifest declares, then the conventional index file every
/// resolver falls back to.
fn entry_candidates(manifest: &Value, dir: &str) -> Vec<String> {
    let mut candidates: Vec<String> = declared_entries(manifest)
        .into_iter()
        .map(|entry| join(dir, entry.trim_start_matches("./")))
        .collect();
    candidates.push(join(dir, "index"));
    candidates.push(join(dir, "src/index"));
    candidates
}

/// The entries the manifest declares. `exports` is the modern declaration of
/// what a package offers; `module`, `main`, and `types` are the older fields,
/// each naming the same entry for a different consumer.
fn declared_entries(manifest: &Value) -> Vec<String> {
    let mut entries = Vec::new();
    if let Some(exported) = manifest.get("exports").and_then(exported_entry) {
        entries.push(exported);
    }
    entries.extend(
        ["module", "main", "types"]
            .into_iter()
            .filter_map(|field| manifest.get(field).and_then(Value::as_str))
            .map(str::to_owned),
    );
    entries
}

/// The file the `exports` field offers for the package itself. The field is
/// either one path or a map of subpaths, of which only `"."` is the package;
/// a conditional entry names one file per consumer, and every condition
/// names the same entry of the package.
fn exported_entry(exports: &Value) -> Option<String> {
    if let Some(path) = exports.as_str() {
        return Some(path.to_owned());
    }
    let root = exports.get(".")?;
    if let Some(path) = root.as_str() {
        return Some(path.to_owned());
    }
    ["import", "require", "default", "node"]
        .into_iter()
        .find_map(|condition| root.get(condition).and_then(Value::as_str))
        .map(str::to_owned)
}
