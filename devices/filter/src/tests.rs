// Copyright (C) 2026 The Suhina contributors
// SPDX-License-Identifier: GPL-3.0-or-later

use suhina_device_api::conformance::test_signal;

use super::*;

const SR: f64 = 48_000.0;

suhina_device_api::conformance_test!(Filter::with_rate);

fn filter(settings: &[(usize, f64)]) -> Filter {
    let mut f = Filter::new();
    for &(index, value) in settings {
        f.set_param(index, value);
    }
    f.reset();
    f
}

fn stopped() -> ProcessContext {
    ProcessContext { tempo: 120.0, beat: 0.0, playing: false }
}

/// The gain of `f` at `hz` on the left side, in dB, measured with a sine after it settles.
fn measured_db(f: &mut Filter, hz: f64) -> f64 {
    let frames = SR as usize;
    let mut buffer: Vec<f32> = (0..frames)
        .flat_map(|i| {
            let x = (std::f64::consts::TAU * hz * i as f64 / SR).sin() as f32 * 0.25;
            [x, x]
        })
        .collect();
    f.process(&stopped(), &mut buffer);
    let (mut re, mut im) = (0.0, 0.0);
    for (i, frame) in buffer.as_chunks::<2>().0.iter().enumerate().skip(frames / 2) {
        let phase = std::f64::consts::TAU * hz * i as f64 / SR;
        re += f64::from(frame[0]) * phase.sin();
        im += f64::from(frame[0]) * phase.cos();
    }
    20.0 * (2.0 * re.hypot(im) / (frames / 2) as f64 / 0.25).log10()
}

/// The display's curve for the device's settings, in dB at `hz`.
fn drawn_db(settings: &[(usize, f64)], hz: f64) -> f64 {
    let f = filter(settings);
    let s = Settings {
        kind: f.kind(),
        frequency: f.value(param::FREQUENCY),
        resonance: f.value(param::RESONANCE),
        gain: f.value(param::GAIN),
        morph: f.value(param::MORPH),
    };
    Plan::new(&s, SR).db(SR, hz)
}

#[test]
fn every_type_sounds_as_its_curve_is_drawn() {
    for (t, name) in TYPE_NAMES.iter().enumerate() {
        let settings = [
            (param::TYPE, t as f64),
            (param::FREQUENCY, 800.0),
            (param::RESONANCE, 0.4),
            (param::GAIN, 9.0),
            (param::MORPH, 0.3),
        ];
        for hz in [100.0, 450.0, 800.0, 1_300.0, 4_000.0] {
            let (sounds, drawn) = (measured_db(&mut filter(&settings), hz), drawn_db(&settings, hz));
            // Deep notches measure only as deep as the sine's leftovers; above -30 dB they agree.
            if drawn > -30.0 {
                assert!((sounds - drawn).abs() < 0.5, "{} at {hz} Hz: sounds {sounds:.2}, drawn {drawn:.2}", name);
            } else {
                assert!(sounds < -24.0, "{} at {hz} Hz: sounds {sounds:.2}, drawn {drawn:.2}", name);
            }
        }
    }
}

#[test]
fn the_cuts_pass_one_side_and_cut_the_other() {
    let low = [(param::TYPE, kind::LOW_PASS_24 as f64), (param::FREQUENCY, 1_000.0)];
    assert!(drawn_db(&low, 100.0).abs() < 0.5);
    assert!(drawn_db(&low, 10_000.0) < -70.0, "24 dB/octave over three octaves");
    let high = [(param::TYPE, kind::HIGH_PASS_12 as f64), (param::FREQUENCY, 1_000.0)];
    assert!(drawn_db(&high, 10_000.0).abs() < 0.5);
    assert!(drawn_db(&high, 100.0) < -35.0);
}

#[test]
fn resonance_raises_a_peak_at_the_frequency() {
    let at = |resonance| drawn_db(&[(param::FREQUENCY, 2_000.0), (param::RESONANCE, resonance)], 2_000.0);
    assert!(at(0.1) < 0.0);
    assert!(at(1.0) > 20.0, "Q 20 stands 26 dB high");
}

#[test]
fn the_comb_peaks_on_its_harmonics() {
    let plus = [(param::TYPE, kind::COMB_PLUS as f64), (param::FREQUENCY, 500.0), (param::RESONANCE, 1.0)];
    for hz in [500.0, 1_000.0, 1_500.0] {
        assert!(drawn_db(&plus, hz).abs() < 0.01, "{hz}");
    }
    assert!(drawn_db(&plus, 750.0) < -40.0);
    let minus = [(param::TYPE, kind::COMB_MINUS as f64), (param::FREQUENCY, 500.0), (param::RESONANCE, 1.0)];
    assert!(drawn_db(&minus, 250.0).abs() < 0.01);
    assert!(drawn_db(&minus, 500.0) < -40.0);
}

#[test]
fn the_formant_moves_with_the_vowel() {
    let a = curve::formants(0.0, curve::FORMANT_REFERENCE);
    let i = curve::formants(0.5, curve::FORMANT_REFERENCE);
    assert_eq!(a[0], 730.0);
    assert_eq!(i[0], 270.0);
    // The frequency shifts every formant by the same ratio.
    let shifted = curve::formants(0.0, 2.0 * curve::FORMANT_REFERENCE);
    assert!(shifted.iter().zip(a).all(|(s, a)| (s / a - 2.0).abs() < 1e-9));
}

#[test]
fn defaults_with_no_mix_pass_the_signal_through_exactly() {
    let input = test_signal(4_096);
    let mut output = input.clone();
    filter(&[(param::MIX, 0.0)]).process(&stopped(), &mut output);
    assert_eq!(input, output);
}

#[test]
fn the_lfo_sweeps_the_frequency_and_reports_it() {
    let mut f = filter(&[(param::LFO_AMOUNT, 2.0), (param::LFO_RATE, 4.0), (param::LFO_PHASE, 0.5)]);
    let mut lows = f64::MAX;
    let mut highs = 0.0f64;
    // 1/16 s blocks over one second: the readout follows the LFO a quarter cycle at a time.
    for _ in 0..16 {
        let mut buffer = vec![0.0f32; 2 * 3_000];
        f.process(&stopped(), &mut buffer);
        let [left, right] = [readout::FREQUENCY_LEFT, readout::FREQUENCY_RIGHT].map(|r| f64::from(f.readouts[r]));
        lows = lows.min(left);
        highs = highs.max(left);
        // Half a cycle apart, the sides sit mirrored around the frequency in octaves.
        assert!(((left * right).sqrt() - 1_000.0).abs() < 1.0, "{left} {right}");
    }
    assert!((lows - 250.0).abs() < 1.0 && (highs - 4_000.0).abs() < 1.0, "{lows} {highs}");
}

#[test]
fn a_synced_lfo_lands_on_the_same_point_of_the_song() {
    let settings = [(param::LFO_AMOUNT, 1.0), (param::LFO_SYNC, 1.0), (param::LFO_SHAPE, 2.0)];
    let at = |beat: f64| {
        let mut f = filter(&settings);
        let mut buffer = vec![0.0f32; 2];
        f.process(&ProcessContext { tempo: 120.0, beat, playing: true }, &mut buffer);
        f64::from(f.readouts[readout::FREQUENCY_LEFT])
    };
    // A quarter-note saw: every beat the same, half a beat later an octave higher.
    assert!((at(3.0) - at(7.0)).abs() < 1e-3);
    assert!((at(3.5) / at(3.0) - 2.0).abs() < 1e-3, "{} {}", at(3.0), at(3.5));
}

#[test]
fn changing_type_while_running_stays_finite() {
    let mut f = filter(&[(param::RESONANCE, 1.0), (param::DRIVE, 1.0), (param::LFO_AMOUNT, 4.0)]);
    let mut buffer = test_signal(512);
    for t in 0..(TYPE_NAMES.len() * 3) {
        f.set_param(param::TYPE, (t % TYPE_NAMES.len()) as f64);
        f.process(&stopped(), &mut buffer);
        assert!(buffer.iter().all(|s| s.is_finite() && s.abs() < 50.0), "{}", TYPE_NAMES[t % TYPE_NAMES.len()]);
    }
}

#[test]
fn every_parameter_reaches_the_sound() {
    let input = test_signal(8_192);
    let base = [(param::LFO_RATE, 3.0), (param::TYPE, kind::MORPH as f64)];
    let mut plain = input.clone();
    filter(&base).process(&stopped(), &mut plain);
    let changes = [
        (param::TYPE, kind::NOTCH as f64),
        (param::FREQUENCY, 300.0),
        (param::RESONANCE, 0.8),
        (param::MORPH, 0.6),
        (param::DRIVE, 0.8),
        (param::LFO_AMOUNT, 1.0),
        (param::OUTPUT, -6.0),
        (param::MIX, 0.5),
    ];
    for (index, value) in changes {
        let mut output = input.clone();
        let mut settings = base.to_vec();
        settings.push((index, value));
        filter(&settings).process(&stopped(), &mut output);
        assert_ne!(output, plain, "{}", PARAMS[index].id);
    }
    // Gain only shapes the peak and the shelves.
    let peak = [(param::TYPE, kind::PEAK as f64)];
    assert!((drawn_db(&peak, 1_000.0) - drawn_db(&[peak[0], (param::GAIN, 12.0)], 1_000.0)).abs() > 10.0);
}
