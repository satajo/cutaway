//! go.mod reading: which modules exist and where they live. Requirement
//! directives stay unread - the sources alone witness what a module depends
//! on.

use cutaway_inspection::ports::source_analyzer::SourceAnalysisError;
use cutaway_inspection::ports::source_tree::{SourceFile, SourcePath};

/// One module a go.mod declares.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveredModule {
    /// The module path the `module` directive gives, the name under which
    /// every import of this module's code starts.
    pub path: String,
    /// Directory of the manifest, `""` for the repository root, no trailing
    /// slash otherwise.
    pub dir: String,
}

impl DiscoveredModule {
    /// The manifest that declared this module.
    #[must_use]
    pub fn manifest(&self) -> SourcePath {
        let path = if self.dir.is_empty() {
            "go.mod".to_owned()
        } else {
            format!("{}/go.mod", self.dir)
        };
        SourcePath::new(path).expect("a manifest path is never empty")
    }
}

/// Finds every `go.mod` and reads its `module` directive. A go.mod without
/// one declares nothing the go tool can build, so it is a broken manifest
/// rather than an absent module.
pub fn discover_modules(
    files: &[SourceFile],
) -> Result<Vec<DiscoveredModule>, SourceAnalysisError> {
    let mut modules = Vec::new();
    for file in files {
        let path = file.path.as_str();
        if !(path == "go.mod" || path.ends_with("/go.mod")) {
            continue;
        }
        let text =
            std::str::from_utf8(&file.contents).map_err(|_| SourceAnalysisError::NonUtf8Text {
                path: file.path.clone(),
            })?;
        let Some(module_path) = module_directive(text) else {
            return Err(SourceAnalysisError::Unparseable {
                path: file.path.clone(),
                reason: "the manifest declares no module path".to_owned(),
            });
        };
        modules.push(DiscoveredModule {
            path: module_path,
            dir: path
                .strip_suffix("go.mod")
                .map_or("", |d| d.trim_end_matches('/'))
                .to_owned(),
        });
    }
    modules.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(modules)
}

/// Reads the module path out of a go.mod. The directive stands alone on its
/// line, and go.mod quotes a path only when it holds characters the plain
/// form cannot carry, so both forms appear in the wild.
fn module_directive(text: &str) -> Option<String> {
    for line in text.lines() {
        let line = line.split("//").next().unwrap_or(line).trim();
        let Some(rest) = line.strip_prefix("module") else {
            continue;
        };
        // `modulepath` is one word, not the directive followed by its
        // argument.
        if !rest.starts_with(char::is_whitespace) {
            continue;
        }
        let path = rest.trim();
        let unquoted = unquoted(path, '"')
            .or_else(|| unquoted(path, '`'))
            .unwrap_or(path);
        if unquoted.is_empty() {
            continue;
        }
        return Some(unquoted.to_owned());
    }
    None
}

fn unquoted(path: &str, quote: char) -> Option<&str> {
    path.strip_prefix(quote).and_then(|p| p.strip_suffix(quote))
}
