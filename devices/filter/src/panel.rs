// Copyright (C) 2026 The Suhina contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The Filter's panel (docs/spec/filter.md › The panel): the Type menu over a response display
//! ([`Widget::FilterCurve`], drawn on the host side from [`crate::curve`]), the filter's dials
//! beside it and the LFO's after a divider. The dials and menus are the SDK's own, so the theme
//! draws them like every other device's.

use suhina_device_sdk::builtin::{COMPACT_HEIGHT, Layout};
use suhina_device_sdk::ui::*;

/// The panel's width.
pub const WIDTH: f32 = 708.0;
/// The response display's rectangle under the Type menu.
pub const DISPLAY: [f32; 4] = [0.0, 26.0, 240.0, COMPACT_HEIGHT - 26.0];
/// A dial's cell.
const DIAL: [f32; 2] = [60.0, 84.0];
/// Where the filter's dials start.
const FILTER_X: f32 = 246.0;
/// Where the LFO's dials start, after the divider.
const LFO_X: f32 = 494.0;
const TOOLTIP: &str = "Drag vertically; Shift for precision; double click resets";

/// The layout of the Filter's panel (ADR 0021 › Amendment: device panels for built-in devices).
pub fn layout() -> Layout {
    Layout { ui: Some(panel()), ..Layout::default() }
}

/// `parameter`'s value equal to `value`: for a control shown only in one LFO mode.
fn when(parameter: &str, value: f32) -> Option<UiValue> {
    Some(UiValue::Expression(Box::new(UiExpression::Equal {
        a: UiValue::parameter(parameter, false),
        b: value.into(),
    })))
}

/// `|parameter|`, as the larger of it and its negation.
fn magnitude(parameter: &str) -> UiValue {
    let value = UiValue::parameter(parameter, false);
    UiValue::Expression(Box::new(UiExpression::Max { a: value.clone(), b: value * (-1.0).into() }))
}

fn control(id: &str, name: &str, bounds: [f32; 4], widget: Widget) -> CustomControl {
    CustomControl {
        id: id.into(),
        name: name.into(),
        tooltip: String::new(),
        bounds,
        widget: Some(widget),
        gesture: UiGesture::Drag,
        x: None,
        y: None,
        drawing: vec![],
        visible_when: None,
    }
}

/// A dial for `id` in the cell at `x`, row `row`.
fn dial(id: &str, label: &str, x: f32, row: usize, logarithmic: bool, decimals: usize) -> CustomControl {
    let mut c = control(
        id,
        label,
        [x, row as f32 * DIAL[1], DIAL[0], DIAL[1]],
        Widget::Dial { label: label.into(), decimals, format: TextFormat::Number, image: None },
    );
    c.tooltip = TOOLTIP.into();
    c.y = Some(UiBinding { parameter: id.into(), logarithmic, drag_scale: 1.0 });
    c
}

/// A menu for `id`.
fn menu(id: &str, name: &str, bounds: [f32; 4], tooltip: &str) -> CustomControl {
    let mut c = control(id, name, bounds, Widget::Menu);
    c.gesture = UiGesture::Menu;
    c.tooltip = tooltip.into();
    c.x = Some(UiBinding::new(id));
    c
}

fn label(id: &str, text: &str, bounds: [f32; 4]) -> CustomControl {
    control(id, text, bounds, Widget::Label { label: text.into() })
}

fn panel() -> DeviceUi {
    let column = |i: usize, start: f32| start + i as f32 * DIAL[0];
    let controls = vec![
        menu("type", "Type", [2.0, 2.0, DISPLAY[2] - 4.0, 20.0], "The filter's type"),
        {
            let mut c = control("filter-curve", "Filter response", DISPLAY, Widget::FilterCurve);
            c.tooltip = "Drag the dot sideways for the frequency and up and down for the resonance; \
                         double-click resets both. The bright curve is the filter as the LFO has moved \
                         it, the shaded band the range the LFO sweeps."
                .into();
            c
        },
        dial("frequency", "Freq", column(0, FILTER_X), 0, true, 0),
        dial("resonance", "Res", column(1, FILTER_X), 0, false, 0),
        dial("gain", "Gain", column(2, FILTER_X), 0, false, 1),
        dial("morph", "Morph", column(3, FILTER_X), 0, false, 0),
        dial("drive", "Drive", column(0, FILTER_X), 1, false, 0),
        dial("output", "Output", column(1, FILTER_X), 1, false, 1),
        dial("mix", "Dry/Wet", column(2, FILTER_X), 1, false, 0),
        control("divider", "Divider", [LFO_X - 10.0, 6.0, 8.0, COMPACT_HEIGHT - 12.0], Widget::Separator),
        dial("lfoAmount", "LFO Amt", column(0, LFO_X), 0, false, 2),
        {
            let mut c = dial("lfoRate", "Rate", column(1, LFO_X), 0, true, 2);
            c.visible_when = when("lfoSync", 0.0);
            c
        },
        {
            let mut c = label("synced-rate-caption", "Rate", [column(1, LFO_X) + 18.0, 14.0, DIAL[0] - 18.0, 16.0]);
            c.visible_when = when("lfoSync", 1.0);
            c
        },
        {
            let mut c = menu(
                "lfoSyncedRate",
                "LFO Synced Rate",
                [column(1, LFO_X), 34.0, DIAL[0] + 4.0, 20.0],
                "The LFO's cycle as a note length",
            );
            c.visible_when = when("lfoSync", 1.0);
            c
        },
        dial("lfoMorph", "To Morph", column(2, LFO_X), 0, false, 0),
        dial("lfoPhase", "Stereo", column(0, LFO_X), 1, false, 2),
        dial("lfoOffset", "Offset", column(1, LFO_X), 1, false, 2),
        label("lfo-caption", "LFO", [column(2, LFO_X) + 4.0, 88.0, 90.0, 14.0]),
        {
            let mut c = control(
                "lfoSync",
                "LFO Sync",
                [column(2, LFO_X) + 4.0, 106.0, WIDTH - column(2, LFO_X) - 8.0, 20.0],
                Widget::Button { label: "Sync".into(), off_label: Some("Free".into()), indicator: false },
            );
            c.gesture = UiGesture::Toggle;
            c.tooltip = "Free runs the LFO in hertz; Sync follows the song in note lengths.".into();
            c.x = Some(UiBinding::new("lfoSync"));
            c
        },
        menu(
            "lfoShape",
            "LFO Shape",
            [column(2, LFO_X) + 4.0, 134.0, WIDTH - column(2, LFO_X) - 8.0, 20.0],
            "The LFO's waveform",
        ),
    ];
    // The canvas draws every parameter, so the generated list under it draws none of them.
    let hidden_parameters = crate::INFO.params.iter().map(|p| p.id.to_string()).collect();
    DeviceUi {
        size: [WIDTH, COMPACT_HEIGHT],
        hidden_parameters,
        controls,
        // Repaints keep coming while the LFO moves the filter, so the display follows it.
        animate_when: Some(magnitude("lfoAmount") + magnitude("lfoMorph")),
        ..DeviceUi::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::INFO;

    #[test]
    fn the_panel_loads_and_places_every_parameter_inside_it() {
        let loaded = layout().panel(&INFO).unwrap();
        let canvas = loaded.ui.as_ref().unwrap();
        assert_eq!(canvas.size, [WIDTH, COMPACT_HEIGHT]);
        for control in &canvas.controls {
            let [x, y, w, h] = control.bounds;
            assert!(x >= 0.0 && y >= 0.0, "{} is outside", control.id);
            assert!(x + w <= canvas.size[0] && y + h <= canvas.size[1], "{} is outside", control.id);
        }
        let bound: Vec<_> = canvas
            .controls
            .iter()
            .flat_map(|c| [c.x.as_ref(), c.y.as_ref()].into_iter().flatten())
            .map(|b| b.parameter.as_str())
            .collect();
        for parameter in INFO.params.iter() {
            assert!(bound.contains(&&*parameter.id), "{} has no control", parameter.id);
        }
        assert!(canvas.controls.iter().any(|c| matches!(c.widget, Some(Widget::FilterCurve))));
    }
}
