// Copyright (C) 2026 The Suhina contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! What the filter runs for one set of values, and the response it has: the one place both the
//! audio path ([`crate::Filter`]) and the panel's response display read, so the curve on screen
//! is the filter that sounds.

use suhina_dsp::{Comb, SvfCoefficients};

/// The filter types, in the order the Type menu lists them.
pub const TYPE_NAMES: [&str; 14] = [
    "Low-pass 12",
    "Low-pass 24",
    "High-pass 12",
    "High-pass 24",
    "Band-pass",
    "Notch",
    "Peak",
    "Low Shelf",
    "High Shelf",
    "Morph",
    "Comb +",
    "Comb -",
    "Phaser",
    "Formant",
];

/// Indices into [`TYPE_NAMES`].
pub mod kind {
    pub const LOW_PASS_12: usize = 0;
    pub const LOW_PASS_24: usize = 1;
    pub const HIGH_PASS_12: usize = 2;
    pub const HIGH_PASS_24: usize = 3;
    pub const BAND_PASS: usize = 4;
    pub const NOTCH: usize = 5;
    pub const PEAK: usize = 6;
    pub const LOW_SHELF: usize = 7;
    pub const HIGH_SHELF: usize = 8;
    pub const MORPH: usize = 9;
    pub const COMB_PLUS: usize = 10;
    pub const COMB_MINUS: usize = 11;
    pub const PHASER: usize = 12;
    pub const FORMANT: usize = 13;
}

/// The types whose Gain shapes the response.
pub const GAIN_TYPES: [usize; 3] = [kind::PEAK, kind::LOW_SHELF, kind::HIGH_SHELF];
/// The types whose Morph shapes the response.
pub const MORPH_TYPES: [usize; 2] = [kind::MORPH, kind::FORMANT];

/// The frequency parameter's range, which the display's axis spans too.
pub const FREQUENCY_RANGE: (f64, f64) = (20.0, 20_000.0);
/// The phaser's all-pass stages; each pair makes one notch.
pub const PHASER_STAGES: usize = 6;
/// The Q of each phaser stage: wide notches.
const PHASER_Q: f64 = 0.5;
/// The phaser's feedback at full resonance.
const PHASER_MAX_FEEDBACK: f64 = 0.9;
/// The comb's feedback at no resonance and at full: some comb is always heard.
const COMB_FEEDBACK: (f64, f64) = (0.3, 0.99);
/// The Q of a resonant section at no resonance and at full, spread evenly in octaves.
const Q_RANGE: (f64, f64) = (0.5, 20.0);
/// The formants' Q at no resonance and at full.
const FORMANT_Q: (f64, f64) = (3.0, 20.0);

/// The first three formants of five vowels (A, E, I, O, U), in Hz: averages of adult voices
/// from the published vowel measurements in acoustic phonetics (Peterson & Barney, 1952).
const VOWELS: [[f64; 3]; 5] = [
    [730.0, 1_090.0, 2_440.0],
    [530.0, 1_840.0, 2_480.0],
    [270.0, 2_290.0, 3_010.0],
    [570.0, 840.0, 2_410.0],
    [300.0, 870.0, 2_240.0],
];
/// The formants' levels: each higher formant softer, as in a voice.
const FORMANT_GAINS: [f64; 3] = [1.0, 0.5, 0.35];
/// The Frequency at which the formants sit at their vowels' own frequencies; another
/// Frequency shifts them all by its ratio to this.
pub const FORMANT_REFERENCE: f64 = 1_000.0;

/// The values that decide the filter's response at one moment: the parameters, with the LFO
/// already applied to the frequency and the morph.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Settings {
    pub kind: usize,
    pub frequency: f64,
    /// 0 to 1.
    pub resonance: f64,
    /// In dB, for the peak and the shelves.
    pub gain: f64,
    /// 0 to 1: the morph filter's position, or the formant filter's vowel.
    pub morph: f64,
}

/// What to run for one set of [`Settings`]: the sections' coefficients and how they connect.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Plan {
    pub kind: usize,
    /// Up to three state-variable sections: in series for the plain types, in parallel for the
    /// formants; the phaser runs the first in every stage.
    pub sections: [SvfCoefficients; 3],
    /// How many of `sections` are in use.
    pub count: usize,
    /// The comb's tuning, in Hz.
    pub frequency: f64,
    /// The comb's or the phaser's feedback.
    pub feedback: f64,
}

/// `value` within `0..=1`, non-finite as 0.
fn unit(value: f64) -> f64 {
    if value.is_finite() { value.clamp(0.0, 1.0) } else { 0.0 }
}

/// `from` at 0 to `to` at 1, evenly in ratio.
fn geometric((from, to): (f64, f64), t: f64) -> f64 {
    from * (to / from).powf(unit(t))
}

/// The Q of a resonant section at `resonance`.
pub fn q(resonance: f64) -> f64 {
    geometric(Q_RANGE, resonance)
}

/// The three formant frequencies at `morph` (0 to 1 through A, E, I, O, U), shifted by
/// `frequency`'s ratio to [`FORMANT_REFERENCE`].
pub fn formants(morph: f64, frequency: f64) -> [f64; 3] {
    let position = unit(morph) * (VOWELS.len() - 1) as f64;
    let from = (position as usize).min(VOWELS.len() - 2);
    let t = position - from as f64;
    let shift = if frequency.is_finite() && frequency > 0.0 { frequency / FORMANT_REFERENCE } else { 1.0 };
    let (a, b) = (VOWELS[from], VOWELS[from + 1]);
    // Between vowels each formant glides evenly in pitch.
    [0, 1, 2].map(|i| a[i] * (b[i] / a[i]).powf(t) * shift)
}

impl Plan {
    /// The plan for `s` at `sample_rate`.
    pub fn new(s: &Settings, sample_rate: f64) -> Self {
        let f = if s.frequency.is_finite() {
            s.frequency.clamp(FREQUENCY_RANGE.0, FREQUENCY_RANGE.1)
        } else {
            FREQUENCY_RANGE.0
        };
        let q = q(s.resonance);
        let identity = SvfCoefficients::IDENTITY;
        let one = |c: SvfCoefficients| ([c, identity, identity], 1);
        let (sections, count) = match s.kind {
            kind::LOW_PASS_12 => one(SvfCoefficients::lowpass(sample_rate, f, q)),
            // Two sections, each with the square root of the Q, so the slope steepens without the
            // resonance standing twice as high.
            kind::LOW_PASS_24 => {
                let c = SvfCoefficients::lowpass(sample_rate, f, q.sqrt());
                ([c, c, identity], 2)
            }
            kind::HIGH_PASS_12 => one(SvfCoefficients::highpass(sample_rate, f, q)),
            kind::HIGH_PASS_24 => {
                let c = SvfCoefficients::highpass(sample_rate, f, q.sqrt());
                ([c, c, identity], 2)
            }
            kind::BAND_PASS => one(SvfCoefficients::bandpass(sample_rate, f, q)),
            kind::NOTCH => one(SvfCoefficients::notch(sample_rate, f, q)),
            kind::PEAK => one(SvfCoefficients::bell(sample_rate, f, q, s.gain)),
            kind::LOW_SHELF => one(SvfCoefficients::low_shelf(sample_rate, f, q, s.gain)),
            kind::HIGH_SHELF => one(SvfCoefficients::high_shelf(sample_rate, f, q, s.gain)),
            kind::MORPH => one(SvfCoefficients::morph(sample_rate, f, q, unit(s.morph))),
            kind::PHASER => one(SvfCoefficients::allpass(sample_rate, f, PHASER_Q)),
            kind::FORMANT => {
                let fq = geometric(FORMANT_Q, s.resonance);
                (formants(s.morph, f).map(|at| SvfCoefficients::bandpass(sample_rate, at, fq)), 3)
            }
            // The combs run no sections.
            _ => ([identity; 3], 0),
        };
        let feedback = match s.kind {
            kind::COMB_PLUS => COMB_FEEDBACK.0 + (COMB_FEEDBACK.1 - COMB_FEEDBACK.0) * unit(s.resonance),
            kind::COMB_MINUS => -(COMB_FEEDBACK.0 + (COMB_FEEDBACK.1 - COMB_FEEDBACK.0) * unit(s.resonance)),
            kind::PHASER => PHASER_MAX_FEEDBACK * unit(s.resonance),
            _ => 0.0,
        };
        Self { kind: s.kind.min(TYPE_NAMES.len() - 1), sections, count, frequency: f, feedback }
    }

    /// The complex response at `at` Hz, `[real, imaginary]`.
    pub fn response(&self, sample_rate: f64, at: f64) -> [f64; 2] {
        let section = |i: usize| self.sections.get(i).map_or([1.0, 0.0], |c| c.response(sample_rate, at));
        match self.kind {
            kind::COMB_PLUS | kind::COMB_MINUS => Comb::response(sample_rate, self.frequency, self.feedback, at),
            kind::PHASER => {
                // w = A v, v = x + g w z⁻¹, y = (x + w) / 2.
                let mut a = [1.0, 0.0];
                for _ in 0..PHASER_STAGES {
                    a = multiply(a, section(0));
                }
                let omega = std::f64::consts::TAU * at / sample_rate;
                let delayed = multiply(a, [omega.cos(), -omega.sin()]);
                let w = divide(a, [1.0 - self.feedback * delayed[0], -self.feedback * delayed[1]]);
                [(1.0 + w[0]) / 2.0, w[1] / 2.0]
            }
            kind::FORMANT => {
                let mut sum = [0.0, 0.0];
                for (i, gain) in FORMANT_GAINS.iter().enumerate() {
                    let [re, im] = section(i);
                    sum = [sum[0] + gain * re, sum[1] + gain * im];
                }
                sum
            }
            _ => {
                let mut total = [1.0, 0.0];
                for i in 0..self.count {
                    total = multiply(total, section(i));
                }
                total
            }
        }
    }

    /// The gain at `at` Hz in dB, floored at -120.
    pub fn db(&self, sample_rate: f64, at: f64) -> f64 {
        let [re, im] = self.response(sample_rate, at);
        let magnitude = re.hypot(im);
        if magnitude.is_finite() { 20.0 * magnitude.max(1.0e-6).log10() } else { 0.0 }
    }
}

/// The formants' levels, for the audio path.
pub(crate) const fn formant_gains() -> [f64; 3] {
    FORMANT_GAINS
}

fn multiply([a, b]: [f64; 2], [c, d]: [f64; 2]) -> [f64; 2] {
    [a * c - b * d, a * d + b * c]
}

fn divide(n: [f64; 2], [c, d]: [f64; 2]) -> [f64; 2] {
    let m = c * c + d * d;
    if !(m.is_finite() && m > 0.0) {
        return [1.0, 0.0];
    }
    let [re, im] = multiply(n, [c, -d]);
    [re / m, im / m]
}
