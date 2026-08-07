//! The boundary canvas: draws the laid-out view and reports what the user
//! clicked. All state changes happen in the app shell; the canvas only
//! renders, steers its camera, and hit-tests.
//!
//! The canvas maps world coordinates to the screen with its own pan/zoom
//! camera instead of egui's Scene. Scene scales already-rasterized glyphs,
//! which pixelates text; the camera instead picks the font size each frame,
//! so labels stay sharp at every magnification.

use std::collections::BTreeMap;

use cutaway_architecture::{ArchitectureGraph, ElementId, Relation};
use eframe::egui::emath::TSTransform;
use eframe::egui::{
    self, Align2, Color32, CornerRadius, CursorIcon, FontId, Pos2, Rect, Sense, Shape, Stroke,
    StrokeKind, Ui, Vec2, pos2, vec2,
};

use crate::bundle::{self, Bundle};
use crate::camera::{self, Camera};
use crate::focus::{Containment, Direction, Focus, Selected, Strength, focus_of};
use crate::glyph;
use crate::label::{Label, Labels, Renames};
use crate::layout::{HEADER, Layout};
use crate::minimap::Minimap;
use crate::routing::{self, Path, Route, Scope};
use crate::summary::{Block, Summary, summarize};

/// How one dependency edge is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EdgeStatus {
    /// Part of the current architecture: monochrome, solid.
    Existing,
    /// Every concrete dependency behind it is planned for removal: red,
    /// dashed.
    Severed,
    /// Some but not all concrete dependencies behind it are planned for
    /// removal: the normal stroke - most of the connection stays - with a
    /// red mark beside the arrowhead saying the plan has touched it.
    PartiallySevered { severed: usize, total: usize },
    /// Planned addition: green, dashed.
    Drawn,
}

#[derive(Debug, Clone)]
pub struct EdgeVisual {
    pub relation: Relation,
    pub status: EdgeStatus,
    pub annotated: bool,
    /// How many concrete dependencies this one edge stands for. A heavier
    /// edge draws thicker, so the picture says where the traffic is.
    pub weight: usize,
}

/// What the plan does to one box the picture draws. A box no [`Content`]
/// names stands as the architecture has it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeStatus {
    /// The element itself, or a boundary that holds it, is planned for
    /// removal: red, exactly as a severed connection reads.
    Removed,
    /// The element exists only in the plan: green, exactly as a drawn
    /// connection reads.
    Added,
    /// The element stays where it is and changes: renamed, split, merged
    /// into another, or reworked. Amber, because neither what is going nor
    /// what is arriving describes it.
    Modified,
}

/// What the user clicked on the canvas.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CanvasAction {
    Node(ElementId),
    /// A double-clicked boundary: the reader asks to see inside it.
    Expand(ElementId),
    Edge(Relation),
    Background,
}

/// One frame's input to the canvas.
pub struct Content<'a> {
    /// Which rebuild produced this picture. Everything the canvas keeps
    /// between frames answers for one generation and is computed again once
    /// a new one arrives.
    pub generation: u64,
    pub view: &'a ArchitectureGraph,
    pub layout: &'a Layout,
    /// The containment of the view, resolved with it.
    pub containment: &'a Containment,
    /// The rectangle the whole picture occupies in world coordinates.
    pub world: Rect,
    pub edges: &'a [EdgeVisual],
    /// The boxes the plan has touched. Everything else stands as it is.
    pub nodes: &'a BTreeMap<ElementId, NodeStatus>,
    /// The new names the plan gives boxes, so a renamed one says what it
    /// becomes.
    pub renames: &'a Renames,
    pub selected_edge: Option<&'a Relation>,
    pub selected_node: Option<&'a ElementId>,
    pub draw_source: Option<&'a ElementId>,
}

/// The three colours the plan speaks in: what is going, what is arriving,
/// and what stays and changes. They carry the same weight on a dark canvas,
/// so no mark shouts over the others and the picture reads as one plan.
pub const SEVERED: Color32 = Color32::from_rgb(205, 70, 60);
pub const DRAWN: Color32 = Color32::from_rgb(70, 165, 80);
pub const MODIFIED: Color32 = Color32::from_rgb(214, 162, 48);

/// The two colours a selection speaks in: what the selected boundary depends
/// on, and what depends on it. A selection asks both questions at once, and
/// a single ink for both leaves the reader counting arrowheads to tell the
/// answers apart. Cyan and magenta are the two hues the plan's red, green
/// and amber leave free - orange would sit beside both the amber and the
/// red - so a lit connection never reads as a planned one. Both run bright:
/// they carry the answer the reader asked for and paint over the picture to
/// give it.
pub const DEPENDENCY: Color32 = Color32::from_rgb(60, 190, 215);
pub const DEPENDENT: Color32 = Color32::from_rgb(225, 105, 195);

/// The size a label paints at while the camera stands at 1:1.
pub(crate) const LABEL_SIZE: f32 = 13.0;
/// The smallest font that still reads: below this a label paints texture
/// instead of a name, so it paints nothing at all.
pub(crate) const LEGIBLE_FONT: f32 = 4.0;
/// The thinnest a stroke ever draws, however far the camera pulls back.
pub(crate) const HAIRLINE: f32 = 0.75;
/// Screen-pixel distance within which a pointer catches an edge.
const EDGE_REACH: f32 = 8.0;
/// Straight segments each cubic of a route flattens into for drawing and
/// hit-testing.
const CURVE_SEGMENTS: u16 = 24;
/// Color strength left to everything outside the selection's neighborhood.
const FADE: f32 = 0.18;
/// Color strength left to the frames around the selection's neighborhood:
/// enough to read their names, little enough to stay background.
const CONTEXT: f32 = 0.55;
/// Color strength of a frame's border: the frame is the room its parts sit
/// in, so its outline stays a step behind the name it carries.
const FRAME_BORDER: f32 = 0.6;
/// Color strength of a kind glyph beside the name it marks.
const GLYPH: f32 = 0.55;
/// The gap between a kind glyph and its name, in font sizes.
const GLYPH_GAP: f32 = 0.3;
/// How far a frame's interior steps from the backdrop toward the tint of its
/// nesting level.
const WASH: f32 = 0.06;
/// Stroke width of an edge that stands for a single concrete dependency.
const EDGE_WIDTH: f32 = 1.2;
/// Width added per square root of the further concrete dependencies a
/// rolled-up edge stands for.
const WEIGHT_WIDTH: f32 = 0.7;
/// The widest an edge draws, however much it stands for: past this the
/// stroke stops reading as a line and starts hiding the boxes.
const MAX_EDGE_WIDTH: f32 = 4.0;
/// Width added while the pointer catches an edge.
const HOVER_WIDTH: f32 = 0.7;
/// Width added while an edge is selected.
const SELECTED_WIDTH: f32 = 1.5;
/// Width and color strength left to an edge that stays inside one top-level
/// boundary: a boundary's internal wiring is not the picture's subject, and
/// a dense boundary would otherwise drown the crossings around it.
const INTRA_WIDTH: f32 = 0.75;
const INTRA: f32 = 0.6;
/// How many edges must share one side before their ink is at its calmest. A
/// popular module receives fifteen to twenty arrows; past that the picture
/// says "many", and the count itself is the panel's answer, not the paint's.
const CROWD_FULL: f32 = 16.0;
/// The ink a stroke keeps in a full crowd: enough to follow one stroke with
/// the eye, little enough that twenty arrivals read as a calm mass instead
/// of a thicket over the box they reach. A stroke that runs alone keeps all
/// of its ink.
const CROWD_INK: f32 = 0.45;
/// Color strength of a summary block's fill: far past the wash of a frame
/// that shows its parts, so a block reads as the one solid thing it is.
const BLOCK: f32 = 0.22;
/// The size of a block's count line, in shares of the name above it.
const COUNT_SIZE: f32 = 0.8;
/// The margin a summary block keeps around its text.
const BLOCK_MARGIN: f32 = 6.0;
/// Opacity of the minimap's own background: enough to lift the map off the
/// picture, little enough to read the picture through it. The map is the one
/// surface whose translucency carries meaning - it lies over the whole
/// picture and must let it through - so it stays alpha-based while the
/// picture itself paints flat.
const MAP_FILL: f32 = 0.85;
/// Color strength of the map's frame, of the fill of the boxes on it, and
/// of their outline. The map is chrome about the picture, so it stays under
/// everything the picture itself draws. All three lie on the map's own
/// translucent surface rather than on the backdrop, so they thin their alpha
/// along with it.
const MAP_BORDER: f32 = 0.3;
const MAP_BOX_FILL: f32 = 0.14;
const MAP_BOX_BORDER: f32 = 0.45;
/// Width of the rectangle that marks where the reader stands on the map.
const HERE_WIDTH: f32 = 1.5;
/// How far past its own box a mark of the picture may still put ink, in
/// screen pixels. A border straddles the box edge, an arrowhead spreads
/// across the line it ends, and the dot of a severed connection sits on the
/// line: at the closest the camera ever comes, the widest of them reaches a
/// little over twenty pixels out. Everything that decides whether a mark is
/// worth building gives it this much room, so nothing is dropped that the
/// reader would have seen a sliver of.
const OVERHANG: f32 = 32.0;

pub fn show(
    ui: &mut Ui,
    content: &Content<'_>,
    camera: &mut Camera,
    strokes: &mut Strokes,
) -> Option<CanvasAction> {
    let viewport = ui.max_rect();
    let world = content.world;

    // The background sits below every node, so nodes win overlapping hits.
    let background = ui.interact(
        viewport,
        ui.id().with("canvas-background"),
        Sense::click_and_drag(),
    );
    // The map paints and answers the pointer in a layer of its own above the
    // picture, so a click on it reaches neither the boxes nor the background
    // beneath it. Travel lands here, before anything else this frame reads
    // the camera.
    let (camera, map) = {
        let mut current = camera.advance(ui, world, viewport);
        current = camera::steer(ui, &background, viewport, world, camera, current);
        let map = Minimap::of(world, viewport, current);
        if let Some(map) = &map
            && let Some(travelled) = travel(ui, map, content.layout, viewport, current)
        {
            // A grip on the map is the reader's hand on the picture: it
            // must answer under the finger, never travel behind it.
            current = camera.hold(travelled);
        }
        (current, map)
    };

    // What the magnification has shrunk past reading decides before anything
    // paints, hit-tests, or routes: the picture that follows is the one that
    // stands, boxes and edges alike. Nothing of it follows the pointer, so
    // it is decided again only where the scene or the magnification moved.
    strokes.refresh(
        content,
        Vantage {
            scene: content.generation,
            zoom: camera.scaling,
        },
    );
    strokes.project(camera);
    let touched = interact_nodes(ui, content.layout, &strokes.summary, camera, viewport);
    let hovered_node = touched.hovered;

    // The map is opaque to the picture beneath it: an edge that runs under
    // the map is not the edge the reader points at.
    let pointer = ui
        .input(|input| input.pointer.hover_pos())
        .filter(|position| viewport.contains(*position))
        .filter(|position| !map.as_ref().is_some_and(|map| map.rect.contains(*position)));
    let hovered_edge = match &hovered_node {
        Some(_) => None,
        None => pointer.and_then(|position| nearest_curve(&strokes.drawn, position, camera)),
    };
    // One reading of the labels answers the whole frame: the tooltip over a
    // stroke and every box of the picture name the same boundaries.
    let labels = Labels::over(content.view, content.containment, content.renames);
    if let Some(index) = hovered_edge {
        ui.ctx()
            .output_mut(|output| output.cursor_icon = CursorIcon::PointingHand);
        describe_edge(ui, content, &labels, &strokes.drawn[index].bundle);
    }

    let visuals = ui.visuals();
    let surface = Paint {
        painter: ui.painter().with_clip_rect(viewport),
        camera,
        viewport,
        background: visuals.panel_fill,
        base: visuals.text_color(),
        fill: visuals.extreme_bg_color,
        washes: washes(visuals),
        labels,
        focus: focus(content),
    };
    paint(
        &surface,
        content,
        strokes,
        &Pointed {
            node: hovered_node.as_ref(),
            edge: hovered_edge,
        },
    );

    // A double click also reports as a click, and its first click already
    // selected the node; the deeper answer is the one that stands.
    if let Some(id) = touched.double_clicked {
        return Some(CanvasAction::Expand(id));
    }
    if let Some(id) = touched.clicked {
        return Some(CanvasAction::Node(id));
    }
    if background.clicked() {
        return background
            .interact_pointer_pos()
            .and_then(|position| nearest_curve(&strokes.drawn, position, camera))
            .map(|index| {
                CanvasAction::Edge(
                    content.edges[strokes.drawn[index].bundle.lead]
                        .relation
                        .clone(),
                )
            })
            .or(Some(CanvasAction::Background));
    }
    None
}

/// What a set of routed strokes answers for: the scene that laid the boxes
/// out, and the magnification the summary decided against.
///
/// Neither the pointer nor the selection appears here, because neither
/// changes where a stroke runs: they change only how strongly it paints.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Vantage {
    scene: u64,
    zoom: f32,
}

/// The strokes of the picture, kept between frames.
///
/// Deciding what the magnification blurs, gathering the edges into strokes
/// and routing them around the boxes is the most expensive work the canvas
/// does, and none of it follows the reader's hand. A frame that changed
/// neither the scene nor the magnification therefore draws the strokes it
/// drew last time; a frame that only moved the camera flattens nothing anew
/// and merely places the lines it already holds in front of it.
#[derive(Default)]
pub struct Strokes {
    /// What the held strokes answer for; None until the first frame routes
    /// them.
    vantage: Option<Vantage>,
    summary: Summary,
    drawn: Vec<DrawnEdge>,
}

impl Strokes {
    /// Brings the strokes up to date for one scene at one magnification.
    ///
    /// The summary answers to both, so a turn of the wheel decides it anew.
    /// The strokes answer to the scene and to what the summary blurs alone:
    /// pulling back a little changes the magnification without changing what
    /// stands for what, and then nothing is bundled or routed again.
    fn refresh(&mut self, content: &Content<'_>, vantage: Vantage) {
        if self.vantage == Some(vantage) {
            return;
        }
        let same_scene = self.vantage.is_some_and(|held| held.scene == vantage.scene);
        self.vantage = Some(vantage);
        let summary = summarize(content.containment, content.layout, vantage.zoom);
        let stands = same_scene && summary.stands_for() == self.summary.stands_for();
        self.summary = summary;
        if stands {
            return;
        }
        // The edges gather into strokes before anything is routed: the
        // routing spreads one anchor per edge along a side, so a pile the
        // picture draws as one line must arrive there as one line.
        let bundles = bundle::bundles(content.edges, self.summary.stands_for());
        let routes = routing::routes(
            content.view,
            content.layout,
            self.summary.stands_for(),
            bundles
                .iter()
                .map(|stroke| &content.edges[stroke.lead].relation),
        );
        self.drawn = bundles
            .into_iter()
            .zip(routes)
            .map(|(bundle, route)| DrawnEdge {
                curve: route.as_ref().map(|route| Curve::of(&route.path)),
                route,
                bundle,
                screen: Vec::new(),
            })
            .collect();
    }

    /// Places the flattened lines in front of the camera, writing into the
    /// buffers of the last frame. The camera moves every frame and the lines
    /// themselves do not, so the run is a scale and a shift per point and
    /// nothing is allocated after the first frame of a set of strokes.
    fn project(&mut self, camera: TSTransform) {
        for stroke in &mut self.drawn {
            stroke.screen.clear();
            let Some(curve) = &stroke.curve else {
                continue;
            };
            stroke
                .screen
                .extend(curve.points.iter().map(|point| camera.mul_pos(*point)));
        }
    }
}

/// One stroke of the picture: the edges it draws, where it runs, the line it
/// flattens to in world coordinates, and that same line in front of the
/// camera of the current frame.
struct DrawnEdge {
    bundle: Bundle,
    route: Option<Route>,
    curve: Option<Curve>,
    /// The world line placed on the screen. Empty while the stroke has no
    /// route to draw.
    screen: Vec<Pos2>,
}

/// One run flattened to a polyline in world coordinates, with the box it
/// occupies. The box answers the pointer before the points do: a stroke
/// whose box the pointer misses cannot be the stroke the pointer catches.
struct Curve {
    points: Vec<Pos2>,
    bounds: Rect,
}

impl Curve {
    fn of(path: &Path) -> Self {
        let points = path.points(CURVE_SEGMENTS);
        let bounds = Rect::from_points(&points);
        Self { points, bounds }
    }
}

/// Names the stroke under the pointer: which boundaries it joins, how many
/// concrete dependencies it stands for, and what the plan does to it. A
/// stroke that draws several connections names the pair of boxes it runs
/// between and the one connection a click on it selects. The canvas
/// hit-tests its own edges, so the tooltip follows the pointer instead of a
/// widget.
fn describe_edge(ui: &Ui, content: &Content<'_>, labels: &Labels<'_>, bundle: &Bundle) {
    let Some(lead) = content.edges.get(bundle.lead) else {
        return;
    };
    let joins = format!(
        "{} {} {}",
        labels.qualified(&lead.relation.from),
        glyph::OUTWARD,
        labels.qualified(&lead.relation.to)
    );
    let heading = if bundle.merged() {
        format!(
            "{} connections between {} and {}",
            bundle.members.len(),
            labels.qualified(&bundle.from),
            labels.qualified(&bundle.to)
        )
    } else {
        joins.clone()
    };
    let selects = bundle
        .merged()
        .then(|| format!("Click selects the heaviest: {joins}"));
    // A planned addition stands for nothing concrete yet, so it counts
    // nothing; every edge the architecture already carries does.
    let stands_for = match (lead.status, bundle.weight) {
        (EdgeStatus::Drawn, _) => None,
        (_, 1) => Some("1 concrete dependency".to_owned()),
        (_, many) => Some(format!("{many} concrete dependencies")),
    };
    egui::Tooltip::always_open(
        ui.ctx().clone(),
        ui.layer_id(),
        ui.id().with("edge-tooltip"),
        egui::PopupAnchor::Pointer,
    )
    .show(|ui| {
        ui.label(heading);
        if let Some(stands_for) = stands_for {
            ui.label(stands_for);
        }
        if let Some(selects) = selects {
            ui.label(selects);
        }
        match lead.status {
            EdgeStatus::Existing => {}
            EdgeStatus::Severed => {
                ui.colored_label(SEVERED, "Planned for removal.");
            }
            EdgeStatus::PartiallySevered { severed, total } => {
                ui.colored_label(
                    SEVERED,
                    format!("{severed} of {total} concrete dependencies severed."),
                );
            }
            EdgeStatus::Drawn => {
                ui.colored_label(DRAWN, "Planned addition.");
            }
        }
    });
}

/// Paints the corner overview and answers the pointer on it: a click or a
/// drag anywhere on the map travels there, and the marked rectangle follows
/// the pointer as a scrollbar thumb does. Returns the camera the map asks
/// for, and None while nobody points at it.
///
/// The map lives in a layer above the picture, so its area takes the click
/// before the boxes and the background under it ever see it.
fn travel(
    ui: &Ui,
    map: &Minimap,
    layout: &Layout,
    viewport: Rect,
    camera: TSTransform,
) -> Option<TSTransform> {
    let mut travelled = None;
    egui::Area::new(ui.id().with("minimap"))
        .order(egui::Order::Middle)
        .fixed_pos(map.rect.min)
        .show(ui.ctx(), |ui| {
            let response = ui.allocate_rect(map.rect, Sense::click_and_drag());
            if let Some(pointer) = response.interact_pointer_pos() {
                travelled = Some(map.travel(pointer, viewport, camera));
            }
            if response.hovered() {
                ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
            }
            paint_map(ui, map, layout, viewport, travelled.unwrap_or(camera));
        });
    travelled
}

/// Draws the world at map scale: the outer boxes as plain outlines, no
/// edges and no names. At this size a name is a smudge and an edge is a
/// scribble; the shape of the picture and the place the reader stands in it
/// are the whole message.
fn paint_map(ui: &Ui, map: &Minimap, layout: &Layout, viewport: Rect, camera: TSTransform) {
    let visuals = ui.visuals();
    let ink = visuals.text_color();
    let painter = ui.painter();
    painter.rect(
        map.rect,
        CornerRadius::same(4),
        visuals.panel_fill.gamma_multiply(MAP_FILL),
        Stroke::new(1.0, ink.gamma_multiply(MAP_BORDER)),
        StrokeKind::Middle,
    );
    for boundary in map.boxes(layout) {
        painter.rect(
            boundary,
            CornerRadius::ZERO,
            ink.gamma_multiply(MAP_BOX_FILL),
            Stroke::new(HAIRLINE, ink.gamma_multiply(MAP_BOX_BORDER)),
            StrokeKind::Middle,
        );
    }
    painter.rect_stroke(
        map.looked_at(viewport, camera),
        CornerRadius::ZERO,
        Stroke::new(HERE_WIDTH, visuals.selection.stroke.color),
        StrokeKind::Middle,
    );
}

/// What the pointer did to the nodes this frame.
struct Touched {
    hovered: Option<ElementId>,
    clicked: Option<ElementId>,
    double_clicked: Option<ElementId>,
}

/// Registers a click-and-hover area for every node the picture paints. The
/// nodes register after the background, so a click that lands on a node
/// never reaches the background - which is what keeps a double-clicked node
/// out of the background's refit.
fn interact_nodes(
    ui: &mut Ui,
    layout: &Layout,
    summary: &Summary,
    camera: TSTransform,
    viewport: Rect,
) -> Touched {
    let targets = targets(layout, summary);

    let mut touched = Touched {
        hovered: None,
        clicked: None,
        double_clicked: None,
    };
    for (id, rect) in targets {
        let on_screen = camera.mul_rect(rect).intersect(viewport);
        if !on_screen.is_positive() {
            continue;
        }
        let response = ui
            .interact(on_screen, ui.id().with(id.as_str()), Sense::click())
            .on_hover_cursor(CursorIcon::PointingHand);
        if response.hovered() {
            touched.hovered = Some(id.clone());
        }
        if response.clicked() {
            touched.clicked = Some(id.clone());
        }
        if response.double_clicked() {
            touched.double_clicked = Some(id.clone());
        }
    }
    touched
}

/// What the pointer can touch, in the order the areas register: containers
/// first, so a leaf inside one wins the overlapping hit. A container answers
/// on its header strip alone, because its interior belongs to its children;
/// a summary block has no interior to give away, so it answers over its
/// whole box, exactly as a leaf does. Nothing a block stands for offers the
/// pointer anything at all.
fn targets<'a>(layout: &'a Layout, summary: &Summary) -> Vec<(&'a ElementId, Rect)> {
    let mut targets: Vec<(&ElementId, Rect)> = layout
        .containers
        .iter()
        .filter(|frame| !summary.hides(&frame.id))
        .map(|frame| {
            let rect = layout.rects[&frame.id];
            let touchable = match summary.block(&frame.id) {
                Some(_) => rect,
                None => header_of(rect),
            };
            (&frame.id, touchable)
        })
        .collect();
    targets.extend(
        layout
            .leaves
            .iter()
            .filter(|id| !summary.hides(id))
            .map(|id| (id, layout.rects[id])),
    );
    targets
}

/// The clickable strip along a container's top edge.
fn header_of(rect: Rect) -> Rect {
    Rect::from_min_size(rect.min, vec2(rect.width(), HEADER))
}

struct Paint<'a> {
    painter: egui::Painter,
    camera: TSTransform,
    /// The screen rectangle the picture paints into. A mark that misses it
    /// costs the same to build as one the reader sees, so it is never built.
    viewport: Rect,
    /// The surface the whole picture lies on. Every weakened mark blends
    /// against it instead of thinning its own alpha, so no two weakened
    /// marks can stack into a stronger one.
    background: Color32,
    base: Color32,
    fill: Color32,
    /// The inks frames tint their interiors with, by nesting depth parity.
    washes: [Color32; 2],
    labels: Labels<'a>,
    focus: Option<Focus<'a>>,
}

impl Paint<'_> {
    /// The label font at the current zoom; None when too small to read.
    fn font(&self) -> Option<FontId> {
        let size = LABEL_SIZE * self.camera.scaling;
        (size >= LEGIBLE_FONT).then(|| FontId::proportional(size))
    }

    fn stroke_width(&self, width: f32) -> f32 {
        (width * self.camera.scaling).max(HAIRLINE)
    }

    fn element(&self, id: &ElementId) -> Strength {
        self.focus
            .as_ref()
            .map_or(Strength::Focused, |focus| focus.element(id))
    }

    fn edge(&self, relation: &Relation) -> Strength {
        self.focus
            .as_ref()
            .map_or(Strength::Focused, |focus| focus.edge(relation))
    }

    /// Which way this edge runs about the selection. Nothing selected is
    /// nothing asked, so no edge runs any way at all.
    fn direction(&self, relation: &Relation) -> Option<Direction> {
        self.focus
            .as_ref()
            .and_then(|focus| focus.direction(relation))
    }

    /// The ink a frame at this nesting depth tints its interior toward.
    fn wash(&self, depth: usize) -> Color32 {
        self.washes[depth % 2]
    }
}

/// The tints that separate one nesting level from the next: consecutive
/// depths wash in opposite directions, one toward the text and one toward
/// the deepest background, so no frame shares the shade of the frame around
/// it. Both directions come from the theme, so a light and a dark canvas
/// read alike. Each frame steps only [`WASH`] of the way toward its tint,
/// which leaves the boxes and the edges the picture.
fn washes(visuals: &egui::Visuals) -> [Color32; 2] {
    [visuals.text_color(), visuals.extreme_bg_color]
}

/// A flat step from the backdrop toward an ink: each channel moves `share`
/// of the way, and the answer is opaque.
///
/// This is what weakening a mark means on this canvas. Weakening by alpha
/// instead lets marks composite: two faded strokes crossing on a dark
/// backdrop add their ink and read brighter than either, which is exactly
/// the shout the fade exists to prevent. A flat step answers one color per
/// (backdrop, ink, share), so a hundred faded strokes over one another read
/// as one.
///
/// Blending the same backdrop twice is the same as blending once with the
/// product of the shares, so callers that weaken for several reasons
/// multiply the shares and step once - one rounding, one answer.
fn toward(background: Color32, ink: Color32, share: f32) -> Color32 {
    // A share outside the range names no color between the two ends, so it
    // holds at the end it passed.
    let share = share.clamp(0.0, 1.0);
    // Both ends enter opaque, which is what makes the step a step between
    // two colors rather than a lerp of two coverages.
    let opaque = |color: Color32| Color32::from_rgb(color.r(), color.g(), color.b());
    opaque(background).lerp_to_gamma(opaque(ink), share)
}

/// How much of its ink a mark keeps at each strength.
fn share_of(strength: Strength) -> f32 {
    match strength {
        Strength::Focused => 1.0,
        Strength::Context => CONTEXT,
        Strength::Faded => FADE,
    }
}

/// The ink a box draws its border and its name in: the plan's colour where
/// the plan has touched the box, the picture's own ink everywhere else. The
/// plan speaks before the focus dims, so a marked box still fades into the
/// background when the reader asks about something else.
fn node_ink(base: Color32, status: Option<&NodeStatus>) -> Color32 {
    match status {
        None => base,
        Some(NodeStatus::Removed) => SEVERED,
        Some(NodeStatus::Added) => DRAWN,
        Some(NodeStatus::Modified) => MODIFIED,
    }
}

/// The color a mark paints in at a given strength: the ink itself in focus,
/// and a flat step from the backdrop toward it everywhere else.
fn shade(background: Color32, ink: Color32, strength: Strength) -> Color32 {
    toward(background, ink, share_of(strength))
}

/// The ink a stroke draws its line, its arrowhead and its annotation mark
/// in.
///
/// The plan speaks first: a connection it removes or adds is going or
/// arriving whatever the reader asks about. Everything the architecture
/// carries answers the selection instead, one colour per question - what the
/// selection depends on, what depends on the selection - so the two answers
/// never have to be told apart by following an arrow to its head. A
/// connection with both ends inside the selection answers neither question
/// and keeps the picture's own ink, as does every connection while nothing
/// is selected.
fn edge_ink(base: Color32, status: EdgeStatus, direction: Option<Direction>) -> Color32 {
    match status {
        EdgeStatus::Severed => SEVERED,
        EdgeStatus::Drawn => DRAWN,
        // A partly severed connection mostly stays, so it reads as the
        // connection it is; the red mark by its arrowhead carries the plan's
        // part of the story.
        EdgeStatus::Existing | EdgeStatus::PartiallySevered { .. } => match direction {
            Some(Direction::Outgoing) => DEPENDENCY,
            Some(Direction::Incoming) => DEPENDENT,
            Some(Direction::Internal) | None => base,
        },
    }
}

/// The one way a stroke runs about the selection, out of the ways the edges
/// it draws run. A stroke is a single line and carries a single answer, so
/// edges that disagree - and a stroke the selection says nothing about -
/// leave it without a direction.
fn agreed(directions: impl IntoIterator<Item = Option<Direction>>) -> Option<Direction> {
    let mut agreed: Option<Direction> = None;
    for direction in directions.into_iter().flatten() {
        match agreed {
            None => agreed = Some(direction),
            Some(first) if first == direction => {}
            Some(_) => return None,
        }
    }
    agreed
}

/// How each stroke of one frame paints: how strongly, and whether it paints
/// over the boxes or under them. Both answers come from one walk over the
/// strokes, and both passes of the painting read them.
struct Emphasis {
    strengths: Vec<Strength>,
    lifted: Vec<bool>,
}

impl Emphasis {
    fn of(
        paint: &Paint<'_>,
        content: &Content<'_>,
        strokes: &Strokes,
        hovered: Option<usize>,
    ) -> Emphasis {
        let strengths: Vec<Strength> = strokes
            .drawn
            .iter()
            .map(|stroke| {
                stroke
                    .bundle
                    .strength(|edge| paint.edge(&content.edges[edge].relation))
            })
            .collect();
        let lifted = lifted(&strengths, hovered, paint.focus.is_some());
        Emphasis { strengths, lifted }
    }
}

/// Which strokes paint after the boxes instead of before them.
///
/// The strokes a selection lights are the answer the reader asked for, and
/// an answer painted under the boxes and among the faded strokes is one the
/// reader has to dig out: a leaf covers it, and any dim stroke drawn later
/// crosses over it. So the lit strokes - and the one the pointer holds -
/// paint last, over everything. With nothing selected there is no answer to
/// lift and every stroke paints in one pass, under the boxes it runs between.
fn lifted(strengths: &[Strength], hovered: Option<usize>, selected: bool) -> Vec<bool> {
    strengths
        .iter()
        .enumerate()
        .map(|(index, strength)| {
            selected && (*strength == Strength::Focused || hovered == Some(index))
        })
        .collect()
}

/// What the pointer holds this frame: at most one box, or at most one
/// stroke. A box under the pointer takes it, so the two never answer at
/// once.
struct Pointed<'a> {
    node: Option<&'a ElementId>,
    edge: Option<usize>,
}

fn paint(paint: &Paint<'_>, content: &Content<'_>, strokes: &Strokes, pointed: &Pointed<'_>) {
    let emphasis = Emphasis::of(paint, content, strokes, pointed.edge);
    paint_containers(paint, content, &strokes.summary, pointed.node);
    paint_edges(paint, content, strokes, pointed, &emphasis, false);
    paint_leaves(paint, content, &strokes.summary, pointed.node);
    paint_edges(paint, content, strokes, pointed, &emphasis, true);
}

/// The selection's neighborhood. None when nothing is selected: then
/// nothing fades.
fn focus<'a>(content: &Content<'a>) -> Option<Focus<'a>> {
    let selected = match (content.selected_node, content.selected_edge) {
        (Some(id), _) => Selected::Node(id),
        (None, Some(relation)) => Selected::Edge(relation),
        (None, None) => return None,
    };
    Some(focus_of(
        content.containment,
        content.edges.iter().map(|edge| &edge.relation),
        selected,
    ))
}

/// Whether a mark that occupies this screen rectangle reaches the canvas at
/// all. A mark entirely beside the viewport paints nothing a reader can see,
/// and building its shapes costs exactly as much as building a visible one.
///
/// The rectangle is the mark's own box; the border, the arrowhead and the
/// dots a mark carries reach a little past it, so the test gives every mark
/// [`OVERHANG`] of slack and never drops one that grazes the edge.
fn on_screen(mark: Rect, viewport: Rect) -> bool {
    mark.expand(OVERHANG).intersects(viewport)
}

fn paint_containers(
    paint: &Paint<'_>,
    content: &Content<'_>,
    summary: &Summary,
    hovered: Option<&ElementId>,
) {
    for frame in &content.layout.containers {
        let id = &frame.id;
        if summary.hides(id) {
            continue;
        }
        let rect = paint.camera.mul_rect(content.layout.rects[id]);
        if !on_screen(rect, paint.viewport) {
            continue;
        }
        let block = summary.block(id);
        // A block is the only mark its contents have left, so it paints as
        // strongly as the strongest thing it stands for.
        let strength = match block {
            Some(block) => block.strength(|inside| paint.element(inside)),
            None => paint.element(id),
        };
        let selected = content.selected_node == Some(id) || content.draw_source == Some(id);
        let width = if selected {
            2.5
        } else if hovered == Some(id) {
            1.8
        } else {
            1.0
        };
        let ink = node_ink(paint.base, content.nodes.get(id));
        let border = Stroke::new(
            paint.stroke_width(width),
            toward(paint.background, ink, FRAME_BORDER * share_of(strength)),
        );
        // A frame's interior lies on the backdrop and on the frames that
        // hold it, never on a box or an edge - containers paint before
        // both. Nothing is meant to read through it, so it paints flat:
        // translucent tints would add wherever nesting stacks them, and the
        // deepest frame would glow instead of receding.
        let (tint, presence) = match block {
            Some(_) => (paint.base, BLOCK),
            None => (paint.wash(frame.depth), WASH),
        };
        paint.painter.rect(
            rect,
            CornerRadius::same(6),
            toward(paint.background, tint, presence * share_of(strength)),
            border,
            StrokeKind::Middle,
        );
        match block {
            Some(block) => paint_block_text(paint, rect, id, block, strength, ink),
            None => {
                if let Some(font) = paint.font() {
                    paint.painter.text(
                        rect.min + vec2(10.0, 6.0) * paint.camera.scaling,
                        Align2::LEFT_TOP,
                        paint.labels.label(id).text(),
                        font,
                        shade(paint.background, ink, strength),
                    );
                }
            }
        }
    }
}

/// What a summary block says: the frame's name across the middle, and under
/// it how many boundaries the block hides. Each line paints only while the
/// block has room for it, so a block too small even for its name reads as a
/// solid mass - which still tells the reader that something sits there.
///
/// The text is laid out in screen space, not in the world the camera scales.
/// A block appears exactly when the camera has pulled far enough back to
/// shrink its contents past reading, which is where a world-scaled font has
/// stopped reading too: the block would then be a large anonymous shape. It
/// takes the comfortable reading size instead, shrunk only as far as its own
/// screen box demands, so pulling back swaps a frame's contents for its name
/// the way a map swaps streets for a city.
///
/// A block alone earns screen-space text. A frame that shows its parts hangs
/// its name in a header strip that shrinks with the camera, and text held at
/// screen size would spill out of the strip and across the very children the
/// frame exists to show. A block has no children to spill onto: the name is
/// its whole content.
fn paint_block_text(
    paint: &Paint<'_>,
    rect: Rect,
    id: &ElementId,
    block: &Block,
    strength: Strength,
    ink: Color32,
) {
    let named = shade(paint.background, ink, strength);
    let text = paint.labels.label(id).text();
    let comfortable = FontId::proportional(LABEL_SIZE);
    let measured = paint
        .painter
        .layout_no_wrap(text.clone(), comfortable, named)
        .size();
    let room = rect.size() - Vec2::splat(2.0 * BLOCK_MARGIN);
    let Some(size) = fitted_size(room, measured, LABEL_SIZE, LEGIBLE_FONT) else {
        return;
    };
    let font = FontId::proportional(size);
    let name = paint.painter.layout_no_wrap(text, font.clone(), named);
    let center = rect.center();
    let counted = toward(paint.background, ink, GLYPH * share_of(strength));
    let count = (block.inside > 0).then(|| {
        paint.painter.layout_no_wrap(
            format!("{} inside", block.inside),
            FontId::proportional(font.size * COUNT_SIZE),
            counted,
        )
    });
    let (name_size, count_size) = (name.size(), count.as_ref().map_or(Vec2::ZERO, |c| c.size()));
    let both = vec2(name_size.x.max(count_size.x), name_size.y + count_size.y);
    let Some(count) = count.filter(|_| fits(both, rect)) else {
        paint.painter.galley(center - name_size / 2.0, name, named);
        return;
    };
    let top = center.y - both.y / 2.0;
    paint
        .painter
        .galley(pos2(center.x - name_size.x / 2.0, top), name, named);
    paint.painter.galley(
        pos2(center.x - count_size.x / 2.0, top + name_size.y),
        count,
        counted,
    );
}

/// Whether a run of text fits inside a summary block, its margin included.
fn fits(text: Vec2, rect: Rect) -> bool {
    text.x + 2.0 * BLOCK_MARGIN <= rect.width() && text.y + 2.0 * BLOCK_MARGIN <= rect.height()
}

/// The size a run of text paints at inside the room it is given: the size
/// asked for while the text already fits, shrunk in proportion while it does
/// not, and nothing at all once the fit would fall below the floor of
/// legibility. Both extents bind, so a wide flat box shrinks the text as
/// readily as a narrow one.
///
/// A glyph scales with its point size, so measuring the run once at the
/// desired size answers for every smaller size as well.
fn fitted_size(room: Vec2, at_desired: Vec2, desired: f32, floor: f32) -> Option<f32> {
    let share = |room: f32, needed: f32| {
        if needed > 0.0 {
            room / needed
        } else {
            f32::INFINITY
        }
    };
    let fitted = desired
        * share(room.x, at_desired.x)
            .min(share(room.y, at_desired.y))
            .min(1.0);
    (fitted >= floor).then_some(fitted)
}

/// Draws one pass of the strokes: the ones that lie under the boxes, or the
/// lit ones that lie over them. Each stroke carries its arrowhead and its
/// marks into whichever pass it belongs to, so a lifted connection arrives
/// whole.
fn paint_edges(
    paint: &Paint<'_>,
    content: &Content<'_>,
    strokes: &Strokes,
    pointed: &Pointed<'_>,
    emphasis: &Emphasis,
    over: bool,
) {
    for (index, stroke) in strokes.drawn.iter().enumerate() {
        if emphasis.lifted[index] != over {
            continue;
        }
        let (Some(route), Some(curve)) = (&stroke.route, &stroke.curve) else {
            continue;
        };
        if !on_screen(paint.camera.mul_rect(curve.bounds), paint.viewport) {
            continue;
        }
        let points = &stroke.screen;
        let bundle = &stroke.bundle;
        let visual = |edge: usize| &content.edges[edge];
        // A stroke draws in the manner of the edge it answers for, and a
        // merged one gathers edges of the current architecture alone.
        let lead = visual(bundle.lead);
        let ink = edge_ink(
            paint.base,
            lead.status,
            agreed(
                bundle
                    .members
                    .iter()
                    .map(|edge| paint.direction(&visual(*edge).relation)),
            ),
        );
        let selected = bundle.any(|edge| content.selected_edge == Some(&visual(edge).relation));
        let hovered = pointed.edge == Some(index);
        let share = stroke_share(
            route.scope,
            route.crowd,
            emphasis.strengths[index],
            selected || hovered,
            paint.focus.is_some(),
        );
        let color = toward(paint.background, ink, share);
        let width = edge_width(bundle.weight, route.scope)
            + if selected {
                SELECTED_WIDTH
            } else if hovered {
                HOVER_WIDTH
            } else {
                0.0
            };
        let pen = Stroke::new(paint.stroke_width(width), color);
        if matches!(
            lead.status,
            EdgeStatus::Existing | EdgeStatus::PartiallySevered { .. }
        ) {
            paint.painter.add(Shape::line(points.clone(), pen));
        } else {
            let dash = paint.camera.scaling.max(0.5);
            for shape in Shape::dashed_line(points, pen, 8.0 * dash, 5.0 * dash) {
                paint.painter.add(shape);
            }
        }
        arrow_head(&paint.painter, points, color, paint.camera.scaling);
        if matches!(lead.status, EdgeStatus::PartiallySevered { .. }) {
            // The mark sits just short of the arrowhead: the reader looks
            // there to see what arrives, so that is where the plan speaks.
            paint.painter.circle_filled(
                points[(points.len() - 1) * 9 / 10],
                (4.0 * paint.camera.scaling).max(2.5),
                SEVERED,
            );
        }
        if bundle.any(|edge| visual(edge).annotated) {
            paint.painter.circle_filled(
                points[points.len() / 2],
                (4.0 * paint.camera.scaling).max(2.0),
                color,
            );
        }
    }
}

/// How thick an edge draws before selection and hover add to it: the weight
/// enters as its square root, so a boundary that answers ten dependencies
/// reads heavier than one that answers two without swallowing the picture.
fn edge_width(weight: usize, scope: Scope) -> f32 {
    let further = f32::from(u16::try_from(weight.saturating_sub(1)).unwrap_or(u16::MAX));
    let width = (EDGE_WIDTH + WEIGHT_WIDTH * further.sqrt()).min(MAX_EDGE_WIDTH);
    match scope {
        Scope::Intra => width * INTRA_WIDTH,
        Scope::Cross => width,
    }
}

/// How much of its ink one stroke keeps: how far it reaches, how much
/// company it keeps, and what the reader asked about.
///
/// A stroke answers the reader when the pointer holds it, when it is the
/// selected connection, or when a standing selection lights it. An answer
/// keeps every drop of its ink: it already paints over the boxes, and
/// holding it back for the room it runs in or the company it keeps buries
/// the very thing the reader asked to see.
///
/// Everything else falls back for its reach and its crowd: an internal edge
/// stays behind the crossings, and a stroke among many stays behind one
/// that runs alone. With nothing selected every stroke stands focused, so
/// the reach and the crowd are the whole of what separates the picture.
///
/// The three reasons meet in one share, and the caller steps once from the
/// backdrop with it: stepping once per reason would round three times, and
/// a stroke that crosses another would still add ink where they meet.
fn stroke_share(
    scope: Scope,
    crowd: usize,
    strength: Strength,
    asked_about: bool,
    selection_stands: bool,
) -> f32 {
    let answering = asked_about || (selection_stands && strength == Strength::Focused);
    let held_back = if answering {
        1.0
    } else {
        scope_ink(scope) * crowd_ink(crowd)
    };
    held_back * share_of(strength)
}

/// How much of its ink a stroke keeps for the reach it has: an edge that
/// stays inside one top-level boundary is not the picture's subject.
fn scope_ink(scope: Scope) -> f32 {
    match scope {
        Scope::Intra => INTRA,
        Scope::Cross => 1.0,
    }
}

/// How much of its ink a stroke keeps among the strokes that share its
/// busiest side. Thickness already carries what an edge stands for; this
/// carries how much company it keeps, so the two never speak over each
/// other. The fall is even from one stroke to a full side, so a side that
/// gains one more arrival never jumps.
fn crowd_ink(crowd: usize) -> f32 {
    let company = f32::from(u16::try_from(crowd.saturating_sub(1)).unwrap_or(u16::MAX));
    1.0 - (1.0 - CROWD_INK) * (company / (CROWD_FULL - 1.0)).min(1.0)
}

fn paint_leaves(
    paint: &Paint<'_>,
    content: &Content<'_>,
    summary: &Summary,
    hovered: Option<&ElementId>,
) {
    for id in &content.layout.leaves {
        if summary.hides(id) {
            continue;
        }
        let rect = paint.camera.mul_rect(content.layout.rects[id]);
        if !on_screen(rect, paint.viewport) {
            continue;
        }
        let strength = paint.element(id);
        let selected = content.selected_node == Some(id) || content.draw_source == Some(id);
        let hover = hovered == Some(id);
        let width = if selected {
            2.5
        } else if hover {
            1.8
        } else {
            1.0
        };
        let ink = node_ink(paint.base, content.nodes.get(id));
        let presence = if hover { 1.0 } else { 0.8 };
        // A leaf's box is solid where the reader looks, and a faded one is
        // not a window: an edge that runs under it stays under it either
        // way. So the fill steps toward the backdrop instead of thinning,
        // and a stroke beneath a faded box no longer half-shows through it.
        paint.painter.rect(
            rect,
            CornerRadius::same(5),
            shade(paint.background, paint.fill, strength),
            Stroke::new(
                paint.stroke_width(width),
                toward(paint.background, ink, presence * share_of(strength)),
            ),
            StrokeKind::Middle,
        );
        if let Some(font) = paint.font() {
            let named = share_of(strength);
            paint_leaf_label(
                &paint.painter,
                rect.center(),
                &paint.labels.label(id),
                &font,
                toward(paint.background, ink, named),
                toward(paint.background, ink, named * GLYPH),
            );
        }
    }
}

/// Writes a leaf's label centered in its box. The kind glyph paints dimmer
/// than the name beside it: the name is the subject, the glyph only says
/// what kind of thing carries it. Both colors arrive ready, each blended
/// from the backdrop in one step, so neither is a shade of the other.
/// Layout measures glyph and name together, so the pair always fits.
fn paint_leaf_label(
    painter: &egui::Painter,
    center: Pos2,
    label: &Label,
    font: &FontId,
    named: Color32,
    glyphed: Color32,
) {
    let name = painter.layout_no_wrap(label.name.clone(), font.clone(), named);
    let Some(glyph) = label.glyph else {
        let size = name.size();
        painter.galley(center - size / 2.0, name, named);
        return;
    };
    let glyph = painter.layout_no_wrap((*glyph).to_owned(), font.clone(), glyphed);
    let (glyph_size, name_size) = (glyph.size(), name.size());
    let gap = font.size * GLYPH_GAP;
    let left = center.x - (glyph_size.x + gap + name_size.x) / 2.0;
    painter.galley(pos2(left, center.y - glyph_size.y / 2.0), glyph, glyphed);
    painter.galley(
        pos2(left + glyph_size.x + gap, center.y - name_size.y / 2.0),
        name,
        named,
    );
}

fn arrow_head(painter: &egui::Painter, points: &[Pos2], color: Color32, zoom: f32) {
    let &[.., before, tip] = points else {
        return;
    };
    let direction = (tip - before).normalized();
    if !direction.is_finite() {
        return;
    }
    let normal = vec2(-direction.y, direction.x);
    let length = (10.0 * zoom).clamp(6.0, 20.0);
    let left = tip - direction * length + normal * (length / 2.0);
    let right = tip - direction * length - normal * (length / 2.0);
    painter.add(Shape::convex_polygon(
        vec![tip, left, right],
        color,
        Stroke::NONE,
    ));
}

/// Whether the pointer can be within [`EDGE_REACH`] of a line that stays
/// inside this screen box. A line never leaves its own box, so a pointer
/// further than the reach from the box is further than the reach from every
/// point of the line, and the walk over those points can be skipped.
fn within_reach(bounds: Rect, pointer: Pos2) -> bool {
    bounds.expand(EDGE_REACH).contains(pointer)
}

fn nearest_curve(strokes: &[DrawnEdge], pointer: Pos2, camera: TSTransform) -> Option<usize> {
    let mut best: Option<(f32, usize)> = None;
    for (index, stroke) in strokes.iter().enumerate() {
        let Some(curve) = &stroke.curve else {
            continue;
        };
        if !within_reach(camera.mul_rect(curve.bounds), pointer) {
            continue;
        }
        for pair in stroke.screen.windows(2) {
            let distance = distance_to_segment(pointer, pair[0], pair[1]);
            if distance < EDGE_REACH && best.is_none_or(|(closest, _)| distance < closest) {
                best = Some((distance, index));
            }
        }
    }
    best.map(|(_, index)| index)
}

fn distance_to_segment(point: Pos2, a: Pos2, b: Pos2) -> f32 {
    let ab = b - a;
    let length_squared = ab.length_sq();
    if length_squared == 0.0 {
        return (point - a).length();
    }
    let t = ((point - a).dot(ab) / length_squared).clamp(0.0, 1.0);
    (point - (a + ab * t)).length()
}

/// The rectangle the whole picture occupies in world coordinates.
pub(crate) fn world_bounds(layout: &Layout) -> Rect {
    layout
        .rects
        .values()
        .fold(Rect::NOTHING, |acc, rect| acc.union(*rect))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use cutaway_architecture::{Element, ElementKind, ElementName, RelationKind};

    use super::*;
    use crate::layout::Frame;

    fn id(text: &str) -> ElementId {
        ElementId::new(text).unwrap()
    }

    /// One frame around two leaves, with the boxes the layout would give
    /// them.
    fn frame_of_leaves() -> (ArchitectureGraph, Layout) {
        let mut graph = ArchitectureGraph::new();
        for element in ["package:a", "a/one", "a/two"] {
            graph
                .add_element(Element {
                    id: id(element),
                    name: ElementName::new(element).unwrap(),
                    kind: ElementKind::Module,
                    fingerprint: None,
                })
                .unwrap();
        }
        for inner in ["a/one", "a/two"] {
            graph
                .add_relation(Relation {
                    from: id("package:a"),
                    to: id(inner),
                    kind: RelationKind::Contains,
                })
                .unwrap();
        }
        let layout = Layout {
            rects: BTreeMap::from([
                (
                    id("package:a"),
                    Rect::from_min_size(pos2(0.0, 0.0), vec2(200.0, 160.0)),
                ),
                (
                    id("a/one"),
                    Rect::from_min_size(pos2(20.0, 40.0), vec2(100.0, 30.0)),
                ),
                (
                    id("a/two"),
                    Rect::from_min_size(pos2(20.0, 100.0), vec2(100.0, 30.0)),
                ),
            ]),
            containers: vec![Frame {
                id: id("package:a"),
                depth: 0,
            }],
            leaves: vec![id("a/one"), id("a/two")],
        };
        (graph, layout)
    }

    #[test]
    fn no_interaction_target_hides_under_a_summary_block() {
        let (view, layout) = frame_of_leaves();
        let summary = summarize(&Containment::of(&view), &layout, 0.1);

        let targets = targets(&layout, &summary);
        assert_eq!(
            targets.iter().map(|(id, _)| *id).collect::<Vec<_>>(),
            vec![&id("package:a")],
            "the block alone answers the pointer"
        );
        assert_eq!(
            targets[0].1,
            layout.rects[&id("package:a")],
            "a block answers over its whole box, as a leaf does"
        );
    }

    #[test]
    fn a_frame_that_shows_its_parts_answers_on_its_header_alone() {
        let (view, layout) = frame_of_leaves();
        let summary = summarize(&Containment::of(&view), &layout, 1.0);

        let targets = targets(&layout, &summary);
        assert_eq!(targets.len(), 3);
        assert_eq!(targets[0].0, &id("package:a"));
        assert_eq!(
            targets[0].1,
            header_of(layout.rects[&id("package:a")]),
            "a frame around its parts answers on its header strip alone"
        );
    }

    /// The extent of one name at [`LABEL_SIZE`], as the painter measures it.
    const MEASURED: Vec2 = Vec2::new(100.0, 15.0);

    #[test]
    fn a_block_with_room_for_its_name_paints_it_at_reading_size() {
        assert_eq!(
            fitted_size(vec2(400.0, 200.0), MEASURED, LABEL_SIZE, LEGIBLE_FONT),
            Some(LABEL_SIZE),
            "a name that already fits is never enlarged either"
        );
    }

    #[test]
    fn a_name_wider_than_its_block_shrinks_until_it_fits() {
        let fitted = fitted_size(vec2(50.0, 200.0), MEASURED, LABEL_SIZE, LEGIBLE_FONT)
            .expect("half the width still reads");
        assert!((fitted - LABEL_SIZE / 2.0).abs() < 0.01, "{fitted}");
    }

    #[test]
    fn a_flat_block_shrinks_its_name_to_the_height_it_has() {
        let fitted = fitted_size(vec2(400.0, 7.5), MEASURED, LABEL_SIZE, LEGIBLE_FONT)
            .expect("half the height still reads");
        assert!((fitted - LABEL_SIZE / 2.0).abs() < 0.01, "{fitted}");
    }

    #[test]
    fn a_name_that_would_shrink_past_reading_paints_nothing() {
        assert_eq!(
            fitted_size(vec2(20.0, 200.0), MEASURED, LABEL_SIZE, LEGIBLE_FONT),
            None,
            "a fifth of the width falls below the legible floor"
        );
        assert_eq!(
            fitted_size(vec2(-4.0, 200.0), MEASURED, LABEL_SIZE, LEGIBLE_FONT),
            None,
            "a box smaller than its own margins holds no text"
        );
    }

    #[test]
    fn a_block_label_never_scales_with_the_camera() {
        let far_out = fitted_size(vec2(400.0, 200.0), MEASURED, LABEL_SIZE, LEGIBLE_FONT);
        let close_in = fitted_size(vec2(4000.0, 2000.0), MEASURED, LABEL_SIZE, LEGIBLE_FONT);
        assert_eq!(
            far_out, close_in,
            "the block's screen box decides the size, and nothing else does"
        );
    }

    /// A dark canvas and its text: the pair every mark blends between.
    const BACKDROP: Color32 = Color32::from_rgb(27, 27, 27);
    const INK: Color32 = Color32::from_rgb(140, 140, 140);
    /// A light canvas and its text. The picture reads on either, so every
    /// invariant about colour answers for both.
    const LIGHT_BACKDROP: Color32 = Color32::from_rgb(248, 248, 248);
    const LIGHT_INK: Color32 = Color32::from_rgb(60, 60, 60);
    const THEMES: [(Color32, Color32); 2] = [(BACKDROP, INK), (LIGHT_BACKDROP, LIGHT_INK)];
    /// Every colour the picture speaks in beside the ink of its theme.
    const ACCENTS: [Color32; 5] = [SEVERED, DRAWN, MODIFIED, DEPENDENCY, DEPENDENT];
    /// How far apart two marks must lie in some channel to read as two
    /// marks. A smaller difference is one nobody sees, and two colours
    /// nobody can tell apart say the same thing.
    const SEPARATION: i32 = 6;

    fn apart(one: Color32, other: Color32) -> bool {
        let channels = |color: Color32| [color.r(), color.g(), color.b()].map(i32::from);
        channels(one)
            .into_iter()
            .zip(channels(other))
            .any(|(one, other)| (one - other).abs() >= SEPARATION)
    }

    #[test]
    fn a_mark_with_no_ink_left_is_the_backdrop_itself() {
        assert_eq!(toward(BACKDROP, INK, 0.0), BACKDROP);
    }

    #[test]
    fn a_mark_at_full_strength_paints_the_ink_it_was_given() {
        assert_eq!(toward(BACKDROP, INK, 1.0), INK);
        assert_eq!(toward(BACKDROP, SEVERED, 1.0), SEVERED);
    }

    #[test]
    fn a_weakened_mark_is_never_translucent() {
        for (backdrop, theme) in THEMES {
            for share in [0.0, FADE, CONTEXT, 0.99, 1.0] {
                for ink in ACCENTS.into_iter().chain([theme]) {
                    assert_eq!(
                        toward(backdrop, ink, share).a(),
                        255,
                        "share {share} of {ink:?} must cover what it paints over"
                    );
                }
            }
        }
    }

    #[test]
    fn two_faded_marks_over_each_other_read_as_one() {
        let first = toward(BACKDROP, SEVERED, FADE);
        let second = toward(BACKDROP, SEVERED, FADE);
        assert_eq!(
            first, second,
            "the same backdrop, ink and share answer the same color, and an \
             opaque one covers its twin exactly"
        );
    }

    #[test]
    fn every_color_the_picture_speaks_in_stays_apart_from_the_rest() {
        for (backdrop, theme) in THEMES {
            for share in [FADE, CONTEXT, 1.0] {
                let marks: Vec<Color32> = ACCENTS
                    .into_iter()
                    .chain([theme])
                    .map(|ink| toward(backdrop, ink, share))
                    .collect();
                for (index, mark) in marks.iter().enumerate() {
                    assert!(
                        apart(*mark, backdrop),
                        "at share {share}, {mark:?} must lift off {backdrop:?}"
                    );
                    for other in &marks[index + 1..] {
                        assert!(
                            apart(*mark, *other),
                            "at share {share} on {backdrop:?}, {mark:?} and {other:?} \
                             read as one colour"
                        );
                    }
                }
            }
        }
    }

    fn depends(from: &str, to: &str) -> Relation {
        Relation {
            from: id(from),
            to: id(to),
            kind: RelationKind::DependsOn,
        }
    }

    /// package:a ⊃ {a/one, a/two} beside the leaf package:b. a/one reaches
    /// package:b, package:b reaches a/two, and a/one reaches a/two within
    /// the frame.
    fn wired() -> (ArchitectureGraph, Vec<Relation>) {
        let (mut graph, _) = frame_of_leaves();
        graph
            .add_element(Element {
                id: id("package:b"),
                name: ElementName::new("package:b").unwrap(),
                kind: ElementKind::Package,
                fingerprint: None,
            })
            .unwrap();
        let edges = vec![
            depends("a/one", "package:b"),
            depends("package:b", "a/two"),
            depends("a/one", "a/two"),
        ];
        (graph, edges)
    }

    /// Which way one edge of that picture runs while `selected` stands
    /// selected.
    fn direction_of(from: &str, to: &str, selected: &str) -> Option<Direction> {
        let (view, edges) = wired();
        let selection = id(selected);
        focus_of(&Containment::of(&view), &edges, Selected::Node(&selection))
            .direction(&depends(from, to))
    }

    #[test]
    fn a_connection_leaving_the_selection_wears_the_dependency_color() {
        assert_eq!(
            edge_ink(
                INK,
                EdgeStatus::Existing,
                direction_of("a/one", "package:b", "package:a")
            ),
            DEPENDENCY,
            "the selection depends on what its connection reaches"
        );
    }

    #[test]
    fn a_connection_arriving_at_the_selection_wears_the_dependent_color() {
        assert_eq!(
            edge_ink(
                INK,
                EdgeStatus::Existing,
                direction_of("package:b", "a/two", "package:a")
            ),
            DEPENDENT
        );
    }

    #[test]
    fn a_connection_inside_the_selection_keeps_the_pictures_own_ink() {
        assert_eq!(
            edge_ink(
                INK,
                EdgeStatus::Existing,
                direction_of("a/one", "a/two", "package:a")
            ),
            INK,
            "internal wiring answers neither question the selection asks"
        );
    }

    #[test]
    fn nothing_takes_a_direction_while_nothing_is_selected() {
        assert_eq!(edge_ink(INK, EdgeStatus::Existing, None), INK);
    }

    #[test]
    fn a_planned_connection_keeps_its_plan_color_under_a_selection() {
        let leaving = direction_of("a/one", "package:b", "package:a");
        assert_eq!(leaving, Some(Direction::Outgoing));
        assert_eq!(edge_ink(INK, EdgeStatus::Severed, leaving), SEVERED);
        assert_eq!(edge_ink(INK, EdgeStatus::Drawn, leaving), DRAWN);
        assert_eq!(
            edge_ink(
                INK,
                EdgeStatus::PartiallySevered {
                    severed: 1,
                    total: 3
                },
                leaving
            ),
            DEPENDENCY,
            "a connection that mostly stays reads as the dependency it is, \
             and its red mark carries the rest"
        );
    }

    #[test]
    fn a_stroke_whose_connections_disagree_runs_no_way_at_all() {
        assert_eq!(
            agreed([Some(Direction::Outgoing), None]),
            Some(Direction::Outgoing),
            "a connection the selection ignores leaves the answer to the rest"
        );
        assert_eq!(
            agreed([Some(Direction::Outgoing), Some(Direction::Incoming)]),
            None
        );
        assert_eq!(agreed([None, None]), None);
    }

    #[test]
    fn the_lit_strokes_paint_after_the_boxes_while_a_selection_stands() {
        assert_eq!(
            lifted(
                &[Strength::Faded, Strength::Focused, Strength::Context],
                None,
                true
            ),
            vec![false, true, false],
            "the answer paints over the picture, and the rest stays under it"
        );
    }

    #[test]
    fn the_stroke_under_the_pointer_lifts_beside_the_lit_ones() {
        assert_eq!(
            lifted(&[Strength::Faded, Strength::Faded], Some(1), true),
            vec![false, true]
        );
    }

    #[test]
    fn nothing_lifts_while_nothing_is_selected() {
        assert_eq!(
            lifted(&[Strength::Focused, Strength::Focused], Some(0), false),
            vec![false, false],
            "with no question asked the strokes draw in one pass, under the boxes"
        );
    }

    /// A stroke in the busiest room the picture has: inside one top-level
    /// boundary, on a side it shares with a full crowd. Everything the
    /// reach and the crowd can hold back is held back from it.
    fn crowded(strength: Strength, asked_about: bool, selection_stands: bool) -> f32 {
        stroke_share(Scope::Intra, 16, strength, asked_about, selection_stands)
    }

    #[test]
    fn a_lit_stroke_paints_at_full_ink_while_a_selection_stands() {
        assert!(
            (crowded(Strength::Focused, false, true) - 1.0).abs() < 0.001,
            "the answer the reader asked for is not held back by the room \
             it runs in or the company it keeps"
        );
    }

    #[test]
    fn a_faded_stroke_keeps_its_crowd_under_the_same_selection() {
        let held_back = crowded(Strength::Faded, false, true);
        assert!(
            (held_back - INTRA * CROWD_INK * FADE).abs() < 0.001,
            "a stroke outside the answer still stands behind the strokes \
             that run alone: {held_back}"
        );
    }

    #[test]
    fn the_stroke_the_pointer_holds_keeps_all_of_its_ink() {
        assert!((crowded(Strength::Context, true, true) - CONTEXT).abs() < 0.001);
    }

    #[test]
    fn the_reach_and_the_crowd_speak_while_nothing_is_selected() {
        let alone = crowded(Strength::Focused, false, false);
        assert!(
            (alone - INTRA * CROWD_INK).abs() < 0.001,
            "with no question asked every stroke stands focused, and the \
             reach and the crowd are all that separates them: {alone}"
        );
    }

    #[test]
    fn a_stroke_that_runs_alone_keeps_all_of_its_ink() {
        assert!((crowd_ink(1) - 1.0).abs() < 0.001);
    }

    #[test]
    fn strokes_that_share_a_side_calm_down_as_they_gather() {
        let ink: Vec<f32> = [1, 4, 8, 16].into_iter().map(crowd_ink).collect();
        for pair in ink.windows(2) {
            assert!(pair[1] < pair[0], "more company, less ink: {ink:?}");
        }
        assert!(
            (ink[3] - CROWD_INK).abs() < 0.001,
            "a full side reaches the calmest the paint goes: {ink:?}"
        );
    }

    #[test]
    fn a_crowd_past_a_full_side_calms_no_further() {
        assert!((crowd_ink(40) - crowd_ink(16)).abs() < 0.001);
    }

    /// Everything one frame of the canvas reads about a picture, owned by
    /// the test so that a [`Content`] can borrow it as the shell's scene
    /// does.
    struct Picture {
        graph: ArchitectureGraph,
        layout: Layout,
        containment: Containment,
        edges: Vec<EdgeVisual>,
        nodes: BTreeMap<ElementId, NodeStatus>,
        renames: Renames,
    }

    impl Picture {
        /// The picture of [`wired`] with a box for every boundary in it.
        fn wired() -> Self {
            Self::drawing(wired().1)
        }

        fn drawing(edges: Vec<Relation>) -> Self {
            let (graph, _) = wired();
            let (_, mut layout) = frame_of_leaves();
            layout.rects.insert(
                id("package:b"),
                Rect::from_min_size(pos2(400.0, 0.0), vec2(120.0, 60.0)),
            );
            layout.leaves.push(id("package:b"));
            Self {
                containment: Containment::of(&graph),
                graph,
                layout,
                edges: edges
                    .into_iter()
                    .map(|relation| EdgeVisual {
                        relation,
                        status: EdgeStatus::Existing,
                        annotated: false,
                        weight: 1,
                    })
                    .collect(),
                nodes: BTreeMap::new(),
                renames: Renames::default(),
            }
        }

        /// The picture as the shell hands it over after `generation`
        /// rebuilds, with `selected` standing selected.
        fn at<'a>(&'a self, generation: u64, selected: Option<&'a ElementId>) -> Content<'a> {
            Content {
                generation,
                view: &self.graph,
                layout: &self.layout,
                containment: &self.containment,
                world: world_bounds(&self.layout),
                edges: &self.edges,
                nodes: &self.nodes,
                renames: &self.renames,
                selected_edge: None,
                selected_node: selected,
                draw_source: None,
            }
        }
    }

    /// A magnification at which every box of the fixture still reads, and
    /// one at which the frame's children have blurred past reading.
    const READS: f32 = 1.0;
    const BLURS: f32 = 0.2;

    fn vantage(scene: u64, zoom: f32) -> Vantage {
        Vantage { scene, zoom }
    }

    #[test]
    fn a_picture_that_did_not_move_draws_the_strokes_it_drew_before() {
        let picture = Picture::wired();
        let mut strokes = Strokes::default();
        strokes.refresh(&picture.at(1, None), vantage(1, READS));
        let routed = strokes.drawn.as_ptr();

        strokes.refresh(&picture.at(1, None), vantage(1, READS));

        assert!(
            !strokes.drawn.is_empty(),
            "the picture draws strokes at all"
        );
        assert_eq!(
            strokes.drawn.as_ptr(),
            routed,
            "the same scene at the same magnification is routed once"
        );
    }

    #[test]
    fn a_magnification_that_blurs_nothing_new_reroutes_nothing() {
        let picture = Picture::wired();
        let mut strokes = Strokes::default();
        strokes.refresh(&picture.at(1, None), vantage(1, READS));
        let routed = strokes.drawn.as_ptr();

        strokes.refresh(&picture.at(1, None), vantage(1, READS * 0.9));

        assert_eq!(
            strokes.drawn.as_ptr(),
            routed,
            "a camera that moved without changing what stands for what leaves \
             the strokes where they run"
        );
    }

    #[test]
    fn a_magnification_that_blurs_a_frame_lands_its_strokes_on_the_block() {
        let picture = Picture::wired();
        let mut strokes = Strokes::default();
        strokes.refresh(&picture.at(1, None), vantage(1, READS));
        assert_eq!(strokes.drawn[0].bundle.from, id("a/one"));

        strokes.refresh(&picture.at(1, None), vantage(1, BLURS));

        assert!(strokes.summary.hides(&id("a/one")));
        assert_eq!(
            strokes.drawn[0].bundle.from,
            id("package:a"),
            "an edge out of something hidden leaves the block that stands for it"
        );
    }

    #[test]
    fn a_selection_leaves_the_strokes_exactly_where_they_run() {
        let picture = Picture::wired();
        let mut strokes = Strokes::default();
        strokes.refresh(&picture.at(1, None), vantage(1, READS));
        let routed = strokes.drawn.as_ptr();

        strokes.refresh(&picture.at(1, Some(&id("package:a"))), vantage(1, READS));

        assert_eq!(
            strokes.drawn.as_ptr(),
            routed,
            "a selection changes how strongly a stroke paints, never where it runs"
        );
    }

    #[test]
    fn a_rebuilt_scene_routes_its_strokes_anew() {
        let before = Picture::wired();
        let after = Picture::drawing(vec![depends("a/one", "package:b")]);
        let mut strokes = Strokes::default();
        strokes.refresh(&before.at(1, None), vantage(1, READS));
        assert_eq!(strokes.drawn.len(), 3);

        strokes.refresh(&after.at(2, None), vantage(2, READS));

        assert_eq!(
            strokes.drawn.len(),
            1,
            "a new scene is a new picture, whatever the magnification"
        );
    }

    #[test]
    fn the_screen_line_of_a_stroke_is_its_world_line_in_front_of_the_camera() {
        let picture = Picture::wired();
        let mut strokes = Strokes::default();
        strokes.refresh(&picture.at(1, None), vantage(1, READS));
        let camera =
            TSTransform::from_translation(vec2(30.0, -12.0)) * TSTransform::from_scaling(2.0);

        strokes.project(camera);

        let stroke = &strokes.drawn[0];
        let curve = stroke.curve.as_ref().expect("the stroke has a route");
        assert_eq!(stroke.screen.len(), curve.points.len());
        for (world, screen) in curve.points.iter().zip(&stroke.screen) {
            assert_eq!(*screen, camera.mul_pos(*world));
        }
    }

    #[test]
    fn placing_the_strokes_anew_writes_into_the_buffers_of_the_last_frame() {
        let picture = Picture::wired();
        let mut strokes = Strokes::default();
        strokes.refresh(&picture.at(1, None), vantage(1, READS));
        strokes.project(TSTransform::from_scaling(1.0));
        let buffer = strokes.drawn[0].screen.as_ptr();

        strokes.project(TSTransform::from_scaling(3.0));

        assert_eq!(
            strokes.drawn[0].screen.as_ptr(),
            buffer,
            "a camera that moved allocates nothing: the points are overwritten"
        );
    }

    /// One stroke that runs along the given screen points and nothing else.
    fn stroke_along(points: Vec<Pos2>) -> DrawnEdge {
        DrawnEdge {
            bundle: Bundle {
                from: id("a/one"),
                to: id("package:b"),
                members: vec![0],
                lead: 0,
                weight: 1,
            },
            route: None,
            curve: Some(Curve {
                bounds: Rect::from_points(&points),
                points: points.clone(),
            }),
            screen: points,
        }
    }

    #[test]
    fn the_pointer_catches_a_diagonal_stroke_beside_its_middle() {
        let strokes = vec![stroke_along(vec![
            pos2(0.0, 0.0),
            pos2(50.0, 50.0),
            pos2(100.0, 100.0),
        ])];

        assert_eq!(
            nearest_curve(&strokes, pos2(52.0, 50.0), TSTransform::IDENTITY),
            Some(0),
            "a run across its own box is caught beside the line, not beside the box"
        );
        assert_eq!(
            nearest_curve(&strokes, pos2(95.0, 5.0), TSTransform::IDENTITY),
            None,
            "the far corner of the box is nowhere near the line that crosses it"
        );
    }

    #[test]
    fn no_stroke_within_reach_of_the_pointer_is_ruled_out_by_its_box() {
        let bounds = Rect::from_min_size(pos2(100.0, 100.0), vec2(200.0, 40.0));
        for beside in [
            pos2(100.0 - EDGE_REACH, 120.0),
            pos2(300.0 + EDGE_REACH, 120.0),
            pos2(200.0, 100.0 - EDGE_REACH),
            pos2(200.0, 140.0 + EDGE_REACH),
            pos2(200.0, 120.0),
        ] {
            assert!(
                within_reach(bounds, beside),
                "a line touching {bounds:?} may run within reach of {beside:?}"
            );
        }
        assert!(
            !within_reach(bounds, pos2(100.0 - EDGE_REACH - 1.0, 120.0)),
            "past the reach no point of the box can be caught"
        );
    }

    /// A canvas of the usual size, at the origin.
    const CANVAS: Rect = Rect {
        min: Pos2 { x: 0.0, y: 0.0 },
        max: Pos2 { x: 800.0, y: 600.0 },
    };

    #[test]
    fn every_mark_that_meets_the_canvas_is_painted() {
        for meeting in [
            Rect::from_min_size(pos2(-40.0, 300.0), vec2(60.0, 20.0)),
            Rect::from_min_size(pos2(780.0, 590.0), vec2(100.0, 100.0)),
            Rect::from_min_size(pos2(400.0, -20.0), vec2(40.0, 40.0)),
            Rect::from_min_size(pos2(-1000.0, -1000.0), vec2(4000.0, 4000.0)),
            Rect::from_min_size(pos2(0.0, 0.0), vec2(0.0, 0.0)),
        ] {
            assert!(
                on_screen(meeting, CANVAS),
                "{meeting:?} puts ink on the canvas"
            );
        }
    }

    #[test]
    fn a_mark_just_off_the_canvas_still_paints_the_ink_it_hangs_over_the_edge() {
        assert!(
            on_screen(
                Rect::from_min_size(pos2(-10.0, 300.0), vec2(5.0, 20.0)),
                CANVAS
            ),
            "a box past the edge still strokes a border the reader sees"
        );
    }

    #[test]
    fn a_mark_far_from_the_canvas_is_never_built() {
        assert!(!on_screen(
            Rect::from_min_size(pos2(-500.0, 300.0), vec2(100.0, 20.0)),
            CANVAS
        ));
        assert!(!on_screen(
            Rect::from_min_size(pos2(200.0, 900.0), vec2(100.0, 20.0)),
            CANVAS
        ));
    }
}
