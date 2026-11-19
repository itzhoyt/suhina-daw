// Copyright (C) 2026 The Suhina contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! `track.bounce`: an audio or MIDI track rendered pre-fader, through its own devices and
//! plug-ins, to a WAV file next to the project and placed on a new audio track below it
//! (`docs/spec/bounce.md`). The render is the export's node render, the same `process` as
//! playback.

use std::path::Path;

use serde_json::{Value, json};
use suhina_engine::{RenderInputs, RenderOptions, Renderer};
use suhina_model::{
    AudioRegion, Clip, ClipContent, ContentId, ContentLane, LaneData, Project, TrackId, automation::MIXER_PARAMS,
};

use crate::commands::{Args, track_id};
use crate::editing::{free_path, sanitized, write_wav};
use crate::files::{TrackKind, insert_track};
use crate::{CommandError, Session};

/// The folder next to the project file that bounced audio is written to.
pub(crate) const BOUNCED_FOLDER: &str = "Bounced";
/// The sample rate a bounce renders at without an audio device.
const DEFAULT_RATE: u32 = 48_000;
/// The longest tail, in beats.
const MAX_TAIL: f64 = 64.0;
/// Frames per call to the renderer.
const BLOCK_FRAMES: usize = 512;
/// The render's samples are capped by bytes, as an export's are.
const MAX_RENDER_BYTES: usize = 2 * 1024 * 1024 * 1024;

/// `track.bounce {track, start?, end?, tail?}`.
pub(crate) fn bounce(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    let track = track_id(&session.project, params)?;
    let p = &session.project;
    let source = p.track(track).ok_or("the track is missing")?;
    let is_return =
        source.channel.and_then(|c| p.channel(c)).is_some_and(|c| c.role == suhina_model::ChannelRole::Effect);
    if is_return || TrackKind::of(source).is_none() || source.channel.is_none() {
        return Err("only an audio or a MIDI track can be bounced".into());
    }
    let name = p.shown_track_name(track);
    let (start, end) = range(p, track, &name, params)?;
    let tail = match Args::new(params).value("tail").filter(|v| !v.is_null()) {
        None => bar_beats(p),
        Some(_) => {
            let tail = Args::new(params).f64("tail")?;
            if !(0.0..=MAX_TAIL).contains(&tail) {
                return Err(format!("tail must be from 0 to {MAX_TAIL} beats").into());
            }
            tail
        }
    };
    let folder = session
        .source()
        .and_then(Path::parent)
        .map(|f| f.join(BOUNCED_FOLDER))
        .ok_or("save the project first: bounced audio is written to a folder next to the project file")?;
    let rate = session.player().map_or(DEFAULT_RATE, |player| player.sample_rate());

    let (samples, peak, warnings) = render(session, track, start, end + tail, rate)?;
    std::fs::create_dir_all(&folder).map_err(|e| format!("could not make {}: {e}", folder.display()))?;
    let path = free_path(&folder, &format!("{} bounce", sanitized(&name)))?;
    write_wav(&path, rate, &samples)?;
    let key = crate::files::absolute(&path).display().to_string();
    let seconds = (samples.len() / 2) as f64 / f64::from(rate);
    session.add_audio(key.clone(), suhina_engine::AudioData { sample_rate: rate, channels: 2, samples });
    let region = AudioRegion {
        file: key.clone(),
        absolute_file: Some(key.clone()),
        sample_rate: Some(rate),
        channels: Some(2),
        duration_seconds: Some(seconds),
        stream: None,
    };
    let (new_track, clip) = place(&mut session.project, track, &name, region, start, end + tail)?;
    Ok(json!({
        "track": new_track.0,
        "clip": clip,
        "file": key,
        "source": track.0,
        "start": start,
        "end": end + tail,
        "peak": peak,
        "warnings": warnings,
    }))
}

/// The range in beats: the parameters', or from the bar the track's first enabled arrangement
/// clip starts in to the end of its last one.
fn range(p: &Project, track: TrackId, name: &str, params: &Value) -> Result<(f64, f64), CommandError> {
    let clips: Vec<&Clip> = p
        .arrangement
        .lanes
        .iter()
        .filter(|l| l.track == Some(track))
        .flat_map(|l| &l.clips)
        .filter(|c| c.enabled && c.start.is_finite() && c.length.is_finite() && c.length > 0.0)
        .collect();
    if clips.is_empty() {
        return Err(format!("track {name:?} has no clips in the arrangement to bounce").into());
    }
    let bar = bar_beats(p);
    let first = clips.iter().map(|c| c.start).fold(f64::INFINITY, f64::min);
    let last = clips.iter().map(|c| c.start + c.length).fold(0.0, f64::max);
    let start = match Args::new(params).value("start").filter(|v| !v.is_null()) {
        None => (first / bar).floor() * bar,
        Some(_) => Args::new(params).f64("start")?,
    };
    let end = match Args::new(params).value("end").filter(|v| !v.is_null()) {
        None => last,
        Some(_) => Args::new(params).f64("end")?,
    };
    if start < 0.0 || end > suhina_engine::MAX_START_BEAT {
        return Err(format!("start and end must be from 0 to {}", suhina_engine::MAX_START_BEAT).into());
    }
    if end - start < 1.0 {
        return Err("end must be at least one beat after start".into());
    }
    Ok((start, end))
}

/// Renders `track` pre-fader from `start` to `end` beats at `rate`: the node render with the
/// track's volume, pan and mute (and their lanes) left out and no solo anywhere. Returns the
/// interleaved stereo samples, their peak and the warnings.
fn render(
    session: &mut Session,
    track: TrackId,
    start: f64,
    end: f64,
    rate: u32,
) -> Result<(Vec<f32>, f32, Vec<String>), CommandError> {
    let data = session.export_data();
    let mut project = data.project.clone();
    let own = project.track(track).and_then(|t| t.channel);
    for channel in &mut project.channels {
        channel.solo = false;
        if Some(channel.id) == own {
            channel.volume = 1.0;
            channel.pan = 0.0;
            channel.mute = false;
            channel.automation.retain(|lane| !MIXER_PARAMS.contains(&lane.param.as_str()));
        }
    }
    let first = suhina_engine::frame_of_beat(&project, start, rate) as usize;
    let last = suhina_engine::frame_of_beat(&project, end, rate) as usize;
    let seconds = last as f64 / f64::from(rate);
    if seconds > suhina_engine::MAX_RENDER_SECONDS {
        return Err(format!(
            "the bounce would be {seconds:.0} s long from the start; the limit is {:.0} s",
            suhina_engine::MAX_RENDER_SECONDS
        )
        .into());
    }
    if last.saturating_mul(2).saturating_mul(size_of::<f32>()) > MAX_RENDER_BYTES {
        return Err("the bounce would be more than 2.0 GiB of samples".into());
    }
    // `started` stays whole while the renderer runs: dropping it ends the plug-in hosts.
    let started = match data.plugins.as_deref() {
        Some(env) => crate::rack::start_offline(&project, &data.audio, env, rate),
        None => crate::rack::OfflinePlugins::none(),
    };
    let notices = started.notices.clone();
    let mut renderer = Renderer::build(
        &project,
        RenderInputs::from_audio(&data.audio, crate::devices()).with_assets(&data.assets).with_plugins(started.plugins),
        rate,
        RenderOptions { source: Some(track), ..RenderOptions::PLAYING },
    )
    .map_err(|e| format!("could not render the track: {e}"))?;
    renderer.set_offline(true);
    renderer.set_follow_loop(false);
    // The render comes out as late as the project's latency compensation makes every track;
    // the file starts on its beat, since the new track is compensated again when it plays.
    let latency = renderer.latency() as usize;
    let (first, last) = (first.saturating_add(latency), last.saturating_add(latency));
    let mut out = vec![0.0f32; last.saturating_mul(2)];
    for block in out.chunks_mut(BLOCK_FRAMES * 2) {
        renderer.process(block);
    }
    out.drain(..first.saturating_mul(2).min(out.len()));
    if out.is_empty() {
        return Err("the range holds no audio between its beats".into());
    }
    let peak = out.iter().fold(0.0f32, |m, s| m.max(s.abs()));
    let missing = crate::relink::missing_of(&data.project, data.folder.as_deref(), &data.audio, &data.trusted)
        .into_iter()
        .map(|missing| format!("{} is missing", missing.file));
    let warnings = data.warnings.into_iter().chain(notices).chain(missing).collect();
    Ok((out, peak, warnings))
}

/// Adds the audio track that plays `region` from `start` to `end`, right after `source` and
/// with its colour and mixer settings, and mutes `source`. Returns the new track and clip ids.
fn place(
    p: &mut Project,
    source: TrackId,
    name: &str,
    region: AudioRegion,
    start: f64,
    end: f64,
) -> Result<(TrackId, u32), CommandError> {
    let own = p.track(source).and_then(|t| t.channel).ok_or("the track has no channel")?;
    let mixer = p.channel(own).cloned().ok_or("the track's channel is missing")?;
    let color = p.track(source).and_then(|t| t.color);
    let new_track = insert_track(p, TrackKind::Audio, &format!("{name} bounce"), Some(source), None)?;
    let channel = p.track(new_track).and_then(|t| t.channel).ok_or("the new track has no channel")?;
    if let Some(t) = p.tracks.iter_mut().find(|t| t.id == new_track) {
        t.color = color;
    }
    if let Some(c) = p.channels.iter_mut().find(|c| c.id == channel) {
        c.volume = mixer.volume;
        c.pan = mixer.pan;
        c.output = mixer.output;
        c.sends = mixer.sends.clone();
        c.automation =
            mixer.automation.iter().filter(|lane| MIXER_PARAMS.contains(&lane.param.as_str())).cloned().collect();
    }
    // The bounced track is muted, so the part is not heard twice; a mute lane would undo it.
    if let Some(c) = p.channels.iter_mut().find(|c| c.id == own) {
        c.mute = true;
        for lane in c.automation.iter_mut().filter(|lane| lane.param == suhina_model::automation::MUTE) {
            lane.enabled = false;
        }
    }

    let content =
        ContentId(p.contents.iter().map(|c| c.id.0).max().unwrap_or(0).checked_add(1).ok_or("too many clips")?);
    p.contents.push(ClipContent {
        id: content,
        name: name.to_string(),
        length: end - start,
        lanes: vec![ContentLane { target: None, data: LaneData::Audio(region) }].into(),
        extensions: Default::default(),
        scale: None,
        linked: false,
    });
    let mut clip = Clip::new(content, start, end - start);
    clip.id = p.new_clip_id()?;
    let id = clip.id.0;
    let lane = p.arrangement.lanes.iter_mut().find(|l| l.track == Some(new_track)).ok_or("the new lane is missing")?;
    lane.clips.push(clip);
    Ok((new_track, id))
}

fn bar_beats(p: &Project) -> f64 {
    let ts = p.transport.time_signature;
    if ts.denominator == 0 { 4.0 } else { f64::from(ts.numerator.max(1)) * 4.0 / f64::from(ts.denominator) }
}
