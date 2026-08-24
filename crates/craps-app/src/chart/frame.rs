// Copyright (c) 2026 Jim Schreckengast
// SPDX-License-Identifier: MIT

//! The universal chart container and paint context.
//!
//! Every chart lives in a [`ChartFrame`]; none render bare. The frame owns
//! the chrome (title, story slot, STALE/PARTIAL badge, provenance corner)
//! and the input surface (probe with 80 ms hover-in, up to three pins keyed
//! by data-x, x-range brush, ⌘-scroll zoom, ⇧-scroll pan, double-click
//! reset). The body paints through [`ChartCx`] layers; layers tessellate in
//! a fixed order, so uncertainty CANNOT paint over point estimates —
//! paint-order-as-API (Honesty rule 7 enforced by types).

use egui::text::LayoutJob;
use egui::{
    Align2, Color32, FontId, Mesh, Pos2, Rect, Response, Sense, Shape, Stroke, StrokeKind, Ui,
};

use super::probe::ProbePins;
use super::scale::LinearScale;
use crate::ui::theme::{self, type_scale, Theme};

/// Paint order, bottom to top. Ribbons (uncertainty) can only be emitted to
/// a layer tessellated before estimates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layer {
    /// CI ribbons and bands — always beneath their estimates.
    Ribbon = 0,
    /// Gridlines, axes, censoring hatches.
    Grid = 1,
    /// The data itself: bars, curves, dots.
    Data = 2,
    /// Point estimates and analytic markers (means, medians, House Lines).
    Estimate = 3,
    /// Labels, droplines, callouts.
    Annotation = 4,
    /// Probe crosshair, pins, brush fill.
    Overlay = 5,
}

/// Trust badges. STALE: settings changed since this run. PARTIAL: shown
/// with its achieved n.
#[derive(Clone, Debug, PartialEq)]
pub enum Badge {
    Stale,
    Partial(String),
}

/// Per-chart persistent interaction state (egui temp memory).
#[derive(Clone, Default)]
pub struct FrameState {
    /// The run this state belongs to; a new run resets zoom/pins/brush so
    /// the chart recenters on the fresh distribution.
    pub run_key: u64,
    /// Zoomed x-window in data space.
    pub window: Option<(f64, f64)>,
    /// Pinned probes, data-x, at most three (spec §7).
    pub pins: Vec<f64>,
    /// Probe x from last frame, data space, after the 80 ms hover-in delay.
    pub probe: Option<f64>,
    /// When the pointer entered the plot (for the hover-in delay).
    pub hover_since: Option<f64>,
    /// Brush anchor while dragging, data space.
    pub brush_anchor: Option<f64>,
    /// Completed brush from last frame (consumed by zoom on release).
    pub brush_live: Option<(f64, f64)>,
}

/// The paint context handed to a chart body.
pub struct ChartCx<'a> {
    pub theme: &'a Theme,
    /// The plot area in screen space (axes margins included — bodies inset
    /// via [`super::axis`] helpers).
    pub rect: Rect,
    pub response: &'a Response,
    pub x: LinearScale,
    pub y: LinearScale,
    /// Interaction state as of the START of this frame (immediate-mode: one
    /// frame of lag, imperceptible; pins/zoom survive because they live in
    /// data space).
    pub state: FrameState,
    /// A body sets this while it owns the pointer (e.g. dragging the budget
    /// line) so the frame's brush stands down this frame.
    pub suppress_brush: bool,
    /// The un-zoomed data domain, recorded by [`Self::set_x_domain`]; the
    /// frame clamps panning to it and snaps zoom-out back to the full view.
    pub full_x: Option<(f64, f64)>,
    /// Where frame chrome anchored to the plot's bottom (the pan hint)
    /// must stop: bodies that reserve a lower band (Stake's dot field) set
    /// this to the band's top even if they restore `rect` afterwards.
    pub hint_bottom: Option<f32>,
    /// The x-axis tick formatter, recorded by [`super::axis::x_axis`]; the
    /// frame reuses it to print pin values and the Δ between pins.
    pub x_fmt: Option<std::sync::Arc<dyn Fn(f64) -> String>>,
    layers: [Vec<Shape>; 6],
    ctx: egui::Context,
    galleys: Vec<(Pos2, std::sync::Arc<egui::Galley>, Layer)>,
}

impl<'a> ChartCx<'a> {
    /// Set the x scale from a full-data domain; an active zoom window
    /// overrides it. Range spans the plot rect.
    pub fn set_x_domain(&mut self, d0: f64, d1: f64) {
        self.full_x = Some((d0, d1));
        let (d0, d1) = self.state.window.unwrap_or((d0, d1));
        self.x = LinearScale::new(
            (d0, d1),
            (
                self.rect.left() + super::axis::MARGIN_LEFT,
                self.rect.right() - 8.0,
            ),
        );
    }

    /// Linear y over the plot rect (top-down), inset for the x-label strip
    /// below and the twin-axis/label band above.
    pub fn set_y_domain(&mut self, d0: f64, d1: f64) {
        self.y = LinearScale::new(
            (d0, d1),
            (
                self.rect.bottom() - super::axis::MARGIN_BOTTOM,
                self.rect.top() + 14.0,
            ),
        );
    }

    /// Opt-in, labeled log-y (the caller draws the "log" label).
    pub fn set_y_log(&mut self, max: f64) {
        self.y = LinearScale::log_y(
            max,
            (
                self.rect.bottom() - super::axis::MARGIN_BOTTOM,
                self.rect.top() + 14.0,
            ),
        );
    }

    /// The y pixel of the plot floor (domain minimum) — bar baselines,
    /// dropline feet.
    pub fn baseline(&self) -> f32 {
        self.y.r0
    }

    pub fn xy(&self, x: f64, y: f64) -> Pos2 {
        Pos2::new(self.x.to_screen(x), self.y.to_screen(y))
    }

    /// The probe's data-x, if the pointer has dwelt ≥ 80 ms over the plot.
    pub fn probe(&self) -> Option<f64> {
        self.state.probe
    }

    pub fn pins(&self) -> &[f64] {
        &self.state.pins
    }

    /// The active brush (data-x range, ordered), while dragging.
    pub fn brush(&self) -> Option<(f64, f64)> {
        self.state
            .brush_live
            .map(|(a, b)| if a <= b { (a, b) } else { (b, a) })
    }

    // ----- layer paint helpers -------------------------------------------

    pub fn shape(&mut self, layer: Layer, shape: Shape) {
        self.layers[layer as usize].push(shape);
    }

    pub fn line(&mut self, layer: Layer, points: Vec<Pos2>, stroke: Stroke) {
        self.shape(layer, Shape::line(points, stroke));
    }

    pub fn hline(&mut self, layer: Layer, y: f32, stroke: Stroke) {
        self.line(
            layer,
            vec![
                Pos2::new(self.rect.left(), y),
                Pos2::new(self.rect.right(), y),
            ],
            stroke,
        );
    }

    pub fn vline(&mut self, layer: Layer, x: f32, stroke: Stroke) {
        self.line(
            layer,
            vec![
                Pos2::new(x, self.rect.top()),
                Pos2::new(x, self.rect.bottom()),
            ],
            stroke,
        );
    }

    pub fn dashed_vline(&mut self, layer: Layer, x: f32, color: Color32) {
        let mut y = self.rect.top();
        let mut shapes = Vec::new();
        while y < self.rect.bottom() {
            let y2 = (y + 4.0).min(self.rect.bottom());
            shapes.push(Shape::line_segment(
                [Pos2::new(x, y), Pos2::new(x, y2)],
                Stroke::new(1.0, color),
            ));
            y += 7.0;
        }
        self.layers[layer as usize].extend(shapes);
    }

    pub fn rect_filled(&mut self, layer: Layer, rect: Rect, fill: Color32) {
        self.shape(layer, Shape::rect_filled(rect, 0.0, fill));
    }

    pub fn mesh(&mut self, layer: Layer, mesh: Mesh) {
        self.shape(layer, Shape::mesh(mesh));
    }

    pub fn circle(&mut self, layer: Layer, center: Pos2, r: f32, fill: Color32) {
        self.shape(layer, Shape::circle_filled(center, r, fill));
    }

    /// Lay out and queue text. `anchor` positions the text box relative to
    /// `pos` (e.g. LEFT_TOP puts pos at the box's top-left).
    pub fn text(
        &mut self,
        layer: Layer,
        pos: Pos2,
        anchor: Align2,
        text: impl ToString,
        font: FontId,
        color: Color32,
    ) {
        let galley = self.ctx.fonts_mut(|f| {
            f.layout_job(LayoutJob::simple_singleline(text.to_string(), font, color))
        });
        let rect = anchor.anchor_size(pos, galley.size());
        self.galleys.push((rect.min, galley, layer));
    }

    /// Laid-out width of `text` in `font`, for collision-aware label rows.
    pub fn text_width(&mut self, text: &str, font: FontId) -> f32 {
        self.ctx
            .fonts_mut(|f| {
                f.layout_job(LayoutJob::simple_singleline(
                    text.to_owned(),
                    font,
                    Color32::WHITE,
                ))
            })
            .size()
            .x
    }

    /// Text over a snug pill of ground color, for labels that sit on top of
    /// data marks. Within a layer all shapes paint before any text, so the
    /// pill can never cover another label on the same layer.
    #[allow(clippy::too_many_arguments)]
    pub fn text_pilled(
        &mut self,
        layer: Layer,
        pos: Pos2,
        anchor: Align2,
        text: impl ToString,
        font: FontId,
        color: Color32,
        pill: Color32,
    ) {
        let galley = self.ctx.fonts_mut(|f| {
            f.layout_job(LayoutJob::simple_singleline(text.to_string(), font, color))
        });
        let rect = anchor.anchor_size(pos, galley.size());
        self.shape(
            layer,
            Shape::rect_filled(rect.expand2(egui::vec2(3.0, 1.0)), 3.0, pill),
        );
        self.galleys.push((rect.min, galley, layer));
    }
}

/// The universal chart container.
pub struct ChartFrame<'a> {
    pub id: egui::Id,
    pub title: &'a str,
    /// Story-register lead sentence (19 px), optional.
    pub story: Option<String>,
    /// Provenance corner: `n · seed · scenario hash`, 11 px mono.
    pub provenance: String,
    pub badge: Option<Badge>,
    pub height: f32,
    /// Disable brush/zoom/pins (e.g. small multiples panels).
    pub interactive: bool,
    /// Non-interactive frames can still sense plain clicks (the Explorer
    /// strip selects dots without owning brush/zoom).
    pub clickable: bool,
    /// Identity of the run behind this chart (e.g. fingerprint ^ seed).
    /// When it changes, zoom/pins/brush reset so the view recenters on the
    /// new distribution. 0 (the default) never resets.
    pub run_key: u64,
}

impl<'a> ChartFrame<'a> {
    pub fn new(id: egui::Id, title: &'a str) -> Self {
        Self {
            id,
            title,
            story: None,
            provenance: String::new(),
            badge: None,
            height: 320.0,
            interactive: true,
            clickable: false,
            run_key: 0,
        }
    }

    pub fn clickable(mut self, on: bool) -> Self {
        self.clickable = on;
        self
    }

    pub fn run_key(mut self, key: u64) -> Self {
        self.run_key = key;
        self
    }

    pub fn story(mut self, s: impl Into<String>) -> Self {
        self.story = Some(s.into());
        self
    }

    pub fn provenance(mut self, p: impl Into<String>) -> Self {
        self.provenance = p.into();
        self
    }

    pub fn badge(mut self, b: Option<Badge>) -> Self {
        self.badge = b;
        self
    }

    pub fn height(mut self, h: f32) -> Self {
        self.height = h;
        self
    }

    pub fn interactive(mut self, on: bool) -> Self {
        self.interactive = on;
        self
    }

    /// Render chrome + body. Returns the plot response for outer wiring.
    pub fn show(self, ui: &mut Ui, t: &Theme, body: impl FnOnce(&mut ChartCx<'_>)) -> Response {
        let card = egui::Frame::NONE
            .fill(t.surface)
            .stroke(Stroke::new(1.0, t.hairline))
            .corner_radius(8)
            .inner_margin(16.0);
        let mut plot_response: Option<Response> = None;
        card.show(ui, |ui| {
            // Title row: title left, badge right.
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new(self.title)
                        .font(FontId::new(type_scale::SECTION, theme::sans_semibold()))
                        .color(t.ink),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    // The export affordance (spec §7): every frame can leave
                    // as a 2× PNG with its provenance baked in.
                    if self.interactive {
                        let export = crate::ui::icons::button(
                            ui,
                            crate::ui::icons::Icon::ExportImage,
                            t.ink2,
                            t.blue,
                            "export · ⌘⇧C copies · ⌘⇧S saves",
                        );
                        if export.clicked() {
                            ui.ctx().data_mut(|d| {
                                d.insert_temp(egui::Id::new("export_request"), (self.id, false));
                            });
                        }
                        // Pin indicator: numbered pins persist across morphs;
                        // clicking the glyph clears them.
                        let pins = ui
                            .ctx()
                            .data_mut(|d| d.get_temp::<FrameState>(self.id))
                            .map(|s| s.pins.len())
                            .unwrap_or(0);
                        if pins > 0 {
                            let clear = crate::ui::icons::button(
                                ui,
                                crate::ui::icons::Icon::Pin,
                                t.blue,
                                t.blue,
                                &format!(
                                    "{pins} pinned reference(s) — pins hold a position for \
                                     comparison, show the Δ between the last two, and print \
                                     in exports · click a pin (or ⌫) removes · click here \
                                     clears all"
                                ),
                            );
                            if clear.clicked() {
                                ui.ctx().data_mut(|d| {
                                    if let Some(mut s) = d.get_temp::<FrameState>(self.id) {
                                        s.pins.clear();
                                        d.insert_temp(self.id, s);
                                    }
                                });
                            }
                        }
                    }
                    if let Some(b) = &self.badge {
                        let (text, _hover) = match b {
                            Badge::Stale => (
                                "STALE".to_owned(),
                                "settings changed since this run — Space re-runs",
                            ),
                            Badge::Partial(n) => (format!("PARTIAL · {n}"), ""),
                        };
                        let label = egui::RichText::new(text)
                            .font(FontId::new(type_scale::CAPTION, theme::mono()))
                            .color(t.amber);
                        let resp = ui.add(egui::Label::new(label).sense(Sense::hover()));
                        let padded = resp.rect.expand2(egui::vec2(6.0, 2.0));
                        ui.painter().rect_stroke(
                            padded,
                            3.0,
                            Stroke::new(1.0, t.amber),
                            StrokeKind::Outside,
                        );
                        if let Badge::Stale = b {
                            resp.on_hover_text("settings changed since this run — Space re-runs");
                        }
                    }
                });
            });
            if let Some(story) = &self.story {
                ui.add_space(2.0);
                ui.label(
                    egui::RichText::new(story)
                        .font(FontId::new(type_scale::STORY, theme::sans_medium()))
                        .color(t.ink),
                );
            }
            ui.add_space(8.0);

            // Plot area.
            let width = ui.available_width();
            let (rect, response) = ui.allocate_exact_size(
                egui::vec2(width, self.height),
                if self.interactive {
                    Sense::click_and_drag()
                } else if self.clickable {
                    Sense::click()
                } else {
                    Sense::hover()
                },
            );

            // Screen readers get the chart's Story sentence as its label
            // (spec §11): the summary IS the accessible name.
            let accessible = match &self.story {
                Some(story) => format!("{} — {}", self.title, story),
                None => self.title.to_owned(),
            };
            response.widget_info(|| {
                egui::WidgetInfo::labeled(egui::WidgetType::Other, true, accessible.clone())
            });

            let mut state: FrameState = ui
                .ctx()
                .data_mut(|d| d.get_temp::<FrameState>(self.id))
                .unwrap_or_default();

            // A new run means a new distribution: drop zoom, pins, and brush
            // so the chart recenters instead of staring at stale coordinates.
            if self.run_key != 0 && state.run_key != self.run_key {
                state = FrameState {
                    run_key: self.run_key,
                    ..FrameState::default()
                };
            }

            // Probe hover-in delay bookkeeping happens pre-body so the body
            // reads a stable state snapshot.
            let now = ui.input(|i| i.time);
            let hover = response.hover_pos().filter(|p| rect.contains(*p));
            if hover.is_some() {
                if state.hover_since.is_none() {
                    state.hover_since = Some(now);
                }
                // The 80 ms dwell must fire even when the app is otherwise
                // idle: schedule the frame that reveals the probe.
                if state.probe.is_none() {
                    ui.ctx()
                        .request_repaint_after(std::time::Duration::from_millis(85));
                }
            } else {
                state.hover_since = None;
                state.probe = None;
            }

            let mut cx = ChartCx {
                theme: t,
                rect,
                response: &response,
                x: LinearScale::new((0.0, 1.0), (rect.left(), rect.right())),
                y: LinearScale::new((0.0, 1.0), (rect.bottom(), rect.top())),
                state: state.clone(),
                suppress_brush: false,
                full_x: None,
                hint_bottom: None,
                x_fmt: None,
                layers: Default::default(),
                ctx: ui.ctx().clone(),
                galleys: Vec::new(),
            };
            body(&mut cx);
            let x_scale = cx.x;
            let suppress_brush = cx.suppress_brush;
            let full_x = cx.full_x;
            let x_fmt = cx.x_fmt.take();
            // Bodies may shrink their rect (e.g. Stake reserves a dot-field
            // band); frame chrome placed near the bottom must respect that.
            let body_rect = cx.rect;
            let hint_bottom = cx.hint_bottom.unwrap_or(body_rect.bottom());

            // ---- input, using the body's final scales -------------------
            if self.interactive {
                if let Some(p) = hover {
                    // Probe after the 80 ms dwell. The body painted this
                    // frame from the pre-dwell snapshot, so the reveal frame
                    // repaints immediately — readouts land with the
                    // crosshair, not one idle frame later.
                    if now - state.hover_since.unwrap_or(now) >= 0.08 {
                        if state.probe.is_none() {
                            ui.ctx().request_repaint();
                        }
                        state.probe = Some(x_scale.from_screen(p.x));
                    }
                    // ⌘-scroll zooms about the cursor; ⇧-scroll (or a plain
                    // horizontal swipe while zoomed) pans.
                    let (zoom, scroll_x, shift) =
                        ui.input(|i| (i.zoom_delta(), i.smooth_scroll_delta.x, i.modifiers.shift));
                    let (mut d0, mut d1) = state.window.unwrap_or((x_scale.d0, x_scale.d1));
                    if (zoom - 1.0).abs() > 1e-3 {
                        let cx_data = x_scale.from_screen(p.x);
                        let k = 1.0 / zoom as f64;
                        d0 = cx_data + (d0 - cx_data) * k;
                        d1 = cx_data + (d1 - cx_data) * k;
                        // The window lives inside the full data domain:
                        // zooming out to (or past) it snaps to the whole
                        // view, and an off-center zoom-out slides back onto
                        // the data instead of stranding a dead margin the
                        // pan clamp would then refuse to move.
                        state.window = match full_x {
                            Some((f0, f1)) if d1 - d0 >= f1 - f0 => None,
                            Some((f0, f1)) => {
                                let shift_back = if d0 < f0 {
                                    f0 - d0
                                } else if d1 > f1 {
                                    f1 - d1
                                } else {
                                    0.0
                                };
                                Some((d0 + shift_back, d1 + shift_back))
                            }
                            None => Some((d0, d1)),
                        };
                    } else if scroll_x.abs() > 0.0 && (shift || state.window.is_some()) {
                        let span = d1 - d0;
                        let mut dx = -scroll_x as f64 / rect.width() as f64 * span;
                        // Panning stays on the data: the window never leaves
                        // the full domain.
                        if let Some((f0, f1)) = full_x {
                            if span < f1 - f0 {
                                dx = dx.clamp(f0 - d0, f1 - d1);
                            } else {
                                dx = 0.0;
                            }
                        }
                        if dx != 0.0 {
                            state.window = Some((d0 + dx, d1 + dx));
                        }
                    }
                }
                if response.double_clicked() {
                    state.window = None;
                    state.brush_anchor = None;
                    state.brush_live = None;
                }
                if ui.input(|i| i.key_pressed(egui::Key::Escape)) && response.hovered() {
                    state.brush_anchor = None;
                    state.brush_live = None;
                }

                // Brush: plain drag across the plot (unless the body owns
                // the pointer, e.g. a budget-line drag).
                if suppress_brush {
                    state.brush_anchor = None;
                    state.brush_live = None;
                } else if response.drag_started() {
                    if let Some(p) = response.interact_pointer_pos() {
                        state.brush_anchor = Some(x_scale.from_screen(p.x));
                    }
                }
                if response.dragged() {
                    if let (Some(a), Some(p)) =
                        (state.brush_anchor, response.interact_pointer_pos())
                    {
                        state.brush_live = Some((a, x_scale.from_screen(p.x)));
                    }
                }
                if response.drag_stopped() {
                    // Release zooms to the brushed range (spec §8) when it
                    // is a meaningful span; a stray wiggle is ignored.
                    if let Some((a, b)) = state.brush_live.take() {
                        let (mut lo, mut hi) = if a <= b { (a, b) } else { (b, a) };
                        // A drag can extrapolate past the plot edge; the
                        // window still lives inside the full data domain.
                        if let Some((f0, f1)) = full_x {
                            lo = lo.max(f0);
                            hi = hi.min(f1);
                        }
                        // hi > lo also rejects a drag that lay entirely
                        // outside the domain (the clamp would invert it).
                        if hi > lo && (x_scale.to_screen(hi) - x_scale.to_screen(lo)).abs() > 8.0 {
                            state.window = Some((lo, hi));
                        }
                    }
                    state.brush_anchor = None;
                }

                // Click (not drag) pins the probe; clicking near a pin
                // removes it; Backspace removes the most recent.
                if response.clicked() {
                    if let Some(p) = response.interact_pointer_pos() {
                        let x_data = x_scale.from_screen(p.x);
                        let near = state
                            .pins
                            .iter()
                            .position(|&px| (x_scale.to_screen(px) - p.x).abs() < 6.0);
                        match near {
                            Some(i) => {
                                state.pins.remove(i);
                            }
                            None if state.pins.len() < 3 => state.pins.push(x_data),
                            None => {}
                        }
                    }
                }
                if response.hovered() && ui.input(|i| i.key_pressed(egui::Key::Backspace)) {
                    state.pins.pop();
                }
            }

            // ---- flush layers in order ----------------------------------
            let painter = ui.painter_at(rect);
            let export_req: Option<(egui::Id, bool)> = ui
                .ctx()
                .data(|d| d.get_temp(egui::Id::new("export_request")));
            let capturing = export_req.map(|(id, _)| id == self.id).unwrap_or(false);
            let mut export_shapes: Vec<Shape> = Vec::new();
            let ChartCx {
                layers, galleys, ..
            } = cx;
            for (li, layer) in layers.into_iter().enumerate() {
                if capturing {
                    export_shapes.extend(layer.iter().cloned());
                }
                painter.extend(layer);
                for (pos, galley, glayer) in &galleys {
                    if *glayer as usize == li {
                        if capturing {
                            export_shapes.push(Shape::galley(*pos, galley.clone(), t.ink));
                        }
                        painter.galley(*pos, galley.clone(), t.ink);
                    }
                }
            }

            // Frame-drawn overlays: pins and probe crosshair — one shape
            // list feeds the screen and, when capturing, the export.
            let overlay_shapes = ProbePins {
                pins: &state.pins,
                probe: state.probe,
                brush: state.brush_live,
                x_fmt: x_fmt.as_deref(),
            }
            .shapes(ui.ctx(), rect, &x_scale, t);
            if capturing {
                export_shapes.extend(overlay_shapes.iter().cloned());
            }
            painter.extend(overlay_shapes);

            // Wayfinding while zoomed: how to pan and how to get back.
            // Screen-only — exports never carry interaction hints.
            if self.interactive && state.window.is_some() && response.hovered() {
                let galley = ui.ctx().fonts_mut(|f| {
                    f.layout_job(LayoutJob::simple_singleline(
                        "⇧-scroll pans · double-click resets".to_owned(),
                        FontId::new(type_scale::CAPTION, theme::sans()),
                        t.ink2,
                    ))
                });
                let pos = Pos2::new(
                    body_rect.right() - 8.0 - galley.size().x,
                    hint_bottom - super::axis::MARGIN_BOTTOM - galley.size().y - 4.0,
                );
                painter.rect_filled(
                    Rect::from_min_size(pos, galley.size()).expand2(egui::vec2(4.0, 2.0)),
                    3.0,
                    t.pill(),
                );
                painter.galley(pos, galley, t.ink2);
            }

            // Register this frame for the Principle-1 mount check (heroes
            // assert their chart was on screen).
            ui.ctx().data_mut(|d| {
                let key = egui::Id::new("mounted_charts");
                let mut v: Vec<egui::Id> = d.get_temp(key).unwrap_or_default();
                v.push(self.id);
                d.insert_temp(key, v);
            });

            if response.hovered() {
                ui.ctx()
                    .data_mut(|d| d.insert_temp(egui::Id::new("last_hovered_chart"), self.id));
            }

            // Export capture: hand the app the frame's own shape list plus
            // chrome baked as text shapes — provenance survives cropping.
            if capturing {
                let save = export_req.map(|(_, s)| s).unwrap_or(false);
                let sentence: String = ui
                    .ctx()
                    .data(|d| d.get_temp(egui::Id::new("scenario_sentence")))
                    .unwrap_or_default();
                let mut chrome =
                    |text: &str, dy: f32, size: f32, fam: egui::FontFamily, color: Color32| {
                        if text.is_empty() {
                            return;
                        }
                        let galley = ui.ctx().fonts_mut(|f| {
                            f.layout_job(LayoutJob::simple_singleline(
                                text.to_owned(),
                                FontId::new(size, fam),
                                color,
                            ))
                        });
                        export_shapes.push(Shape::galley(
                            Pos2::new(rect.left(), rect.top() + dy),
                            galley,
                            color,
                        ));
                    };
                chrome(
                    self.title,
                    -50.0,
                    type_scale::SECTION,
                    theme::sans_semibold(),
                    t.ink,
                );
                if let Some(story) = &self.story {
                    chrome(story, -30.0, type_scale::BODY, theme::sans_medium(), t.ink);
                }
                chrome(
                    &self.provenance,
                    rect.height() + 6.0,
                    type_scale::CAPTION,
                    theme::mono(),
                    t.ink2,
                );
                chrome(
                    &sentence,
                    rect.height() + 20.0,
                    type_scale::CAPTION,
                    theme::sans(),
                    t.ink2,
                );
                let bundle = super::export::ExportBundle {
                    shapes: export_shapes,
                    rect: Rect::from_min_max(
                        rect.min - egui::vec2(0.0, 56.0),
                        rect.max + egui::vec2(0.0, 36.0),
                    ),
                    background: t.surface,
                };
                ui.ctx().data_mut(|d| {
                    d.remove::<(egui::Id, bool)>(egui::Id::new("export_request"));
                    d.insert_temp(
                        egui::Id::new("captured_export"),
                        std::sync::Arc::new((bundle, save, self.title.to_owned())),
                    );
                });
            }

            ui.ctx().data_mut(|d| d.insert_temp(self.id, state));

            // Provenance corner.
            if !self.provenance.is_empty() {
                ui.add_space(6.0);
                ui.label(
                    egui::RichText::new(&self.provenance)
                        .font(FontId::new(type_scale::CAPTION, theme::mono()))
                        .color(t.ink2),
                );
            }
            plot_response = Some(response);
        });
        plot_response.expect("frame body ran")
    }
}
