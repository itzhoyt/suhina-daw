---
description: The built-in Filter - its fourteen types (low-pass, high-pass, band-pass, notch, peak, shelves, morph, combs, phaser, formant), drive, the LFO on the frequency and the morph, free or synced, and the panel's response display with the LFO's range, the live curve and the frequency/resonance dot
read_when: filter, multimode filter, multi filter, filter device, low-pass, high-pass, band-pass, notch, comb filter, phaser, formant filter, vowel filter, morph filter, filter LFO, filter sweep, resonance, cutoff, filter panel, filter response display, FilterCurve
type: spec
status: current
last_verified: 2026-11-17
covers:
  - devices/filter/**
  - crates/dsp/src/comb.rs
  - crates/ui/src/detail/filter_curve.rs
---

# Filter

Suhina's `filter` (`devices/filter`): one multimode filter with drive and a built-in LFO. It is
Suhina's own design, not modelled on another product's device, so nothing here is observed: the
responses are the textbook ones, from the shared blocks in `crates/dsp`.

- **Sources:** the Audio EQ Cookbook (R. Bristow-Johnson) and A. Simper's papers on the linear
  trapezoidal state-variable filter, through `dsp::svf` as the EQ uses it; the feedback comb and
  the all-pass phaser are standard textbook structures; the vowel formants are the published
  averages of G. E. Peterson and H. L. Barney, *Control Methods Used in a Study of the Vowels*
  (1952).

## Parameters

Ids are stable: a project stores each value by its id, and automation names it by its id.

| Id | Name | Range | Default | What it does |
|---|---|---|---|---|
| `type` | Type | 14 choices | Low-pass 12 | The response, below. |
| `frequency` | Frequency | 20 Hz – 20 kHz, logarithmic | 1 kHz | The corner, centre, or (comb) the tuning. |
| `resonance` | Resonance | 0 – 100 % | 10 % | Q from 0.5 to 20, evenly in ratio (10 % is about 0.72); the comb's and the phaser's feedback; the formants' sharpness. |
| `gain` | Gain | ±24 dB | 0 dB | The peak's and the shelves' boost or cut; the other types ignore it. |
| `morph` | Morph | 0 – 100 % | 0 % | Morph: low-pass, band-pass (25 %), high-pass (50 %), notch (75 %), low-pass. Formant: the vowel, A, E, I, O, U. |
| `drive` | Drive | 0 – 100 % | 0 % | Saturates the input (tanh, up to 10× into it) before the filter, crossfaded in, so 0 % is exactly clean. |
| `lfoAmount` | LFO Amount | ±4 octaves | 0 | How far the LFO moves the frequency either way; negative turns it upside down. |
| `lfoMorph` | LFO to Morph | ±100 % | 0 % | How far the LFO moves the morph (or the vowel). |
| `lfoSync` | LFO Sync | off/on | off | Free in hertz, or synced to the song in note lengths. |
| `lfoRate` | LFO Rate | 0.01 – 40 Hz, logarithmic | 1 Hz | The free rate. |
| `lfoSyncedRate` | LFO Synced Rate | 8 bars … 1/32, dotted and triplets | 1/4 | The synced cycle, the Auto Pan's table. |
| `lfoShape` | LFO Shape | Sine, Triangle, Saw Up, Saw Down, Square, Random, Smooth Random | Sine | Random holds a new value each cycle; Smooth Random glides between them. |
| `lfoPhase` | LFO Stereo Phase | 0 – 1 cycle | 0 | How far the right side's LFO runs ahead of the left's: 0.5 sweeps the sides in opposite directions. |
| `lfoOffset` | LFO Offset | 0 – 1 cycle | 0 | Where in its cycle the LFO is at time (or beat) zero. |
| `output` | Output | ±24 dB | 0 dB | Gain after the filter (the wet signal). |
| `mix` | Dry/Wet | 0 – 100 % | 100 % | `dry + (wet - dry) * mix`. |

The LFO group shows under "LFO" in the parameter list.

## The types

All but the combs run trapezoidal state-variable sections (`dsp::Svf`), whose state stays
meaningful as the coefficients move, so the frequency sweeps without bumps.

- **Low-pass 12, High-pass 12, Band-pass, Notch:** one section at the resonance's Q.
- **Low-pass 24, High-pass 24:** two sections, each at the square root of the Q, so the slope
  steepens and the resonance's peak stays as high as at 12.
- **Peak, Low Shelf, High Shelf:** one cookbook section with the gain.
- **Morph:** one section through low-pass, band-pass, high-pass and notch (`SvfCoefficients::morph`).
- **Comb +:** a feedback comb tuned to the frequency (`dsp::Comb`): peaks at it and every
  multiple, feedback from 0.3 at no resonance to 0.99 at full. **Comb -:** the feedback negated,
  so the peaks fall at the odd multiples of half the frequency (an octave lower, hollow). The
  output is scaled by `1 - |feedback|`, so the peaks stand at unity and only the gaps deepen. Its
  line reaches 20 Hz.
- **Phaser:** six all-pass sections at the frequency (Q 0.5) with feedback up to 0.9, the
  result mixed half and half with the input: three notches that move with the frequency.
- **Formant:** three band-passes in parallel at the vowel's first three formants (levels 1,
  0.5, 0.35), Q 3 to 20 with the resonance. The frequency shifts all three by its ratio to 1 kHz,
  so 1 kHz is the vowels' own pitch.

Changing the type clears the filter's state: the types hold unrelated state, and carrying one's
into another could pop.

## Smoothing and the LFO

Frequency (in octaves), resonance, gain, morph, drive, output and dry/wet ramp over 20 ms to each
new value, so automation does not zip. The LFO moves the frequency in octaves around the smoothed
value, and the morph around its own, once per frame, on each side with its own phase. Free, the
LFO runs on from where it was; synced, it follows the song position (`dsp::sync::LfoClock`), so it
is at the same point at the same beat however playback got there, and keeps running at the tempo
while the transport is stopped. A bar is four beats.

After each block the device hands its panel where the LFO has the frequency and the morph on
each side (`Device::readouts`: frequency left and right, morph left and right).

## The panel

708 × 168: the Type menu over the response display (`Widget::FilterCurve`, host-drawn), the
filter's dials (Freq, Res, Gain, Morph; Drive, Output, Dry/Wet), a divider, then the LFO's (LFO
Amt, Rate or the synced rate's menu, To Morph; Stereo, Offset) beside the Sync switch and the
Shape menu.

The display draws from `filter::curve::Plan`, the same maths the audio runs, at 48 kHz:

- the grid: 20 Hz – 20 kHz on a logarithmic axis, +30 to -42 dB, a line every 12 dB;
- the curve as the filter sounds now, filled under it: the parameters' values, moved by the
  LFO's readouts while the LFO runs;
- while the LFO runs, the range it sweeps as a shaded band (the lowest and highest of nine
  curves across the LFO's swing), the parameters' own curve dashed, and with a stereo phase the
  right side's curve dimmer;
- the type and the live frequency at the top right;
- a dot at the frequency: dragged sideways for the frequency, up and down for the resonance, or
  for the peak's and the shelves' gain; Shift for precision; double-click resets both. A drag is
  one gesture of `device.setParam` commands, so it is one undo step and records as automation
  like any other edit.

The canvas animates (repaints) while the LFO amount or LFO to Morph is not zero. The SDK's
validator lets only a parameter set with `type`, `frequency`, `resonance`, `gain` and `morph`
claim the widget.

## Decisions

- **One LFO destination pair, no envelope follower.** The frequency and the morph are what a
  sweep needs; an envelope follower or MIDI-triggered envelope can come later as parameters
  appended after `mix`.
- **No oversampling.** Drive is a gentle tanh crossfaded in; at full drive on bright material
  some aliasing is audible, the price of zero latency.
