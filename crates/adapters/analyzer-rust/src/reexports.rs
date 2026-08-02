//! What each module re-exports.
//!
//! A top-level `pub use` makes a name available at the module that writes
//! it while the item itself lives elsewhere. Facades are built this way:
//! `lib.rs` writes `mod element;` beside `pub use element::Element;`, and
//! consumers import `thecrate::Element`. Import resolution therefore cannot
//! stop at the module whose name matches - the item it names sits one or
//! more re-export hops further.

use std::collections::BTreeMap;

use cutaway_inspection::ports::source_tree::SourcePath;

use crate::imports::Import;

/// The re-exports of every module, keyed by the file that writes them.
/// When one name is re-exported twice in a file, the first `pub use` in
/// source order answers.
#[derive(Debug, Default)]
pub struct ReexportTable {
    named: BTreeMap<(SourcePath, String), Vec<String>>,
    wildcards: BTreeMap<SourcePath, Vec<Vec<String>>>,
}

impl ReexportTable {
    pub fn add(&mut self, path: &SourcePath, imports: &[Import]) {
        for import in imports.iter().filter(|import| import.reexport) {
            match &import.binding {
                Some(name) => {
                    self.named
                        .entry((path.clone(), name.clone()))
                        .or_insert_with(|| import.path.clone());
                }
                None => self
                    .wildcards
                    .entry(path.clone())
                    .or_default()
                    .push(import.path.clone()),
            }
        }
    }

    /// The path the module at `path` forwards `name` to, written from that
    /// module's own perspective.
    pub fn forwarded(&self, path: &SourcePath, name: &str) -> Option<&[String]> {
        self.named
            .get(&(path.clone(), name.to_owned()))
            .map(Vec::as_slice)
    }

    /// The targets of the `pub use ...::*` declarations of the module at
    /// `path`, in source order. Each of them may hold any name the module
    /// does not declare itself.
    pub fn wildcards(&self, path: &SourcePath) -> &[Vec<String>] {
        self.wildcards.get(path).map_or(&[], Vec::as_slice)
    }
}
