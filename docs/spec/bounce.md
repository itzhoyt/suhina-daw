---
description: Bouncing a track to a new audio track - rendering an audio or MIDI track's arrangement with its devices and plug-ins to a WAV file next to the project, placed on a new audio track below it that takes over the original's mixer settings while the original is muted
read_when: bounce, bounce to new track, track.bounce, resample, resampling, render a track to audio, print a track, Bounced folder, freeze, flatten, render in place, pre-fader render
type: spec
status: current
last_verified: 2026-11-17
covers:
  - crates/session/src/bounce.rs
  - crates/session/tests/bounce.rs
---

# Bouncing a track

Bouncing renders what one track plays, through its own devices and plug-ins, to an audio file
and puts that file on a new audio track, so an instrument or an effect chain becomes plain audio
to edit, slice or resample further. It is the first of the resampling features; freezing a
track in place and recording another track's output live are [not yet](#not-yet) built.

## `track.bounce`

`track.bounce {track, start?, end?, tail?}` renders `track` and returns
`{track, clip, file, source, start, end, peak, warnings}`: the new track's id, the new clip's id,
the file written, the bounced track's id, the range in beats, the file's peak and what the
render could not do. It is one undo step.

- **What is bounced:** an audio or a MIDI track. A group track, a return track and the main
  channel are refused (`"only an audio or a MIDI track can be bounced"`).
- **The range:** `start … end` in beats. Left out, it runs from the bar the track's first
  enabled arrangement clip starts in to the end of its last one. A track with no enabled clip
  on its arrangement lane is refused (`"track \"2 MIDI\" has no clips in the arrangement to
  bounce"`); clips only in the Launcher are not bounced. `end` must be at least one beat after
  `start`, both within `0 … 120 000`, and the render is capped at an hour, as an export's range
  is ([export.md](export.md)).
- **The tail:** `tail` beats rendered past `end`, so a reverb or a delay rings out into the
  file; 0 to 64, and one bar of the project's time signature when left out. The new clip covers
  the range and the tail.
- **The render** is the export's node render ([export.md](export.md) › Sources) of `track`,
  pre-fader: the track's clips and its devices (built-in and plug-ins, started as an export
  starts them, with their state as heard) are in; the track's volume, pan and mute, and their
  automation, are left out (rendered at 0 dB, centred, unmuted), and so is solo anywhere in the
  project. Device automation is in. The arrangement's loop region does not wrap it. It renders
  at the engine's sample rate, or 48 kHz without an audio device. The delay latency
  compensation adds to every track is taken off the file's start, so the file starts on its
  beat and the new track, compensated again when it plays, lines up with the rest of the mix.

The project sounds the same before and after a bounce, apart from the click guard at the new
clip's start when the track is playing there, as after a consolidation
([arrangement-editing.md](arrangement-editing.md) › Consolidating).
- **The file** is a 32-bit float stereo WAV in a folder **`Bounced`** next to the project file,
  named after the track (`2 MIDI bounce.wav`; ` 2`, ` 3` … are added when that file exists). A
  project that has not been saved has no folder for it, so the command asks to save it first:
  `"save the project first: bounced audio is written to a folder next to the project file"`.
  Undo leaves the file on disk.
- **The new track** is an audio track right after the bounced one, in its group, named after it
  with ` bounce` added (`2 MIDI bounce`), with no audio input. It takes over the bounced track's
  colour, volume, pan, output and sends, and the automation lanes of its volume, pan and mute,
  so the mix sounds as it did. Its one clip starts at `start`, plays the file unwarped at 0 dB
  with no fades, and is named after the track.
- **The bounced track is muted**, its mute lane turned off if it has one, so the part is not
  heard twice. Its clips and devices are left as they are, to edit and bounce again.

Warnings list the plug-ins that did not start or take their state and the files its clips name
that cannot be found, as an export's do.

## In the app

The right-click menu of an audio or MIDI track's header, in the Arrangement view and on a
Launcher column, has **Bounce to new track**. It runs `track.bounce` on that track with the
default range. The new track is selected; an error (an unsaved project, a track with no clips)
is shown in the status bar.

## Not yet

- Freezing a track: playing a rendered file in place of its devices until it is unfrozen.
- Bouncing in place: replacing a track's clips and devices with the rendered audio.
- Recording another track's or the main output live into an audio track (a track's input taken
  from the mix rather than the audio device).
- Bouncing a group track, or a time selection from the Arrangement view's menu.
