//! Cargo manifest reading: which packages exist and where they live.
//! Dependency declarations stay unread - the sources alone witness what a
//! package depends on.

use cutaway_inspection::ports::source_analyzer::{AnalysisGap, GapReason};
use cutaway_inspection::ports::source_tree::SourceFile;

/// One package a Cargo.toml declares.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveredPackage {
    pub name: String,
    /// Directory of the manifest, `""` for the repository root, no trailing
    /// slash otherwise.
    pub dir: String,
}

/// Finds every `Cargo.toml` that declares a `[package]`, together with the
/// gap left by every manifest that could not be read. A workspace-only root
/// manifest declares no package and contributes nothing. A manifest no TOML
/// reader can make sense of takes only its own package out of the picture;
/// every other manifest of the tree is read regardless.
pub fn discover_packages(files: &[SourceFile]) -> (Vec<DiscoveredPackage>, Vec<AnalysisGap>) {
    let mut packages = Vec::new();
    let mut gaps = Vec::new();
    for file in files {
        let path = file.path.as_str();
        if !(path == "Cargo.toml" || path.ends_with("/Cargo.toml")) {
            continue;
        }
        let Ok(text) = std::str::from_utf8(&file.contents) else {
            gaps.push(AnalysisGap {
                path: file.path.clone(),
                reason: GapReason::NonUtf8Text,
            });
            continue;
        };
        let table: toml::Table = match toml::from_str(text) {
            Ok(table) => table,
            Err(error) => {
                gaps.push(AnalysisGap {
                    path: file.path.clone(),
                    reason: GapReason::ManifestUnreadable {
                        detail: error.to_string(),
                    },
                });
                continue;
            }
        };
        let Some(name) = table
            .get("package")
            .and_then(|package| package.get("name"))
            .and_then(toml::Value::as_str)
        else {
            continue;
        };
        // A `[package]` section with an empty name declares a package no
        // reader can address and no id can name, so the section is broken
        // rather than absent - the same reading a go.mod without a module
        // path gets.
        if name.is_empty() {
            gaps.push(AnalysisGap {
                path: file.path.clone(),
                reason: GapReason::ManifestUnreadable {
                    detail: "it declares no package name".to_owned(),
                },
            });
            continue;
        }
        packages.push(DiscoveredPackage {
            name: name.to_owned(),
            dir: path
                .strip_suffix("Cargo.toml")
                .map_or("", |d| d.trim_end_matches('/'))
                .to_owned(),
        });
    }
    packages.sort_by(|a, b| a.name.cmp(&b.name));
    (packages, gaps)
}
