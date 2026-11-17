// Copyright (C) 2026 The Suhina contributors
// SPDX-License-Identifier: GPL-3.0-or-later

use super::*;
use suhina_device_sdk::builtin;

/// The panel's parameters at the device's defaults, with `changes`, as `show` reads them.
fn panel_parameters(changes: &[(&str, f64)]) -> Vec<GraphParameter> {
    let mut parameters: Vec<GraphParameter> =
        builtin::parameters(&suhina_device_filter::INFO).iter().map(|p| GraphParameter::new(p, p.default)).collect();
    for (id, value) in changes {
        parameters.iter_mut().find(|p| p.id == *id).unwrap().value = *value;
    }
    parameters
}

fn display() -> Rect {
    Rect::from_min_size(Pos2::ZERO, Vec2::new(240.0, 142.0))
}

#[test]
fn the_axes_map_back_and_forth() {
    let display = display();
    for frequency in [20.0, 100.0, 1_000.0, 20_000.0] {
        let back = x_frequency(frequency_x(frequency, display), display);
        assert!((back - frequency).abs() < frequency * 1.0e-4);
    }
    for t in [0.0, 0.3, 1.0] {
        assert!((y_travel(travel_y(t, display), display) - t).abs() < 1.0e-6);
    }
    assert_eq!(db_y(DB.0, display), display.top());
    assert_eq!(db_y(DB.1, display), display.bottom());
}

#[test]
fn the_values_come_from_the_parameters() {
    let values = values(&panel_parameters(&[("type", 6.0), ("frequency", 440.0), ("gain", 9.0)]));
    assert_eq!(values.settings.kind, 6);
    assert_eq!(values.settings.frequency, 440.0);
    assert!(values.dot_is_gain(), "the peak's dot moves its gain");
    assert!(!values.modulated());
    assert_eq!(values.live(0), values.settings, "with no LFO the curve is the parameters'");
}

#[test]
fn the_readouts_move_the_live_curve_only_while_the_lfo_runs() {
    let mut parameters = panel_parameters(&[("lfoAmount", 2.0)]);
    for (index, value) in [(0, 250.0), (1, 4_000.0), (2, 0.0), (3, 0.0)] {
        parameters.push(GraphParameter::readout(index, value).unwrap());
    }
    let swinging = values(&parameters);
    assert_eq!(swinging.live(0).frequency, 250.0);
    assert_eq!(swinging.live(1).frequency, 4_000.0);
    let band = sweep_band(&swinging);
    // The sweep reaches from two octaves down (250 Hz) to two up (4 kHz): at 2 kHz the
    // low-pass's band spans the filter open (near 0 dB) to well closed.
    let at = frequencies().position(|f| f >= 2_000.0).unwrap();
    assert!(band[at].1 > -1.5 && band[at].0 < -30.0, "{:?}", band[at]);
    let mut stale = panel_parameters(&[]);
    stale.extend_from_slice(&parameters[parameters.len() - 4..]);
    let still = values(&stale);
    assert_eq!(still.live(0).frequency, 1_000.0, "without the LFO a stale readout is ignored");
}

#[test]
fn the_dot_sits_at_the_resonance_or_the_gain() {
    let display = display();
    let low = values(&panel_parameters(&[("resonance", 0.0)]));
    let high = values(&panel_parameters(&[("resonance", 1.0)]));
    assert_eq!(dot_y(&low, display), display.bottom() - DOT_MARGIN);
    assert_eq!(dot_y(&high, display), display.top() + DOT_MARGIN);
    let flat_peak = values(&panel_parameters(&[("type", 6.0), ("gain", 0.0)]));
    assert_eq!(dot_y(&flat_peak, display), display.center().y);
}
