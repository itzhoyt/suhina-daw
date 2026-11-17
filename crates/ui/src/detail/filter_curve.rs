// Copyright (C) 2026 The Suhina contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The Filter's response display (the SDK's `Widget::FilterCurve`, docs/spec/filter.md › The
//! panel), drawn on the UI thread from the device's own maths (`suhina_device_filter::curve`), so
//! the curve on screen is the filter that sounds. It shows the filter as its parameters set it;
//! while the LFO moves it, the range it sweeps as a shaded band and the filter where the LFO has
//! it now (the device's readouts) as the bright curve. A dot at the frequency drags sideways for
//! the frequency and up and down for the resonance, or for the gain of the peak and the shelves.
//! Every edit is a `device.setParam` command, addressed by the device's full path, so automation
//! and undo see it like any other edit.

use egui::{Align2, Color32, Pos2, Rect, Sense, Stroke, Vec2};
use serde_json::json;
use suhina_device_api::DEFAULT_SAMPLE_RATE;
use suhina_device_filter::curve::{FREQUENCY_RANGE, GAIN_TYPES, Plan, Settings, TYPE_NAMES};
use suhina_device_filter::readout;
use suhina_device_sdk::ui::READOUT_IDS;
use suhina_engine::model::TrackId;

use crate::detail::detail_view::{DeviceAt, GraphParameter};
use crate::kit::theme;
use crate::kit::widgets::{Action, fixed};

/// The display's level range in dB: the top edge and the bottom one. A full resonance stands
/// about 26 dB high; the cuts fall away below the floor.
const DB: (f64, f64) = (30.0, -42.0);
/// The gain parameter's range, which the dot's height maps to for the peak and the shelves.
const GAIN: (f64, f64) = (-24.0, 24.0);
/// The curve's points across the display.
const POINTS: usize = 160;
/// The LFO positions the shaded band is drawn from, -1 to 1.
const SWEEP_STEPS: usize = 9;
/// The rate the response is drawn at; the device runs at the project's, and only the bend
/// near this Nyquist differs.
const DRAWN_RATE: f64 = DEFAULT_SAMPLE_RATE;
/// How far in from the display's top and bottom the dot travels.
const DOT_MARGIN: f32 = 8.0;

/// The panel's values the display reads.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Values {
    settings: Settings,
    lfo_amount: f64,
    lfo_morph: f64,
    lfo_phase: f64,
    /// Where the LFO has the frequency and the morph now, on each side, from the readouts.
    live: Option<[(f64, f64); 2]>,
}

/// The parameter's value, or 0 when the panel has no such id.
fn value(parameters: &[GraphParameter], id: &str) -> f64 {
    parameters.iter().find(|p| p.id == id).map_or(0.0, |p| p.value)
}

fn parameter<'a>(parameters: &'a [GraphParameter], id: &str) -> Option<&'a GraphParameter> {
    parameters.iter().find(|p| p.id == id)
}

/// The readout `index`, when the device has handed its panel any.
fn readout_value(parameters: &[GraphParameter], index: usize) -> Option<f64> {
    let id = READOUT_IDS.get(index)?;
    parameters.iter().find(|p| p.id == *id).map(|p| p.value)
}

fn values(parameters: &[GraphParameter]) -> Values {
    let kind = (value(parameters, "type").round().max(0.0) as usize).min(TYPE_NAMES.len() - 1);
    let settings = Settings {
        kind,
        frequency: value(parameters, "frequency").clamp(FREQUENCY_RANGE.0, FREQUENCY_RANGE.1),
        resonance: value(parameters, "resonance").clamp(0.0, 1.0),
        gain: value(parameters, "gain").clamp(GAIN.0, GAIN.1),
        morph: value(parameters, "morph").clamp(0.0, 1.0),
    };
    let (lfo_amount, lfo_morph) = (value(parameters, "lfoAmount"), value(parameters, "lfoMorph"));
    let live = [(readout::FREQUENCY_LEFT, readout::MORPH_LEFT), (readout::FREQUENCY_RIGHT, readout::MORPH_RIGHT)]
        .map(|(f, m)| Some((readout_value(parameters, f)?, readout_value(parameters, m)?)));
    let live = match live {
        [Some(left), Some(right)] if left.0 > 0.0 && right.0 > 0.0 => Some([left, right]),
        _ => None,
    };
    Values { settings, lfo_amount, lfo_morph, lfo_phase: value(parameters, "lfoPhase"), live }
}

impl Values {
    fn modulated(&self) -> bool {
        self.lfo_amount != 0.0 || self.lfo_morph != 0.0
    }

    /// The settings with the LFO at `lfo` (-1 to 1).
    fn swept(&self, lfo: f64) -> Settings {
        Settings {
            frequency: (self.settings.frequency.log2() + self.lfo_amount * lfo)
                .exp2()
                .clamp(FREQUENCY_RANGE.0, FREQUENCY_RANGE.1),
            morph: (self.settings.morph + self.lfo_morph * lfo).clamp(0.0, 1.0),
            ..self.settings
        }
    }

    /// The settings where the LFO has them now on `side`, or the parameters' own.
    fn live(&self, side: usize) -> Settings {
        match self.live.and_then(|live| live.get(side).copied()) {
            Some((frequency, morph)) if self.modulated() => Settings {
                frequency: frequency.clamp(FREQUENCY_RANGE.0, FREQUENCY_RANGE.1),
                morph: morph.clamp(0.0, 1.0),
                ..self.settings
            },
            _ => self.settings,
        }
    }

    /// Whether the dot's height edits the gain rather than the resonance.
    fn dot_is_gain(&self) -> bool {
        GAIN_TYPES.contains(&self.settings.kind)
    }
}

/// The x of a frequency on the display's logarithmic axis.
fn frequency_x(frequency: f64, display: Rect) -> f32 {
    let (low, high) = FREQUENCY_RANGE;
    display.left() + ((frequency / low).ln() / (high / low).ln()).clamp(0.0, 1.0) as f32 * display.width()
}

/// The frequency at an x on the display's axis.
fn x_frequency(x: f32, display: Rect) -> f64 {
    let (low, high) = FREQUENCY_RANGE;
    low * ((((x - display.left()) / display.width()).clamp(0.0, 1.0) as f64) * (high / low).ln()).exp()
}

/// The y of a level in dB, clamped to the display.
fn db_y(db: f64, display: Rect) -> f32 {
    let t = ((DB.0 - db) / (DB.0 - DB.1)).clamp(0.0, 1.0);
    display.top() + t as f32 * display.height()
}

/// The dot's travel: `t` from 0 at the bottom to 1 at the top, inside the margins.
fn travel_y(t: f64, display: Rect) -> f32 {
    display.bottom() - DOT_MARGIN - t.clamp(0.0, 1.0) as f32 * (display.height() - 2.0 * DOT_MARGIN)
}

/// The share of the dot's travel at a y, 0 at the bottom to 1 at the top.
fn y_travel(y: f32, display: Rect) -> f64 {
    f64::from(((display.bottom() - DOT_MARGIN - y) / (display.height() - 2.0 * DOT_MARGIN).max(1.0)).clamp(0.0, 1.0))
}

/// The dot's height for the values: the gain for the peak and the shelves, else the resonance.
fn dot_y(values: &Values, display: Rect) -> f32 {
    if values.dot_is_gain() {
        travel_y((values.settings.gain - GAIN.0) / (GAIN.1 - GAIN.0), display)
    } else {
        travel_y(values.settings.resonance, display)
    }
}

/// The frequencies the curve is drawn at, evenly across the axis.
fn frequencies() -> impl Iterator<Item = f64> {
    let (low, high) = FREQUENCY_RANGE;
    (0..=POINTS).map(move |i| low * ((i as f64 / POINTS as f64) * (high / low).ln()).exp())
}

/// The response for `settings`, in dB at each of [`frequencies`].
fn levels(settings: &Settings) -> Vec<f64> {
    let plan = Plan::new(settings, DRAWN_RATE);
    frequencies().map(|f| plan.db(DRAWN_RATE, f)).collect()
}

/// The curve for `settings` in points across the display.
fn curve_points(settings: &Settings, display: Rect) -> Vec<Pos2> {
    frequencies().zip(levels(settings)).map(|(f, db)| Pos2::new(frequency_x(f, display), db_y(db, display))).collect()
}

/// The lowest and highest level the LFO's sweep reaches at each of [`frequencies`].
fn sweep_band(values: &Values) -> Vec<(f64, f64)> {
    let mut band = vec![(f64::MAX, f64::MIN); POINTS + 1];
    for step in 0..SWEEP_STEPS {
        let lfo = step as f64 / (SWEEP_STEPS - 1) as f64 * 2.0 - 1.0;
        for (range, db) in band.iter_mut().zip(levels(&values.swept(lfo))) {
            *range = (range.0.min(db), range.1.max(db));
        }
    }
    band
}

/// A filled strip between two lines of points with the same xs.
fn strip(upper: &[Pos2], lower: &[Pos2], color: Color32) -> egui::Shape {
    let mut mesh = egui::Mesh::default();
    for (top, bottom) in upper.iter().zip(lower) {
        mesh.colored_vertex(*top, color);
        mesh.colored_vertex(*bottom, color);
    }
    for index in 1..upper.len().min(lower.len()) as u32 {
        let top = index * 2;
        mesh.add_triangle(top - 2, top - 1, top);
        mesh.add_triangle(top - 1, top, top + 1);
    }
    egui::Shape::mesh(mesh)
}

/// A frequency as the display's readout writes it.
fn frequency_text(frequency: f64) -> String {
    if frequency >= 1000.0 {
        format!("{} kHz", fixed(frequency / 1000.0, 2))
    } else {
        format!("{} Hz", fixed(frequency, 0))
    }
}

/// Draws the display into `bounds` and edits through `device.setParam` actions addressed to
/// `at`.
pub(crate) fn show(
    ui: &mut egui::Ui,
    display: Rect,
    parameters: &[GraphParameter],
    track: TrackId,
    at: &DeviceAt,
    actions: &mut Vec<Action>,
) {
    let values = values(parameters);
    draw(ui, display, &values);
    drag_dot(ui, display, &values, parameters, track, at, actions);
}

fn draw(ui: &mut egui::Ui, display: Rect, values: &Values) {
    let colors = super::device_canvas::colours();
    let painter = ui.painter_at(display);
    painter.rect_filled(display, 2.0, colors.window_background);
    let label = theme::ui_font(9.0);
    // The level grid: a line every 12 dB, the 0 dB line brightest.
    for db in [24.0, 12.0, 0.0, -12.0, -24.0, -36.0] {
        let y = db_y(db, display);
        let color = if db == 0.0 { colors.text_secondary.gamma_multiply(0.6) } else { colors.border };
        painter.line_segment([Pos2::new(display.left(), y), Pos2::new(display.right(), y)], Stroke::new(1.0, color));
    }
    // The frequency grid: a line at every decade, fainter ones between, labelled at the bottom.
    for decade in 1..=4 {
        for step in 1..10 {
            let frequency = 10f64.powi(decade) * f64::from(step);
            if !(FREQUENCY_RANGE.0..=FREQUENCY_RANGE.1).contains(&frequency) {
                continue;
            }
            let x = frequency_x(frequency, display);
            let color = if step == 1 { colors.border } else { colors.border.gamma_multiply(0.4) };
            painter
                .line_segment([Pos2::new(x, display.top()), Pos2::new(x, display.bottom())], Stroke::new(1.0, color));
            if step == 1 {
                let text =
                    if frequency >= 1000.0 { format!("{}k", frequency / 1000.0) } else { format!("{frequency}") };
                painter.text(
                    Pos2::new(x + 2.0, display.bottom() - 2.0),
                    Align2::LEFT_BOTTOM,
                    text,
                    label.clone(),
                    colors.text_secondary,
                );
            }
        }
    }

    let bottom: Vec<Pos2> = frequencies().map(|f| Pos2::new(frequency_x(f, display), display.bottom())).collect();
    if values.modulated() {
        // The range the LFO sweeps: a faint band between the lowest and highest curves.
        let band = sweep_band(values);
        let xs: Vec<f32> = frequencies().map(|f| frequency_x(f, display)).collect();
        let upper: Vec<Pos2> = xs.iter().zip(&band).map(|(&x, r)| Pos2::new(x, db_y(r.1, display))).collect();
        let lower: Vec<Pos2> = xs.iter().zip(&band).map(|(&x, r)| Pos2::new(x, db_y(r.0, display))).collect();
        painter.add(strip(&upper, &lower, colors.accent.gamma_multiply(0.18)));
        // The parameters' own curve, where the LFO swings around.
        painter.add(egui::Shape::dashed_line(
            &curve_points(&values.settings, display),
            Stroke::new(1.0, colors.text_secondary.gamma_multiply(0.7)),
            4.0,
            3.0,
        ));
        // The right side's curve when the stereo phase sets it apart from the left's.
        if values.lfo_phase > 0.0 && values.live.is_some() {
            painter.add(egui::Shape::line(
                curve_points(&values.live(1), display),
                Stroke::new(1.25, colors.accent.gamma_multiply(0.45)),
            ));
        }
    }
    // The filter as it sounds now: filled down to the floor, the line over it.
    let now = curve_points(&values.live(0), display);
    painter.add(strip(&now, &bottom, colors.accent.gamma_multiply(0.12)));
    painter.add(egui::Shape::line(now, Stroke::new(1.75, colors.accent)));

    // The type and frequency at the top right.
    let caption = format!(
        "{}  {}",
        TYPE_NAMES.get(values.settings.kind).copied().unwrap_or_default(),
        frequency_text(values.live(0).frequency)
    );
    painter.text(
        Pos2::new(display.right() - 4.0, display.top() + 3.0),
        Align2::RIGHT_TOP,
        caption,
        label,
        colors.text_secondary,
    );
}

/// The edits, all in one gesture, so a drag is one undo step: the window ends the gesture on
/// mouse up.
fn push_edits(actions: &mut Vec<Action>, edits: &[(&GraphParameter, f64)], track: TrackId, at: &DeviceAt) {
    for (parameter, value) in edits {
        actions.push(Action::new(
            "device.setParam",
            json!({
                "track": track.0, "device": at.value(), "param": parameter.id, "value": value,
                "gesture": "filter-curve-dot"
            }),
        ));
    }
}

/// The dot at the frequency: dragged sideways for the frequency and up and down for the
/// resonance (or the gain); Shift for precision; double-click resets both.
fn drag_dot(
    ui: &mut egui::Ui,
    display: Rect,
    values: &Values,
    parameters: &[GraphParameter],
    track: TrackId,
    at: &DeviceAt,
    actions: &mut Vec<Action>,
) {
    let colors = super::device_canvas::colours();
    let painter = ui.painter_at(display);
    let vertical = if values.dot_is_gain() { "gain" } else { "resonance" };
    let position = Pos2::new(
        frequency_x(values.settings.frequency, display).clamp(display.left() + 6.0, display.right() - 6.0),
        dot_y(values, display),
    );
    let id = egui::Id::unique(("filter-curve-dot", track.0, &at.0));
    let caption = if values.dot_is_gain() {
        format!("{}, {} dB", frequency_text(values.settings.frequency), fixed(values.settings.gain, 1))
    } else {
        format!(
            "{}, resonance {} %",
            frequency_text(values.settings.frequency),
            fixed(values.settings.resonance * 100.0, 0)
        )
    };
    let response = ui
        .interact(Rect::from_center_size(position, Vec2::splat(22.0)), id, Sense::click_and_drag())
        .on_hover_text(format!(
            "{caption}. Drag sideways for the frequency, up and down for the {vertical}; double-click resets."
        ))
        .on_hover_cursor(egui::CursorIcon::Move);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::SpinButton, true, &caption));
    let active = response.hovered() || response.dragged();
    painter.circle_filled(position, if active { 6.5 } else { 5.5 }, colors.accent);
    painter.circle_stroke(position, if active { 6.5 } else { 5.5 }, Stroke::new(1.5, colors.panel_background));

    let (Some(frequency), Some(height)) = (parameter(parameters, "frequency"), parameter(parameters, vertical)) else {
        return;
    };
    let state_id = id.with("drag");
    let held = response.is_pointer_button_down_on();
    let start = ui.ctx().data_mut(|d| {
        if held && d.get_temp::<[f32; 2]>(state_id).is_none() {
            d.insert_temp(state_id, [frequency_x(values.settings.frequency, display), dot_y(values, display)]);
        }
        d.get_temp::<[f32; 2]>(state_id)
    });
    let fine = if ui.input(|i| i.modifiers.shift) { 0.25 } else { 1.0 };
    if let Some([start_x, start_y]) = start {
        if response.double_clicked() {
            push_edits(actions, &[(frequency, frequency.default), (height, height.default)], track, at);
        } else if response.dragged() {
            let total = response.total_drag_delta().unwrap_or_default();
            let new_frequency = x_frequency(start_x + total.x * fine, display);
            let t = y_travel(start_y + total.y * fine, display);
            let new_height = if values.dot_is_gain() { GAIN.0 + t * (GAIN.1 - GAIN.0) } else { t };
            let mut edits = Vec::new();
            if (new_frequency - frequency.value).abs() > 1.0e-9 {
                edits.push((frequency, new_frequency));
            }
            if (new_height - height.value).abs() > 1.0e-9 {
                edits.push((height, new_height));
            }
            push_edits(actions, &edits, track, at);
        } else if response.drag_stopped() {
            // The drag ends with the values as they are, so the gesture closes even when the
            // pointer came back to where it started.
            push_edits(actions, &[(frequency, frequency.value), (height, height.value)], track, at);
        }
    }
    if !held {
        ui.ctx().data_mut(|d| d.remove::<[f32; 2]>(state_id));
    }
}

#[cfg(test)]
#[path = "filter_curve_tests.rs"]
mod tests;
