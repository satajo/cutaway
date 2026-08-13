//! go.mod reading: which modules exist and where they live. Requirement
//! directives stay unread - the sources alone witness what a module depends
//! on.

use cutaway_inspection::ports::source_analyzer::{AnalysisGap, GapReason};
use cutaway_inspection::ports::source_tree::SourceFile;

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

/// Finds every `go.mod` and reads its `module` directive, together with the
/// gap left by every manifest that could not be read. A go.mod without a
/// module directive declares nothing the go tool can build, so it is a broken
/// manifest rather than an absent module: the module it would have named is
/// missing from the picture, and the gap says so. Every other manifest of the
/// tree is read regardless.
pub fn discover_modules(files: &[SourceFile]) -> (Vec<DiscoveredModule>, Vec<AnalysisGap>) {
    let mut modules = Vec::new();
    let mut gaps = Vec::new();
    for file in files {
        let path = file.path.as_str();
        if !(path == "go.mod" || path.ends_with("/go.mod")) {
            continue;
        }
        let Ok(text) = std::str::from_utf8(&file.contents) else {
            gaps.push(AnalysisGap {
                path: file.path.clone(),
                reason: GapReason::NonUtf8Text,
            });
            continue;
        };
        let Some(module_path) = module_directive(text) else {
            gaps.push(AnalysisGap {
                path: file.path.clone(),
                reason: GapReason::ManifestUnreadable {
                    detail: "it declares no module path".to_owned(),
                },
            });
            continue;
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
    (modules, gaps)
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
