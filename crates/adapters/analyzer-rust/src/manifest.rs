//! Cargo manifest reading: which packages exist and what they declare.

use cutaway_inspection::ports::source_analyzer::SourceAnalysisError;
use cutaway_inspection::ports::source_tree::SourceFile;

/// One package a Cargo.toml declares.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveredPackage {
    pub name: String,
    /// Directory of the manifest, `""` for the repository root, no trailing
    /// slash otherwise.
    pub dir: String,
    /// Names of the crates the manifest depends on, across normal, dev, and
    /// build dependencies. Renamed dependencies (`alias = { package = "x" }`)
    /// contribute their real crate name.
    pub dependencies: Vec<String>,
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
            dependencies: declared_dependencies(&table),
        });
    }
    packages.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(packages)
}

fn declared_dependencies(table: &toml::Table) -> Vec<String> {
    let mut dependencies = Vec::new();
    for section in ["dependencies", "dev-dependencies", "build-dependencies"] {
        let Some(entries) = table.get(section).and_then(toml::Value::as_table) else {
            continue;
        };
        for (alias, spec) in entries {
            let name = spec
                .as_table()
                .and_then(|spec| spec.get("package"))
                .and_then(toml::Value::as_str)
                .unwrap_or(alias);
            dependencies.push(name.to_owned());
        }
    }
    dependencies.sort();
    dependencies.dedup();
    dependencies
}
