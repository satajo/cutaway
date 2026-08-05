//! Cargo manifest reading: which packages exist and where they live.
//! Dependency declarations stay unread - the sources alone witness what a
//! package depends on.

use cutaway_inspection::ports::source_analyzer::SourceAnalysisError;
use cutaway_inspection::ports::source_tree::SourceFile;

/// One package a Cargo.toml declares.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveredPackage {
    pub name: String,
    /// Directory of the manifest, `""` for the repository root, no trailing
    /// slash otherwise.
    pub dir: String,
}

/// Finds every `Cargo.toml` that declares a `[package]`. A workspace-only
/// root manifest declares no package and contributes nothing.
pub fn discover_packages(
    files: &[SourceFile],
) -> Result<Vec<DiscoveredPackage>, SourceAnalysisError> {
    let mut packages = Vec::new();
    for file in files {
        let path = file.path.as_str();
        if !(path == "Cargo.toml" || path.ends_with("/Cargo.toml")) {
            continue;
        }
        let text =
            std::str::from_utf8(&file.contents).map_err(|_| SourceAnalysisError::NonUtf8Text {
                path: file.path.clone(),
            })?;
        let table: toml::Table =
            toml::from_str(text).map_err(|error| SourceAnalysisError::Unparseable {
                path: file.path.clone(),
                reason: error.to_string(),
            })?;
        let Some(name) = table
            .get("package")
            .and_then(|package| package.get("name"))
            .and_then(toml::Value::as_str)
        else {
            continue;
        };
        packages.push(DiscoveredPackage {
            name: name.to_owned(),
            dir: path
                .strip_suffix("Cargo.toml")
                .map_or("", |d| d.trim_end_matches('/'))
                .to_owned(),
        });
    }
    packages.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(packages)
}
