//! What each module re-exports.
//!
//! An `export ... from` puts a name on the surface of the file that writes it
//! while the declaration itself lives elsewhere. Barrel files are built this
//! way: an `index.ts` re-exports the files of its directory, and consumers
//! import the names from the barrel. Name resolution therefore cannot stop at
//! the module that offers the name - the declaration sits one or more
//! re-export hops further.

use std::collections::BTreeMap;

use cutaway_inspection::ports::source_tree::SourcePath;

use crate::imports::Import;

/// Where one re-exported name leads: the specifier that carries it, and the
/// name the target offers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Forwarded {
    pub specifier: String,
    pub name: String,
}

/// The re-exports of every module, keyed by the file that writes them. When
/// one name is re-exported twice in a file, the first statement in source
/// order answers.
#[derive(Debug, Default)]
pub struct ReexportTable {
    named: BTreeMap<(SourcePath, String), Forwarded>,
    wildcards: BTreeMap<SourcePath, Vec<String>>,
}

impl ReexportTable {
    pub fn add(&mut self, path: &SourcePath, imports: &[Import]) {
        for import in imports {
            for reexport in &import.reexports {
                self.named
                    .entry((path.clone(), reexport.name.clone()))
                    .or_insert_with(|| Forwarded {
                        specifier: import.specifier.clone(),
                        name: reexport.from.clone(),
                    });
            }
            if import.wildcard_reexport {
                self.wildcards
                    .entry(path.clone())
                    .or_default()
                    .push(import.specifier.clone());
            }
        }
    }

    /// Where the module at `path` forwards `name`, written from that module's
    /// own perspective.
    pub fn forwarded(&self, path: &SourcePath, name: &str) -> Option<&Forwarded> {
        self.named.get(&(path.clone(), name.to_owned()))
    }

    /// The specifiers of the `export * from ...` statements of the module at
    /// `path`, in source order. Each of them may hold any name the module
    /// does not declare itself.
    pub fn wildcards(&self, path: &SourcePath) -> &[String] {
        self.wildcards.get(path).map_or(&[], Vec::as_slice)
    }
}
