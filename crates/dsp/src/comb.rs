// Copyright (C) 2026 The Suhina contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! A feedback comb filter: the input plus its own output one period ago, scaled by `feedback`.
//! A positive feedback raises peaks at the tuned frequency and every multiple of it; a negative
//! one at the odd multiples of half of it (an octave lower, hollow, like a closed pipe). The
//! output is scaled by `1 - |feedback|`, so the peaks stand at unity whatever the feedback and
//! only the gaps between them deepen.
//!
//! The period is read between samples (cubic interpolation), so the tuning can sweep smoothly
//! while sound passes.

use crate::DelayLine;

/// Lowest frequency a comb tunes to; its line is made long enough for this period.
pub const MIN_FREQUENCY: f64 = 20.0;
/// Largest feedback magnitude: just below 1, so the peaks ring but never run away.
pub const MAX_FEEDBACK: f64 = 0.99;

/// See the module.
#[derive(Debug, Clone, PartialEq)]
pub struct Comb {
    line: DelayLine,
    sample_rate: f64,
}

impl Default for Comb {
    /// A comb for 48 kHz.
    fn default() -> Self {
        Self::new(48_000.0)
    }
}

/// `feedback` limited to what a comb runs: within ±[`MAX_FEEDBACK`], non-finite as none.
fn usable_feedback(feedback: f64) -> f64 {
    if feedback.is_finite() { feedback.clamp(-MAX_FEEDBACK, MAX_FEEDBACK) } else { 0.0 }
}

/// The period in frames for `frequency`: from [`MIN_FREQUENCY`] up to where it is two frames,
/// the shortest delay the interpolation reads.
fn period(sample_rate: f64, frequency: f64) -> f64 {
    let sample_rate = crate::usable_sample_rate(sample_rate);
    let frequency = if frequency.is_finite() { frequency } else { 1_000.0 };
    sample_rate / frequency.clamp(MIN_FREQUENCY, sample_rate / 2.0)
}

impl Comb {
    /// A silent comb for `sample_rate`, with a line long enough for [`MIN_FREQUENCY`]
    /// (allocates).
    pub fn new(sample_rate: f64) -> Self {
        let sample_rate = crate::usable_sample_rate(sample_rate);
        Self { line: DelayLine::for_seconds(1.0 / MIN_FREQUENCY + 0.001, sample_rate), sample_rate }
    }

    /// One sample through the comb tuned to `frequency` with `feedback`.
    #[inline]
    pub fn process(&mut self, x: f64, frequency: f64, feedback: f64) -> f64 {
        let feedback = usable_feedback(feedback);
        let delayed = f64::from(self.line.read_hermite(period(self.sample_rate, frequency)));
        let y = crate::flush(x + feedback * delayed);
        self.line.push(y as f32);
        y * (1.0 - feedback.abs())
    }

    /// Silences the comb.
    pub fn reset(&mut self) {
        self.line.clear();
    }

    /// The complex response `[real, imaginary]` of a comb at `sample_rate` tuned to
    /// `frequency` with `feedback`, at `at` Hz: `(1 - |g|) / (1 - g e^(-jωD))`.
    pub fn response(sample_rate: f64, frequency: f64, feedback: f64, at: f64) -> [f64; 2] {
        let g = usable_feedback(feedback);
        let sample_rate = crate::usable_sample_rate(sample_rate);
        let angle = std::f64::consts::TAU * at / sample_rate * period(sample_rate, frequency);
        let (d_re, d_im) = (1.0 - g * angle.cos(), g * angle.sin());
        let d = d_re * d_re + d_im * d_im;
        if !(d.is_finite() && d > 0.0) {
            return [1.0, 0.0];
        }
        let scale = 1.0 - g.abs();
        [scale * d_re / d, -scale * d_im / d]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: f64 = 48_000.0;

    fn magnitude(frequency: f64, feedback: f64, at: f64) -> f64 {
        let [re, im] = Comb::response(SR, frequency, feedback, at);
        re.hypot(im)
    }

    /// The gain at `hz`, measured by running a sine through a comb.
    fn measured(frequency: f64, feedback: f64, hz: f64) -> f64 {
        let mut comb = Comb::new(SR);
        let n = 48_000;
        let (mut re, mut im) = (0.0, 0.0);
        for i in 0..n {
            let phase = std::f64::consts::TAU * hz * i as f64 / SR;
            let y = comb.process(phase.sin(), frequency, feedback);
            if i >= n / 2 {
                re += y * phase.sin();
                im += y * phase.cos();
            }
        }
        2.0 * re.hypot(im) / (n / 2) as f64
    }

    #[test]
    fn positive_feedback_peaks_at_every_multiple() {
        for at in [500.0, 1_000.0, 1_500.0] {
            assert!((magnitude(500.0, 0.9, at) - 1.0).abs() < 1e-6, "{at}");
            assert!((measured(500.0, 0.9, at) - 1.0).abs() < 0.02, "{at}");
        }
        // Half way between peaks it falls to (1 - g) / (1 + g).
        assert!((magnitude(500.0, 0.9, 750.0) - 0.1 / 1.9).abs() < 1e-6);
        assert!((measured(500.0, 0.9, 750.0) - 0.1 / 1.9).abs() < 0.01);
    }

    #[test]
    fn negative_feedback_peaks_at_odd_multiples_of_half() {
        assert!((magnitude(500.0, -0.9, 250.0) - 1.0).abs() < 1e-6);
        assert!((magnitude(500.0, -0.9, 750.0) - 1.0).abs() < 1e-6);
        assert!(magnitude(500.0, -0.9, 500.0) < 0.06);
        assert!((measured(500.0, -0.9, 250.0) - 1.0).abs() < 0.02);
    }

    #[test]
    fn no_feedback_passes_the_input() {
        let mut comb = Comb::new(SR);
        for i in 0..1_000 {
            let x = (i as f64 * 0.01).sin();
            assert_eq!(comb.process(x, 440.0, 0.0), x);
        }
    }

    #[test]
    fn broken_settings_are_harmless() {
        let mut comb = Comb::new(f64::NAN);
        let out: Vec<f64> = (0..10_000)
            .map(|i| {
                let (f, g) = match i % 4 {
                    0 => (f64::NAN, f64::INFINITY),
                    1 => (0.0, 5.0),
                    2 => (1e9, -5.0),
                    _ => (f64::NEG_INFINITY, f64::NAN),
                };
                comb.process(if i == 7 { f64::NAN } else { 1.0 }, f, g)
            })
            .collect();
        assert!(out.iter().all(|v| v.is_finite() && v.abs() < 10.0));
        assert!(magnitude(f64::NAN, f64::NAN, f64::NAN).is_finite());
    }

    #[test]
    fn a_sweep_stays_bounded() {
        let mut comb = Comb::new(SR);
        let mut peak: f64 = 0.0;
        for i in 0..96_000 {
            let frequency = 20.0 * 1_000f64.powf((i as f64 / 9_600.0).sin().abs());
            peak = peak.max(comb.process((i as f64 * 0.07).sin(), frequency, 0.99).abs());
        }
        assert!(peak.is_finite() && peak < 4.0, "{peak}");
    }
}
