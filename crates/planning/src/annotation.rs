use std::fmt;

use cutaway_architecture::{ElementId, Relation};

/// A remark a planner writes about part of the architecture: never empty.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Note(String);

impl Note {
    pub fn new(text: impl Into<String>) -> Result<Self, InvalidNote> {
        let text = text.into();
        if text.trim().is_empty() {
            return Err(InvalidNote::Empty);
        }
        Ok(Self(text))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Note {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum InvalidNote {
    #[error("a note must contain text")]
    Empty,
}

/// What an annotation talks about.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum Subject {
    Element(ElementId),
    Relation(Relation),
}

/// A note attached to an existing part of the architecture, as opposed to a
/// note on a proposed change. At most one annotation per subject.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Annotation {
    pub subject: Subject,
    pub note: Note,
}
