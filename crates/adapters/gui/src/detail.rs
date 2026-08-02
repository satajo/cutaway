//! How the reader sets the detail of the whole picture, and what the toolbar
//! says about the cut that follows from it.
//!
//! Three levels with names of their own read as their names, so the picture's
//! detail is three labeled stops rather than a position on a bare slider:
//! the reader sees where the picture stands and what the next step holds
//! without moving anything. Each stop also answers to its number, so a hand
//! on the keyboard changes the whole picture without reaching for the mouse.
//!
//! The line beside the stops says how far the reader's own decisions depart
//! from that whole. It is a pure function of the cut; the toolbar only paints
//! what it answers.

use cutaway_lenses::{Cut, Detail};
use eframe::egui;

/// The keys that pick a detail, coarsest first, so the number of a stop is
/// its position among them.
const KEYS: [egui::Key; 3] = [egui::Key::Num1, egui::Key::Num2, egui::Key::Num3];

pub(crate) fn name(detail: Detail) -> &'static str {
    match detail {
        Detail::Packages => "Packages",
        Detail::Modules => "Modules",
        Detail::Items => "Items",
    }
}

/// The detail of the whole picture, as one stop per level. Answers with the
/// level the reader clicked, and with None every other frame.
pub(crate) fn stops(ui: &mut egui::Ui, current: Detail) -> Option<Detail> {
    let mut chosen = None;
    ui.scope(|ui| {
        // The stops sit against each other, so the three of them read as one
        // control with a position rather than as three buttons.
        ui.spacing_mut().item_spacing.x = 1.0;
        for (position, detail) in Detail::ALL.into_iter().enumerate() {
            let clicked = ui
                .selectable_label(detail == current, name(detail))
                .on_hover_text(format!("{} ({})", name(detail), position + 1))
                .clicked();
            if clicked {
                chosen = Some(detail);
            }
        }
    });
    chosen
}

/// The detail this frame's keys ask for, and None while they ask for none.
///
/// A digit typed into a note or a search is a digit rather than a command, so
/// the shell asks this only while no text field holds the keyboard and no
/// palette is open. The palette takes its own keys before this runs and never
/// asks for a digit, so the two never contend.
pub(crate) fn requested(ctx: &egui::Context) -> Option<Detail> {
    if ctx.text_edit_focused() {
        return None;
    }
    ctx.input_mut(|input| {
        Detail::ALL
            .into_iter()
            .zip(KEYS)
            .find(|(_, key)| input.consume_key(egui::Modifiers::NONE, *key))
            .map(|(detail, _)| detail)
    })
}

/// How far the boundaries the reader opened and closed depart from the detail
/// of the whole, in one line. None while the picture follows one detail
/// throughout: a uniform cut is what the stops already say.
pub(crate) fn departures(cut: &Cut) -> Option<String> {
    let opened = cut
        .overrides
        .values()
        .filter(|within| **within > cut.detail)
        .count();
    let closed = cut
        .overrides
        .values()
        .filter(|within| **within < cut.detail)
        .count();
    match (opened, closed) {
        (0, 0) => None,
        (opened, 0) => Some(format!("{opened} {} opened", boundaries(opened))),
        (0, closed) => Some(format!("{closed} {} closed", boundaries(closed))),
        (opened, closed) => Some(format!(
            "{opened} {} opened, {closed} closed",
            boundaries(opened)
        )),
    }
}

fn boundaries(count: usize) -> &'static str {
    if count == 1 { "boundary" } else { "boundaries" }
}

#[cfg(test)]
mod tests {
    use cutaway_architecture::ElementId;

    use super::*;

    fn cut<const N: usize>(detail: Detail, overrides: [(&str, Detail); N]) -> Cut {
        Cut {
            detail,
            overrides: overrides
                .into_iter()
                .map(|(boundary, within)| (ElementId::new(boundary).unwrap(), within))
                .collect(),
        }
    }

    #[test]
    fn a_picture_following_one_detail_throughout_says_nothing_further() {
        assert_eq!(departures(&Cut::uniform(Detail::Modules)), None);
    }

    #[test]
    fn a_boundary_opened_past_the_whole_counts_as_opened() {
        assert_eq!(
            departures(&cut(Detail::Packages, [("package:a", Detail::Modules)])),
            Some("1 boundary opened".to_owned())
        );
    }

    #[test]
    fn boundaries_closed_before_the_whole_count_as_closed() {
        assert_eq!(
            departures(&cut(
                Detail::Items,
                [
                    ("package:a", Detail::Modules),
                    ("package:b", Detail::Packages)
                ]
            )),
            Some("2 boundaries closed".to_owned())
        );
    }

    #[test]
    fn a_cut_departing_both_ways_counts_both() {
        assert_eq!(
            departures(&cut(
                Detail::Modules,
                [
                    ("package:a", Detail::Items),
                    ("package:b", Detail::Packages)
                ]
            )),
            Some("1 boundary opened, 1 closed".to_owned())
        );
    }

    #[test]
    fn a_boundary_kept_at_the_detail_of_the_whole_departs_from_nothing() {
        assert_eq!(
            departures(&cut(Detail::Modules, [("package:a", Detail::Modules)])),
            None
        );
    }
}
