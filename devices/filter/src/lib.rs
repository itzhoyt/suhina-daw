// Copyright (C) 2026 The Suhina contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Filter: one multimode filter with a built-in LFO (docs/spec/filter.md).
//!
//! The types are the plain second-order ones (low-pass and high-pass at 12 or 24 dB/octave,
//! band-pass, notch, peak, low and high shelf), a morph through low-pass, band-pass, high-pass
//! and notch, two combs (peaks at every multiple of the frequency, or at the odd multiples of
//! half of it), a six-stage phaser and a formant filter that glides through five vowels. Drive
//! saturates the input before the filter, so the resonance has something to bite on.
//!
//! The LFO moves the frequency by up to four octaves either way, and the morph (or the vowel)
//! too; it runs freely in hertz or follows the song position in note lengths, as the Auto Pan's
//! does, and a stereo phase puts the right side's LFO ahead of the left's.
//!
//! Every value is smoothed, the frequency in octaves, so automation and the LFO sweep without
//! zipper noise. What runs for one moment's values is [`curve::Plan`], which the panel's
//! response display draws from too.

use suhina_device_api::{
    DEFAULT_SAMPLE_RATE, Device, DeviceError, DeviceInfo, MAX_READOUTS, ParamBank, ParamInfo, ProcessContext, Role,
    Scale, Unit, check_sample_rate,
};
use suhina_dsp::lfo::{Shape, wander, wave};
use suhina_dsp::sync::LfoClock;
use suhina_dsp::{Comb, Smoother, Svf};

pub mod curve;
pub mod panel;

use curve::{PHASER_STAGES, Plan, Settings, TYPE_NAMES, kind};

/// Ramp for changes of every smoothed value.
pub const RAMP_SECONDS: f64 = 0.02;
/// Largest LFO swing of the frequency, in octaves either way.
pub const MAX_LFO_OCTAVES: f64 = 4.0;

/// Parameter indices, in [`INFO`] order.
pub mod param {
    pub const TYPE: usize = 0;
    pub const FREQUENCY: usize = 1;
    pub const RESONANCE: usize = 2;
    pub const GAIN: usize = 3;
    pub const MORPH: usize = 4;
    pub const DRIVE: usize = 5;
    pub const LFO_AMOUNT: usize = 6;
    pub const LFO_MORPH: usize = 7;
    pub const LFO_SYNC: usize = 8;
    pub const LFO_RATE: usize = 9;
    pub const LFO_SYNCED_RATE: usize = 10;
    pub const LFO_SHAPE: usize = 11;
    pub const LFO_PHASE: usize = 12;
    pub const LFO_OFFSET: usize = 13;
    pub const OUTPUT: usize = 14;
    pub const MIX: usize = 15;
    pub(crate) const COUNT: usize = 16;
}

/// What the device hands its panel after each block (`Device::readouts`): where the LFO has
/// moved the frequency and the morph on each side, so the display draws the filter as it is.
pub mod readout {
    pub const FREQUENCY_LEFT: usize = 0;
    pub const FREQUENCY_RIGHT: usize = 1;
    pub const MORPH_LEFT: usize = 2;
    pub const MORPH_RIGHT: usize = 3;
}

/// Options of the synced rate, and their length in beats.
const SYNCED_RATES: [(&str, f64); 15] = [
    ("8 Bars", 32.0),
    ("4 Bars", 16.0),
    ("2 Bars", 8.0),
    ("1 Bar", 4.0),
    ("1/2", 2.0),
    ("1/4 Dotted", 1.5),
    ("1/2 Triplet", 4.0 / 3.0),
    ("1/4", 1.0),
    ("1/8 Dotted", 0.75),
    ("1/4 Triplet", 2.0 / 3.0),
    ("1/8", 0.5),
    ("1/8 Triplet", 1.0 / 3.0),
    ("1/16", 0.25),
    ("1/16 Triplet", 1.0 / 6.0),
    ("1/32", 0.125),
];
const SYNCED_RATE_NAMES: [&str; 15] = {
    let mut names = [""; 15];
    let mut i = 0;
    while i < SYNCED_RATES.len() {
        names[i] = SYNCED_RATES[i].0;
        i += 1;
    }
    names
};
/// Index of "1/4" in [`SYNCED_RATES`].
const DEFAULT_SYNCED_RATE: usize = 7;
/// The LFO's shapes: the shared ones, then a smoothly wandering random.
const SHAPE_NAMES: [&str; 7] = ["Sine", "Triangle", "Saw Up", "Saw Down", "Square", "Random", "Smooth Random"];
/// Index of "Smooth Random" in [`SHAPE_NAMES`].
const SMOOTH_RANDOM: usize = 6;

static PARAMS: [ParamInfo; param::COUNT] = [
    ParamInfo::choice("type", "Type", &TYPE_NAMES, kind::LOW_PASS_12),
    ParamInfo::continuous("frequency", "Frequency", curve::FREQUENCY_RANGE, 1_000.0, Unit::Hertz, Scale::Logarithmic),
    // 0.1 is about Q 0.72: a plain, unresonant corner.
    ParamInfo::continuous("resonance", "Resonance", (0.0, 1.0), 0.1, Unit::Percent, Scale::Linear),
    ParamInfo::continuous("gain", "Gain", (-24.0, 24.0), 0.0, Unit::Decibels, Scale::Linear),
    ParamInfo::continuous("morph", "Morph", (0.0, 1.0), 0.0, Unit::Percent, Scale::Linear),
    ParamInfo::continuous("drive", "Drive", (0.0, 1.0), 0.0, Unit::Percent, Scale::Linear),
    ParamInfo::in_group("LFO").continuous(
        "lfoAmount",
        "LFO Amount",
        (-MAX_LFO_OCTAVES, MAX_LFO_OCTAVES),
        0.0,
        Unit::Octaves,
        Scale::Linear,
    ),
    ParamInfo::in_group("LFO").continuous("lfoMorph", "LFO to Morph", (-1.0, 1.0), 0.0, Unit::Percent, Scale::Linear),
    ParamInfo::in_group("LFO").toggle("lfoSync", "LFO Sync", false),
    ParamInfo::in_group("LFO").continuous("lfoRate", "LFO Rate", (0.01, 40.0), 1.0, Unit::Hertz, Scale::Logarithmic),
    ParamInfo::in_group("LFO").choice("lfoSyncedRate", "LFO Synced Rate", &SYNCED_RATE_NAMES, DEFAULT_SYNCED_RATE),
    ParamInfo::in_group("LFO").choice("lfoShape", "LFO Shape", &SHAPE_NAMES, 0),
    // In cycles: how far the right side's LFO runs ahead of the left's.
    ParamInfo::in_group("LFO").continuous("lfoPhase", "LFO Stereo Phase", (0.0, 1.0), 0.0, Unit::None, Scale::Linear),
    // In cycles: where in its cycle the LFO is at time (or beat) zero.
    ParamInfo::in_group("LFO").continuous("lfoOffset", "LFO Offset", (0.0, 1.0), 0.0, Unit::None, Scale::Linear),
    ParamInfo::continuous("output", "Output", (-24.0, 24.0), 0.0, Unit::Decibels, Scale::Linear),
    ParamInfo::continuous("mix", "Dry/Wet", (0.0, 1.0), 1.0, Unit::Percent, Scale::Linear),
];

pub static INFO: DeviceInfo = DeviceInfo::new("filter", "Filter", Role::Effect, &PARAMS);

/// How hard full drive pushes the input into the saturator.
const MAX_DRIVE_GAIN: f64 = 10.0;

/// One side's filter state: every structure a type might run, so switching types allocates
/// nothing.
#[derive(Debug, Clone)]
struct Channel {
    sections: [Svf; 3],
    stages: [Svf; PHASER_STAGES],
    comb: Comb,
    /// The phaser's last output, fed back into its input.
    phaser_last: f64,
}

impl Channel {
    fn new(sample_rate: f64) -> Self {
        Self {
            sections: [Svf::default(); 3],
            stages: [Svf::default(); PHASER_STAGES],
            comb: Comb::new(sample_rate),
            phaser_last: 0.0,
        }
    }

    fn reset(&mut self) {
        self.sections.iter_mut().for_each(Svf::reset);
        self.stages.iter_mut().for_each(Svf::reset);
        self.comb.reset();
        self.phaser_last = 0.0;
    }

    /// One sample through what `plan` runs.
    #[inline]
    fn process(&mut self, x: f64, plan: &Plan) -> f64 {
        match plan.kind {
            kind::COMB_PLUS | kind::COMB_MINUS => self.comb.process(x, plan.frequency, plan.feedback),
            kind::PHASER => {
                let mut w = x + plan.feedback * self.phaser_last;
                for stage in &mut self.stages {
                    stage.set(plan.sections[0]);
                    w = stage.process(w);
                }
                self.phaser_last = if w.is_finite() { w } else { 0.0 };
                0.5 * (x + self.phaser_last)
            }
            kind::FORMANT => {
                let mut sum = 0.0;
                for ((section, coefficients), gain) in
                    self.sections.iter_mut().zip(plan.sections).zip(curve::formant_gains())
                {
                    section.set(coefficients);
                    sum += gain * section.process(x);
                }
                sum
            }
            _ => {
                let mut y = x;
                for (section, coefficients) in self.sections.iter_mut().zip(plan.sections).take(plan.count) {
                    section.set(coefficients);
                    y = section.process(y);
                }
                y
            }
        }
    }
}

pub struct Filter {
    sample_rate: f64,
    params: ParamBank<{ param::COUNT }>,
    /// The frequency in octaves (log2 of hertz).
    octaves: Smoother,
    resonance: Smoother,
    gain: Smoother,
    morph: Smoother,
    drive: Smoother,
    output: Smoother,
    mix: Smoother,
    channels: [Channel; 2],
    /// The type the channels last ran, so a change of type starts from silence.
    running: usize,
    /// Free-running position: whole cycles and the fraction of the current one.
    free_cycles: u64,
    free_phase: f64,
    readouts: [f32; MAX_READOUTS],
}

impl Default for Filter {
    fn default() -> Self {
        Self::new()
    }
}

/// `dB` as a linear gain.
fn linear(db: f64) -> f32 {
    10f64.powf(db / 20.0) as f32
}

impl Filter {
    pub fn new() -> Self {
        Self::at(DEFAULT_SAMPLE_RATE)
    }

    /// The same, made for `sample_rate` (ADR 0004 › Amendment: a device is made at its rate);
    /// fails for a rate a device cannot run at. Allocates.
    pub fn with_rate(sample_rate: f64) -> Result<Self, DeviceError> {
        check_sample_rate(sample_rate).map(Self::at)
    }

    /// Everything for `sample_rate`, which has been checked.
    fn at(sample_rate: f64) -> Self {
        let params = ParamBank::new(&PARAMS);
        let ramp = Smoother::ramp_frames(RAMP_SECONDS, sample_rate);
        let smoother = |value: f64| Smoother::new(value as f32, ramp);
        let mut filter = Self {
            sample_rate,
            octaves: smoother(params.get(param::FREQUENCY).log2()),
            resonance: smoother(params.get(param::RESONANCE)),
            gain: smoother(params.get(param::GAIN)),
            morph: smoother(params.get(param::MORPH)),
            drive: smoother(params.get(param::DRIVE)),
            output: Smoother::new(linear(params.get(param::OUTPUT)), ramp),
            mix: smoother(params.get(param::MIX)),
            params,
            channels: [Channel::new(sample_rate), Channel::new(sample_rate)],
            running: kind::LOW_PASS_12,
            free_cycles: 0,
            free_phase: 0.0,
            readouts: [0.0; MAX_READOUTS],
        };
        filter.reset();
        filter
    }

    fn value(&self, index: usize) -> f64 {
        self.params.get(index)
    }

    fn kind(&self) -> usize {
        (self.value(param::TYPE).round().max(0.0) as usize).min(TYPE_NAMES.len() - 1)
    }

    /// Length of one synced cycle in beats.
    fn synced_beats(&self) -> f64 {
        SYNCED_RATES.get(self.value(param::LFO_SYNCED_RATE) as usize).map_or(1.0, |r| r.1)
    }

    /// The LFO, -1 to 1, at position `cycles`.
    fn lfo(shape: usize, cycles: f64) -> f64 {
        if shape == SMOOTH_RANDOM {
            return wander(cycles);
        }
        wave(Shape::ALL.get(shape).copied().unwrap_or(Shape::Sine), cycles)
    }

    fn smoothers(&mut self) -> [&mut Smoother; 7] {
        [
            &mut self.octaves,
            &mut self.resonance,
            &mut self.gain,
            &mut self.morph,
            &mut self.drive,
            &mut self.output,
            &mut self.mix,
        ]
    }
}

impl Device for Filter {
    fn info(&self) -> &DeviceInfo {
        &INFO
    }

    fn prepare(&mut self, sample_rate: f64) -> Result<(), DeviceError> {
        let sr = check_sample_rate(sample_rate)?;
        self.sample_rate = sr;
        let ramp = Smoother::ramp_frames(RAMP_SECONDS, sr);
        for smoother in self.smoothers() {
            smoother.set_ramp(ramp);
        }
        self.channels = [Channel::new(sr), Channel::new(sr)];
        self.reset();
        Ok(())
    }

    fn reset(&mut self) {
        for smoother in self.smoothers() {
            smoother.reset(smoother.target());
        }
        self.channels.iter_mut().for_each(Channel::reset);
        self.running = self.kind();
        self.free_cycles = 0;
        self.free_phase = 0.0;
        let (frequency, morph) = (self.value(param::FREQUENCY) as f32, self.value(param::MORPH) as f32);
        self.readouts = [0.0; MAX_READOUTS];
        self.readouts[readout::FREQUENCY_LEFT] = frequency;
        self.readouts[readout::FREQUENCY_RIGHT] = frequency;
        self.readouts[readout::MORPH_LEFT] = morph;
        self.readouts[readout::MORPH_RIGHT] = morph;
    }

    fn set_param(&mut self, index: usize, value: f64) {
        let Some(value) = self.params.set(index, value) else { return };
        let target = match index {
            param::FREQUENCY => (&mut self.octaves, value.log2() as f32),
            param::RESONANCE => (&mut self.resonance, value as f32),
            param::GAIN => (&mut self.gain, value as f32),
            param::MORPH => (&mut self.morph, value as f32),
            param::DRIVE => (&mut self.drive, value as f32),
            param::OUTPUT => (&mut self.output, linear(value)),
            param::MIX => (&mut self.mix, value as f32),
            _ => return,
        };
        target.0.set_target(target.1);
    }

    fn param(&self, index: usize) -> Option<f64> {
        self.params.param(index)
    }

    fn readouts(&self) -> Option<[f32; MAX_READOUTS]> {
        Some(self.readouts)
    }

    fn process(&mut self, context: &ProcessContext, buffer: &mut [f32]) {
        let kind = self.kind();
        if kind != self.running {
            // The structures of different types hold unrelated state: start the new one clean.
            self.channels.iter_mut().for_each(Channel::reset);
            self.running = kind;
        }
        let synced = self.value(param::LFO_SYNC) >= 0.5;
        let clock = LfoClock::new(
            context.tempo_or_default(),
            self.sample_rate,
            synced.then(|| self.synced_beats()),
            self.value(param::LFO_RATE),
        )
        .following(context.playing, context.beat);
        let cycles_per_frame = clock.cycles_per_frame();
        let shape = self.value(param::LFO_SHAPE) as usize;
        let (amount, to_morph) = (self.value(param::LFO_AMOUNT), self.value(param::LFO_MORPH));
        let (offset, phase) = (self.value(param::LFO_OFFSET), self.value(param::LFO_PHASE));
        let modulated = amount != 0.0 || to_morph != 0.0;
        let sample_rate = self.sample_rate;
        let mut last = [(0.0, 0.0); 2];

        for (n, frame) in buffer.as_chunks_mut::<2>().0.iter_mut().enumerate() {
            let position = clock.song_cycles(n).unwrap_or(self.free_cycles as f64 + self.free_phase);
            let octaves = f64::from(self.octaves.next_value());
            let resonance = f64::from(self.resonance.next_value());
            let gain = f64::from(self.gain.next_value());
            let morph = f64::from(self.morph.next_value());
            let drive = f64::from(self.drive.next_value()).clamp(0.0, 1.0);
            let output = self.output.next_value();
            let mix = self.mix.next_value();

            let lfo = if modulated {
                [Self::lfo(shape, position + offset), Self::lfo(shape, position + offset + phase)]
            } else {
                [0.0; 2]
            };
            let settings = lfo.map(|lfo| Settings {
                kind,
                frequency: (octaves + amount * lfo).exp2(),
                resonance,
                gain,
                morph: (morph + to_morph * lfo).clamp(0.0, 1.0),
            });
            let left = Plan::new(&settings[0], sample_rate);
            let right = if settings[1] == settings[0] { left } else { Plan::new(&settings[1], sample_rate) };

            for ((sample, channel), plan) in frame.iter_mut().zip(&mut self.channels).zip([&left, &right]) {
                let dry = *sample;
                let mut x = f64::from(dry);
                if drive > 0.0 {
                    let pushed = (x * (1.0 + (MAX_DRIVE_GAIN - 1.0) * drive)).tanh();
                    x += drive * (pushed - x);
                }
                let wet = channel.process(x, plan) as f32 * output;
                *sample = dry + (wet - dry) * mix;
            }
            last = [0, 1].map(|i| (settings[i].frequency, settings[i].morph));

            if !clock.follows_song() {
                self.free_phase += cycles_per_frame;
                if self.free_phase >= 1.0 {
                    self.free_cycles = self.free_cycles.wrapping_add(self.free_phase as u64) % (1 << 32);
                    self.free_phase = self.free_phase.fract();
                }
            }
        }
        if buffer.len() >= 2 {
            self.readouts[readout::FREQUENCY_LEFT] = last[0].0 as f32;
            self.readouts[readout::FREQUENCY_RIGHT] = last[1].0 as f32;
            self.readouts[readout::MORPH_LEFT] = last[0].1 as f32;
            self.readouts[readout::MORPH_RIGHT] = last[1].1 as f32;
        }
    }
}

#[cfg(test)]
mod tests;
