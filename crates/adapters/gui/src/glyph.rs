//! Every character the interface paints that ASCII does not carry.
//!
//! The default fonts of the shell cover a part of Unicode only, and a
//! character outside that part paints as a hollow box: the mark vanishes and
//! takes its meaning with it, silently and everywhere at once. Naming every
//! such character here keeps the inventory countable, and
//! `every_glyph_the_interface_paints_is_covered_by_the_fonts` proves the
//! fonts draw all of it. A non-ASCII literal written anywhere else in the
//! interface escapes that proof.

/// Names one glyph each, and gathers the whole inventory for the proof, so
/// a glyph joins the interface and the proof in a single line.
macro_rules! glyphs {
    ($($(#[$description:meta])* $name:ident = $glyph:literal;)*) => {
        $($(#[$description])* pub(crate) const $name: &str = $glyph;)*

        #[cfg(test)]
        const ALL: &[&str] = &[$($name),*];
    };
}

glyphs! {
    /// A whole repository.
    PROJECT = "◎";
    /// One package of a repository.
    PACKAGE = "▣";
    /// One module of a package. Denser than the package around it, so the
    /// two read apart at the size a box gives them.
    MODULE = "■";
    /// One function.
    FUNCTION = "ƒ";
    /// One type.
    TYPE = "T";

    /// A dependency read from the boundary that depends to the boundary it
    /// depends on. A row about one boundary carries it in front of a
    /// partner that boundary reaches; a heading sets it between the two
    /// ends of one dependency.
    OUTWARD = "⏵";
    /// The same dependency read the other way: the partner reaches the
    /// boundary the panel is about.
    INWARD = "⏴";

    /// What a renamed element becomes: it stands between the name the
    /// sources carry and the name the plan gives it.
    BECOMES = "»";

    /// The arrow keys, as a hint names them.
    KEY_UP = "⏶";
    KEY_DOWN = "⏷";

    /// One step down the containment above a search result.
    CONTAINER_STEP = " › ";
    /// What stands between two parts of one hint.
    HINT_STEP = "·";
    /// Text the interface cuts short, and a command that asks for more
    /// before it acts.
    ELLIPSIS = "…";
}

#[cfg(test)]
mod tests {
    use eframe::egui;

    use super::ALL;

    /// A character the fonts lack resolves to the face that holds the
    /// replacement glyph - the hollow box a reader sees - so asking the font
    /// stack itself catches the defect before any of it reaches a screen.
    /// The question errs toward the box: a character the stack answers for
    /// is one it draws, while a character it denies may still be drawable.
    /// A rejected candidate therefore costs a second choice and never a
    /// hollow box on a screen.
    ///
    /// The interface paints all of these in proportional text; the one
    /// monospace run it shows is an element id, which its own constructor
    /// keeps to the characters of a source path.
    #[test]
    fn every_glyph_the_interface_paints_is_covered_by_the_fonts() {
        let context = egui::Context::default();
        // The fonts exist only after a pass, and a pass needs no window:
        // laying out text is CPU work alone.
        let _ = context.run_ui(egui::RawInput::default(), |_| {});
        let font = egui::FontId::proportional(14.0);

        let hollow: Vec<String> = context.fonts_mut(|fonts| {
            ALL.iter()
                .flat_map(|glyph| glyph.chars())
                .filter(|character| !fonts.has_glyph(&font, *character))
                .map(|character| format!("{character} (U+{:04X})", character as u32))
                .collect()
        });

        assert!(
            hollow.is_empty(),
            "the interface paints characters the fonts draw as a hollow box: {hollow:?}"
        );
    }
}
