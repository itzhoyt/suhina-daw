// Copyright (C) 2026 The Suhina contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Bouncing a track to a new audio track (`docs/spec/bounce.md`). The offline render is the
//! oracle: the project sounds the same after a bounce, the bounced track muted and the new one
//! playing its file through the same mixer settings.
#![allow(clippy::unwrap_used, clippy::panic)] // test helpers outside #[test] functions

use std::path::{Path, PathBuf};
use suhina_engine::{RenderInputs, render_offline_from};

use serde_json::json;
use suhina_model::*;
use suhina_session::{Session, blank_project};

use crate::support::run;
use suhina_test_support::{WavFormat, write_wav};

const SR: u32 = 48_000;

fn render(session: &Session) -> Vec<f32> {
    render_offline_from(
        session.project(),
        RenderInputs::from_audio(session.audio(), suhina_session::devices()),
        SR,
        512,
        None,
    )
    .unwrap()
}

/// The click guard a clip starts with, in frames: what a new clip at the bounce's start adds.
const GUARD: usize = 600;

/// The largest difference between two renders over the frames both have, past the first
/// [`GUARD`] frames after frame `start` (where the new clip starts).
fn max_diff(a: &[f32], b: &[f32], start: usize) -> f32 {
    let guard = start * 2..(start + GUARD) * 2;
    a.iter().zip(b).enumerate().filter(|(i, _)| !guard.contains(i)).map(|(_, (x, y))| (x - y).abs()).fold(0.0, f32::max)
}

fn peak(a: &[f32]) -> f32 {
    a.iter().fold(0.0, |m, s| m.max(s.abs()))
}

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("suhina-bounce-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A mono WAV file of `seconds` of a sine at `hz`.
fn sine(dir: &Path, name: &str, seconds: f64, hz: f64) -> PathBuf {
    let path = dir.join(name);
    let frames = (seconds * f64::from(SR)) as usize;
    write_wav(&path, SR, 1, WavFormat::Float32, &suhina_test_support::sine(hz, f64::from(SR), frames, 0.5));
    path
}

/// A saved blank project in `dir` with a sine on track 3 ("3 Audio") from beat 4.
fn audio_session(dir: &Path) -> Session {
    let tone = sine(dir, "tone.wav", 2.0, 330.0);
    let mut session = Session::new(blank_project());
    run(&mut session, "clip.addFiles", json!({ "paths": [tone.display().to_string()], "track": 3, "start": 4.0 }))
        .unwrap();
    session.save(&dir.join("song.suhina"), false).unwrap();
    session
}

fn channel_of(session: &Session, track: u32) -> Channel {
    let p = session.project();
    let id = p.track(TrackId(track)).and_then(|t| t.channel).unwrap();
    p.channel(id).unwrap().clone()
}

fn lane_clips(session: &Session, track: u32) -> Vec<Clip> {
    let p = session.project();
    p.arrangement.lanes.iter().find(|l| l.track == Some(TrackId(track))).unwrap().clips.clone()
}

#[test]
fn a_bounced_audio_track_sounds_the_same_on_its_new_track() {
    let dir = temp_dir("audio");
    let mut session = audio_session(&dir);
    // A device, volume and pan: the device is rendered in, the volume and pan are taken over.
    run(&mut session, "device.add", json!({ "track": 3, "id": "overdrive" })).unwrap();
    run(&mut session, "track.setVolume", json!({ "tracks": [3], "db": -6 })).unwrap();
    run(&mut session, "track.setPan", json!({ "tracks": [3], "pan": -0.4 })).unwrap();
    run(&mut session, "track.setColor", json!({ "track": 3, "color": "#3366cc" })).unwrap();
    let before = render(&session);
    assert!(peak(&before) > 0.05, "the track plays");

    let out = run(&mut session, "track.bounce", json!({ "track": 3 })).unwrap();
    let new = out["track"].as_u64().unwrap() as u32;
    assert_eq!(out["source"], 3);
    // From the bar the clip starts in to its end, and one bar of tail.
    assert_eq!((out["start"].as_f64(), out["end"].as_f64()), (Some(4.0), Some(8.0 + 4.0)));
    let file = PathBuf::from(out["file"].as_str().unwrap());
    assert_eq!(file.parent(), Some(dir.join("Bounced").as_path()));
    assert_eq!(file.file_name().and_then(|n| n.to_str()), Some("3 Audio bounce.wav"));
    let decoded = suhina_media::decode_file(&file).unwrap();
    assert_eq!((decoded.sample_rate, decoded.channels), (SR, 2));
    assert!(out["peak"].as_f64().unwrap() > 0.05);

    // The new track comes right after the bounced one, with its colour and mixer settings.
    let p = session.project();
    let at = p.tracks.iter().position(|t| t.id == TrackId(3)).unwrap();
    let track = &p.tracks[at + 1];
    assert_eq!(track.id, TrackId(new));
    assert_eq!(track.name, "3 Audio bounce");
    assert!(track.content.audio && !track.content.notes);
    assert_eq!(track.color, p.track(TrackId(3)).unwrap().color);
    let (old, made) = (channel_of(&session, 3), channel_of(&session, new));
    assert!(old.mute, "the bounced track is muted");
    assert!(!made.mute);
    assert_eq!((made.volume, made.pan, made.output), (old.volume, old.pan, old.output));
    assert!(made.devices.is_empty());
    // Its old clips and devices are left as they were.
    assert_eq!(lane_clips(&session, 3).len(), 1);
    assert_eq!(old.devices.len(), 1);

    let clips = lane_clips(&session, new);
    assert_eq!(clips.len(), 1);
    assert_eq!(clips[0].id.0, out["clip"].as_u64().unwrap() as u32);
    assert_eq!((clips[0].start, clips[0].length), (4.0, 8.0));
    assert!(!clips[0].audio.warped);

    // Beat 4 at 120 BPM.
    let after = render(&session);
    assert!(max_diff(&before, &after, 2 * SR as usize) < 1e-5, "{}", max_diff(&before, &after, 2 * SR as usize));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_bounced_midi_track_plays_its_instrument_as_audio() {
    let dir = temp_dir("midi");
    let mut session = Session::new(blank_project());
    run(&mut session, "device.add", json!({ "track": 1, "id": "fm-synth" })).unwrap();
    let made = run(&mut session, "clip.create", json!({ "track": 1, "start": 0.0, "length": 4.0 })).unwrap();
    let clip = made["clip"].clone();
    let notes = json!([{ "time": 0, "length": 1, "key": 60 }, { "time": 2, "length": 1, "key": 67 }]);
    run(&mut session, "note.add", json!({ "clip": clip, "notes": notes })).unwrap();
    session.save(&dir.join("song.suhina"), false).unwrap();
    let before = render(&session);
    assert!(peak(&before) > 0.01, "the instrument plays");

    let out = run(&mut session, "track.bounce", json!({ "track": 1, "tail": 0 })).unwrap();
    let new = out["track"].as_u64().unwrap() as u32;
    assert_eq!((out["start"].as_f64(), out["end"].as_f64()), (Some(0.0), Some(4.0)));
    assert!(out["file"].as_str().unwrap().ends_with("1 FM Synth bounce.wav"), "{}", out["file"]);
    assert!(channel_of(&session, 1).mute);
    let after = render(&session);
    assert!(max_diff(&before, &after, 0) < 1e-5, "{}", max_diff(&before, &after, 0));

    // Undo takes the new track away and unmutes the old one; the file stays. Redo brings it back.
    run(&mut session, "edit.undo", json!({})).unwrap();
    assert!(session.project().track(TrackId(new)).is_none());
    assert!(!channel_of(&session, 1).mute);
    assert!(PathBuf::from(out["file"].as_str().unwrap()).is_file(), "undo leaves the file on disk");
    run(&mut session, "edit.redo", json!({})).unwrap();
    assert!(session.project().track(TrackId(new)).is_some());

    // A second bounce of the same track does not overwrite the first file.
    let again = run(&mut session, "track.bounce", json!({ "track": 1, "tail": 0 })).unwrap();
    assert!(again["file"].as_str().unwrap().ends_with("1 FM Synth bounce 2.wav"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_range_and_a_tail_set_what_is_rendered() {
    let dir = temp_dir("range");
    let mut session = audio_session(&dir);
    let out = run(&mut session, "track.bounce", json!({ "track": 3, "start": 6.0, "end": 8.0, "tail": 2 })).unwrap();
    assert_eq!((out["start"].as_f64(), out["end"].as_f64()), (Some(6.0), Some(10.0)));
    let decoded = suhina_media::decode_file(&PathBuf::from(out["file"].as_str().unwrap())).unwrap();
    // Four beats at 120 BPM.
    assert_eq!(decoded.frames(), 2 * SR as usize);
    let clips = lane_clips(&session, out["track"].as_u64().unwrap() as u32);
    assert_eq!((clips[0].start, clips[0].length), (6.0, 4.0));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_bounce_ignores_solo_elsewhere() {
    let dir = temp_dir("solo");
    let mut session = audio_session(&dir);
    run(&mut session, "track.setSolo", json!({ "tracks": [1], "solo": true })).unwrap();
    let out = run(&mut session, "track.bounce", json!({ "track": 3 })).unwrap();
    assert!(out["peak"].as_f64().unwrap() > 0.05, "a solo on another track does not silence the bounce");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn what_cannot_be_bounced_is_refused() {
    let dir = temp_dir("refused");
    let mut session = audio_session(&dir);
    let before = session.project().clone();
    let error = |session: &mut Session, params: serde_json::Value| {
        run(session, "track.bounce", params).unwrap_err().to_string()
    };
    assert!(error(&mut session, json!({ "track": 4 })).contains("has no clips in the arrangement"));
    assert!(error(&mut session, json!({ "track": "A Return" })).contains("only an audio or a MIDI track"));
    assert!(error(&mut session, json!({ "track": 99 })).contains("does not exist"));
    assert!(error(&mut session, json!({})).contains("track"));
    assert!(error(&mut session, json!({ "track": 3, "start": 4.0, "end": 4.5 })).contains("at least one beat"));
    assert!(error(&mut session, json!({ "track": 3, "start": -1.0 })).contains("from 0"));
    assert!(error(&mut session, json!({ "track": 3, "tail": 65 })).contains("tail must be"));
    assert!(error(&mut session, json!({ "track": 3, "tail": "long" })).contains("tail"));
    run(&mut session, "track.group", json!({ "tracks": [3], "name": "Group" })).unwrap();
    let group = session.project().tracks.iter().find(|t| t.name == "Group").unwrap().id.0;
    assert!(error(&mut session, json!({ "track": group })).contains("only an audio or a MIDI track"));
    run(&mut session, "edit.undo", json!({})).unwrap();
    assert_eq!(session.project(), &before, "a refused bounce changes nothing");
    assert!(!dir.join("Bounced").exists());

    // An unsaved project has no folder for the file.
    let tone = sine(&dir, "tone2.wav", 1.0, 440.0);
    let mut unsaved = Session::new(blank_project());
    run(&mut unsaved, "clip.addFiles", json!({ "paths": [tone.display().to_string()], "track": 3, "start": 0.0 }))
        .unwrap();
    assert!(error(&mut unsaved, json!({ "track": 3 })).contains("save the project first"));
    let _ = std::fs::remove_dir_all(&dir);
}
