use std::collections::BTreeSet;
use std::fmt;

/// Identifies an [`Element`] within one [`crate::ArchitectureGraph`].
///
/// Ids are opaque: the producer of a graph chooses the scheme, consumers only
/// compare them. Comparing graphs of the same project across versions relies
/// on the producer deriving ids deterministically from the sources.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ElementId(String);

impl ElementId {
    pub fn new(id: impl Into<String>) -> Result<Self, InvalidElementId> {
        let id = id.into();
        if id.is_empty() {
            return Err(InvalidElementId::Empty);
        }
        Ok(Self(id))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ElementId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum InvalidElementId {
    #[error("an element id must not be empty")]
    Empty,
}

/// The human-facing name of an element, as it appears in the sources.
/// Unlike [`ElementId`], names carry no uniqueness guarantee.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct ElementName(String);

impl ElementName {
    pub fn new(name: impl Into<String>) -> Result<Self, InvalidElementName> {
        let name = name.into();
        if name.is_empty() {
            return Err(InvalidElementName::Empty);
        }
        Ok(Self(name))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ElementName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum InvalidElementName {
    #[error("an element name must not be empty")]
    Empty,
}

/// The coarse classification of an element, and the currency every reader
/// of the model speaks: the lens vocabulary, the chips, the comparison.
///
/// Two readings of one source tree meet in this set. The filesystem reads
/// directories and files - the substrate every project has, whatever wrote
/// it. A language reads projects, packages, modules, types and functions
/// out of that substrate. One element carries at most one kind of each
/// reading, because a module that is exactly one file is one boundary a
/// reader addresses, not two.
///
/// Containment follows the substrate: directories hold what lies in them,
/// files hold the declarations written in them, and a language element fused
/// with a substrate node holds whatever that node holds. Every level is a
/// boundary in the sense of the boundary lens: relations crossing it mean
/// more than relations inside it.
///
/// The declaration order is the order of the containment hierarchy, so every
/// ordering of kinds reads from the coarsest level down.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ElementKind {
    /// One inspected source tree: a repository, a monorepo root.
    Project,
    /// A unit of distribution and dependency declaration: a Rust crate, a Go
    /// module, a Java artifact, an npm package.
    Package,
    /// One directory of the source tree. It groups what lies in it and
    /// nothing more, unless a language reads a boundary of its own out of
    /// the same directory - a Go package, a Rust module spanning `foo.rs`
    /// and `foo/` - in which case one element carries both readings.
    Directory,
    /// A grouping of code within a package: a source file, a namespace.
    Module,
    /// One file of the source tree, holding the declarations written in it.
    /// A file a language reads as a boundary of its own carries that reading
    /// beside this one; a file no language read stands as itself.
    File,
    /// An executable unit: a function, a method, a procedure.
    Function,
    /// A data or interface definition: a struct, an enum, a trait.
    Type,
}

/// The language's reading of an element: what the code declares it to be.
///
/// The declaration order follows [`ElementKind`], so the two sets order
/// alike.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum SemanticKind {
    Project,
    Package,
    Module,
    Function,
    Type,
}

/// The filesystem's reading of an element: what a listing of the tree shows,
/// whatever language wrote the contents.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum SubstrateKind {
    Directory,
    File,
}

impl From<SemanticKind> for ElementKind {
    fn from(kind: SemanticKind) -> Self {
        match kind {
            SemanticKind::Project => Self::Project,
            SemanticKind::Package => Self::Package,
            SemanticKind::Module => Self::Module,
            SemanticKind::Function => Self::Function,
            SemanticKind::Type => Self::Type,
        }
    }
}

impl From<SubstrateKind> for ElementKind {
    fn from(kind: SubstrateKind) -> Self {
        match kind {
            SubstrateKind::Directory => Self::Directory,
            SubstrateKind::File => Self::File,
        }
    }
}

/// What a language read at one place in the sources, under the name the
/// language calls it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Semantic {
    pub kind: SemanticKind,
    pub name: ElementName,
}

/// What the filesystem holds at one place in the sources, under the name the
/// tree shows. The name is the tree's, not the language's: the crate
/// `cutaway-architecture` lives in the directory `architecture`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Substrate {
    pub kind: SubstrateKind,
    pub name: ElementName,
}

/// What an element holds, condensed to one number, so two versions can ask
/// "did it change inside" without carrying the contents.
///
/// Equal fingerprints read as unchanged. The digest is deterministic across
/// platforms and runs, because graphs of different versions must align. The
/// honest limit: a hash collision reads as unchanged, which is the accepted
/// cost of not carrying contents.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Fingerprint(u64);

impl Fingerprint {
    /// Digests the contents with FNV-1a 64, written out here because the
    /// domain depends on no hash crate.
    #[must_use]
    pub fn of(contents: &[u8]) -> Self {
        const OFFSET_BASIS: u64 = 14_695_981_039_346_656_037;
        const PRIME: u64 = 1_099_511_628_211;
        let mut digest = OFFSET_BASIS;
        for byte in contents {
            digest ^= u64::from(*byte);
            digest = digest.wrapping_mul(PRIME);
        }
        Self(digest)
    }
}

impl fmt::Display for Fingerprint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:016x}", self.0)
    }
}

/// One node of the architecture graph: one boundary a reader addresses,
/// under up to two readings of the same thing.
///
/// The two aspects are private because a node without any aspect would stand
/// nameless and kindless, and the constructors are the only way in: every one
/// of them takes an aspect, so the invariant holds by construction. Where
/// both aspects stand they describe one boundary twice - the module `element`
/// and the file `element.rs` - each under its own name, because a language's
/// name for a thing and the tree's name for it need not agree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Element {
    pub id: ElementId,
    semantic: Option<Semantic>,
    substrate: Option<Substrate>,
    /// What the element holds, condensed for change detection between
    /// versions; None where the producer made no statement about the
    /// contents - which is different from stating they are empty.
    pub fingerprint: Option<Fingerprint>,
}

impl Element {
    /// What a language read, standing on nothing the tree names as a whole:
    /// a declaration inside a file, a package spread over many directories.
    #[must_use]
    pub fn semantic(id: ElementId, kind: SemanticKind, name: ElementName) -> Self {
        Self {
            id,
            semantic: Some(Semantic { kind, name }),
            substrate: None,
            fingerprint: None,
        }
    }

    /// A place in the tree no language read a boundary out of: a plain file,
    /// a directory that only groups.
    #[must_use]
    pub fn substrate(
        id: ElementId,
        kind: SubstrateKind,
        name: ElementName,
        fingerprint: Option<Fingerprint>,
    ) -> Self {
        Self {
            id,
            semantic: None,
            substrate: Some(Substrate { kind, name }),
            fingerprint,
        }
    }

    /// One boundary both readings agree on, because the language element
    /// interprets that whole place in the tree and nothing else does.
    #[must_use]
    pub fn fused(
        id: ElementId,
        semantic: Semantic,
        substrate: Substrate,
        fingerprint: Option<Fingerprint>,
    ) -> Self {
        Self {
            id,
            semantic: Some(semantic),
            substrate: Some(substrate),
            fingerprint,
        }
    }

    /// The element one vocabulary kind alone describes. Each kind belongs to
    /// exactly one reading, so the kind decides the aspect. This is the way
    /// in for a caller holding a kind and nothing more; a producer that
    /// knows which reading it speaks names the aspect instead.
    ///
    /// Nothing in the workspace produces elements this way today: a plan
    /// states what a language reads and says so in a [`SemanticKind`], and
    /// the analyzers and the inspection core name their aspects. It stands
    /// for the tests that build a graph from kinds alone.
    #[must_use]
    pub fn of_kind(id: ElementId, kind: ElementKind, name: ElementName) -> Self {
        match kind {
            ElementKind::Project => Self::semantic(id, SemanticKind::Project, name),
            ElementKind::Package => Self::semantic(id, SemanticKind::Package, name),
            ElementKind::Module => Self::semantic(id, SemanticKind::Module, name),
            ElementKind::Function => Self::semantic(id, SemanticKind::Function, name),
            ElementKind::Type => Self::semantic(id, SemanticKind::Type, name),
            ElementKind::Directory => Self::substrate(id, SubstrateKind::Directory, name, None),
            ElementKind::File => Self::substrate(id, SubstrateKind::File, name, None),
        }
    }

    /// Fuses a place in the tree onto what a language read there, so an
    /// element that arrived as an interpretation alone learns where it
    /// stands and what it holds.
    ///
    /// The fingerprint arrives with the place: the tree is the authority on
    /// contents, so whatever the element carried before is replaced - by
    /// None too, where the place says nothing about its contents.
    #[must_use]
    pub fn with_substrate(
        mut self,
        substrate: Substrate,
        fingerprint: Option<Fingerprint>,
    ) -> Self {
        self.substrate = Some(substrate);
        self.fingerprint = fingerprint;
        self
    }

    /// The union of the readings this node carries, for a caller asking
    /// which kinds an architecture holds at all - the chips of a vocabulary
    /// control. Whether a vocabulary draws the node, and as which reading,
    /// is [`Element::speaks_as`]'s answer alone.
    ///
    /// The iterator carries `#[must_use]` of its own, so no attribute stands
    /// here: a discarded call already fails the lints.
    pub fn kinds(&self) -> impl Iterator<Item = ElementKind> {
        [
            self.semantic.as_ref().map(|aspect| aspect.kind.into()),
            self.substrate.as_ref().map(|aspect| aspect.kind.into()),
        ]
        .into_iter()
        .flatten()
    }

    /// The kind the node reads as when nothing narrows the question: what a
    /// language read, else what the tree shows.
    #[must_use]
    pub fn primary_kind(&self) -> ElementKind {
        self.semantic.as_ref().map_or_else(
            || self.expect_substrate().kind.into(),
            |aspect| aspect.kind.into(),
        )
    }

    /// The name the node reads as when nothing narrows the question, chosen
    /// with [`Element::primary_kind`].
    #[must_use]
    pub fn primary_name(&self) -> &ElementName {
        self.semantic
            .as_ref()
            .map_or_else(|| &self.expect_substrate().name, |aspect| &aspect.name)
    }

    /// The aspect a vocabulary lets speak: what a language read when that
    /// kind is rendered, else what the tree shows when that kind is. None
    /// means the vocabulary renders neither reading, so the node is
    /// transparent and its contents hoist past it.
    #[must_use]
    pub fn speaks_as(
        &self,
        vocabulary: &BTreeSet<ElementKind>,
    ) -> Option<(ElementKind, &ElementName)> {
        let semantic = self
            .semantic
            .as_ref()
            .map(|aspect| (aspect.kind.into(), &aspect.name));
        let substrate = self
            .substrate
            .as_ref()
            .map(|aspect| (aspect.kind.into(), &aspect.name));
        [semantic, substrate]
            .into_iter()
            .flatten()
            .find(|(kind, _)| vocabulary.contains(kind))
    }

    /// What a language read here, if one did.
    #[must_use]
    pub fn semantic_aspect(&self) -> Option<&Semantic> {
        self.semantic.as_ref()
    }

    /// Where in the tree this node stands, if it stands anywhere as a whole.
    #[must_use]
    pub fn substrate_aspect(&self) -> Option<&Substrate> {
        self.substrate.as_ref()
    }

    fn expect_substrate(&self) -> &Substrate {
        self.substrate
            .as_ref()
            .expect("every element carries at least one aspect")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_element_id_must_not_be_empty() {
        assert_eq!(ElementId::new(""), Err(InvalidElementId::Empty));
    }

    #[test]
    fn an_element_name_must_not_be_empty() {
        assert_eq!(ElementName::new(""), Err(InvalidElementName::Empty));
    }

    #[test]
    fn the_same_contents_always_give_the_same_fingerprint() {
        assert_eq!(
            Fingerprint::of(b"fn main() {}"),
            Fingerprint::of(b"fn main() {}")
        );
    }

    #[test]
    fn different_contents_give_different_fingerprints() {
        assert_ne!(
            Fingerprint::of(b"fn main() {}"),
            Fingerprint::of(b"fn main() { run(); }")
        );
    }

    fn id(text: &str) -> ElementId {
        ElementId::new(text).unwrap()
    }

    fn name(text: &str) -> ElementName {
        ElementName::new(text).unwrap()
    }

    /// The module `element`, read out of the file `element.rs`.
    fn fused_element() -> Element {
        Element::fused(
            id("src/element.rs"),
            Semantic {
                kind: SemanticKind::Module,
                name: name("element"),
            },
            Substrate {
                kind: SubstrateKind::File,
                name: name("element.rs"),
            },
            Some(Fingerprint::of(b"contents")),
        )
    }

    #[test]
    fn an_element_carries_a_name_per_aspect() {
        let element = fused_element();

        assert_eq!(
            element.semantic_aspect().map(|aspect| aspect.name.as_str()),
            Some("element")
        );
        assert_eq!(
            element
                .substrate_aspect()
                .map(|aspect| aspect.name.as_str()),
            Some("element.rs")
        );
    }

    #[test]
    fn an_element_of_one_aspect_carries_that_aspect_alone() {
        let file = Element::substrate(
            id("README.md"),
            SubstrateKind::File,
            name("README.md"),
            None,
        );

        assert_eq!(file.semantic_aspect(), None);
        assert_eq!(file.primary_kind(), ElementKind::File);
        assert_eq!(file.primary_name().as_str(), "README.md");
    }

    #[test]
    fn an_element_answers_to_the_kind_of_every_aspect_it_carries() {
        assert_eq!(
            fused_element().kinds().collect::<Vec<_>>(),
            vec![ElementKind::Module, ElementKind::File]
        );
        assert_eq!(
            Element::semantic(id("a"), SemanticKind::Package, name("a"))
                .kinds()
                .collect::<Vec<_>>(),
            vec![ElementKind::Package]
        );
    }

    #[test]
    fn an_element_reads_as_what_a_language_made_of_it_before_what_the_tree_shows() {
        let element = fused_element();

        assert_eq!(element.primary_kind(), ElementKind::Module);
        assert_eq!(element.primary_name().as_str(), "element");
    }

    #[test]
    fn an_element_speaks_as_the_aspect_the_vocabulary_renders() {
        let element = fused_element();

        assert_eq!(
            element.speaks_as(&BTreeSet::from([ElementKind::Module, ElementKind::File])),
            Some((ElementKind::Module, &name("element"))),
            "with both kinds rendered the language's reading speaks"
        );
        assert_eq!(
            element.speaks_as(&BTreeSet::from([ElementKind::File])),
            Some((ElementKind::File, &name("element.rs")))
        );
    }

    #[test]
    fn an_element_no_rendered_kind_reaches_speaks_not_at_all() {
        assert_eq!(
            fused_element().speaks_as(&BTreeSet::from([ElementKind::Package])),
            None
        );
    }

    #[test]
    fn fusing_a_place_in_the_tree_onto_an_element_leaves_its_own_reading_standing() {
        let fingerprint = Fingerprint::of(b"contents");
        let module = Element::semantic(id("src/element.rs"), SemanticKind::Module, name("element"))
            .with_substrate(
                Substrate {
                    kind: SubstrateKind::File,
                    name: name("element.rs"),
                },
                Some(fingerprint),
            );

        assert_eq!(module, fused_element());
        assert_eq!(module.fingerprint, Some(fingerprint));
    }

    #[test]
    fn the_place_a_node_fuses_with_states_its_contents_over_anything_it_carried() {
        let file = Substrate {
            kind: SubstrateKind::File,
            name: name("element.rs"),
        };
        let carrying = Element::substrate(
            id("src/element.rs"),
            SubstrateKind::File,
            name("element.rs"),
            Some(Fingerprint::of(b"stale contents")),
        );

        assert_eq!(
            carrying
                .clone()
                .with_substrate(file.clone(), Some(Fingerprint::of(b"contents")))
                .fingerprint,
            Some(Fingerprint::of(b"contents"))
        );
        assert_eq!(
            carrying.with_substrate(file, None).fingerprint,
            None,
            "a place that says nothing about its contents leaves the node saying nothing"
        );
    }

    #[test]
    fn a_kind_alone_decides_which_reading_carries_it() {
        assert_eq!(
            Element::of_kind(id("a"), ElementKind::Module, name("a")).semantic_aspect(),
            Some(&Semantic {
                kind: SemanticKind::Module,
                name: name("a")
            })
        );
        assert_eq!(
            Element::of_kind(id("a"), ElementKind::Directory, name("a")).substrate_aspect(),
            Some(&Substrate {
                kind: SubstrateKind::Directory,
                name: name("a")
            })
        );
    }

    #[test]
    fn empty_contents_have_a_valid_and_stable_fingerprint() {
        assert_eq!(Fingerprint::of(b""), Fingerprint::of(b""));
        // The FNV-1a offset basis, so the digest is pinned across releases
        // and platforms alike.
        assert_eq!(Fingerprint::of(b"").to_string(), "cbf29ce484222325");
    }
}
