//! egui UI: top bar, Session View grid (design D8) and the piano roll for the selected clip.

use dsp::DRUM_NOTES;
use dsp::state::{BEATS_PER_BAR, Note};
use eframe::egui::{self, Color32, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2, vec2};
use wasm_bindgen_futures::spawn_local;

use crate::model::{ClipView, Kind, Launch, ProjectView, SCENES, TrackView};
use crate::web::{Ctx, log, start_audio};

const COL_W: f32 = 120.0;
const SLOT_H: f32 = 26.0;
const HEADER_H: f32 = 22.0;
const SCENE_W: f32 = 44.0;
const PLAY_W: f32 = 22.0; // ▶ hit area at the left of a clip slot

const STEP_W: f32 = 18.0; // one 1/16 note in the piano roll
const LABEL_W: f32 = 44.0;
const VEL_H: f32 = 48.0;
const SYNTH_ROW_H: f32 = 12.0;
const DRUM_ROW_H: f32 = 22.0;
const SYNTH_PITCHES: std::ops::RangeInclusive<u8> = 36..=84;
const DRUM_NAMES: [&str; 4] = ["Kick", "Snare", "Hat", "Bass"];
const RESIZE_EDGE: f32 = 6.0;

#[derive(Clone, Copy)]
enum DragMode {
    Move,
    Resize,
    Velocity,
}

struct Drag {
    note_id: String,
    mode: DragMode,
    orig: Note,
    start: Pos2,
    last: Note,
}

enum ClipState {
    Stopped,
    Queued,
    Playing(f64), // loop progress 0..1
}

pub struct App {
    c: Ctx,
    room: String,
    selected: Option<String>, // clip id
    drag: Option<Drag>,
    new_note_len: f64,
}

fn note_name(p: u8) -> String {
    const NAMES: [&str; 12] = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];
    format!("{}{}", NAMES[p as usize % 12], p as i32 / 12 - 1)
}

fn clip_state(launch: &Launch, clip: &str, len: f64, beat: Option<f64>) -> ClipState {
    let Some(b) = beat else { return ClipState::Stopped };
    let progress = |from: f64| ((b - from).rem_euclid(len)) / len;
    if launch.clip.as_deref() == Some(clip) {
        return if b >= launch.at { ClipState::Playing(progress(launch.at)) } else { ClipState::Queued };
    }
    if launch.prev.as_deref() == Some(clip) && b < launch.at && b >= launch.prev_at {
        return ClipState::Playing(progress(launch.prev_at));
    }
    ClipState::Stopped
}

impl App {
    pub(crate) fn new(c: Ctx, room: String) -> Self {
        Self { c, room, selected: None, drag: None, new_note_len: 0.25 }
    }

    fn top_bar(&mut self, ui: &mut egui::Ui, view: &ProjectView, beat: Option<f64>) {
        let c = &self.c;
        ui.horizontal(|ui| {
            ui.heading("P2P DAW");
            ui.separator();
            if c.audio.borrow().is_none() && ui.button("🔈 Start audio").clicked() {
                    let c = c.clone();
                    spawn_local(async move {
                        if let Err(e) = start_audio(c.clone()).await {
                            log(&format!("audio failed: {e:?}"));
                            c.status("audio failed to start (see console)");
                        }
                    });
            }
            if view.transport.playing {
                if ui.button("■ Stop").clicked() {
                    c.project.stop();
                }
            } else if ui.button("▶ Play").clicked() {
                c.project.play(c.shared_now());
            }
            let mut bpm = view.transport.bpm;
            if ui.add(egui::DragValue::new(&mut bpm).range(20.0..=400.0).speed(0.5).suffix(" BPM")).changed() {
                c.project.set_bpm(bpm.round(), c.shared_now());
            }
            ui.label(match beat {
                Some(b) if b >= 0.0 => format!("{}.{}", (b / BEATS_PER_BAR).floor() + 1.0, (b % BEATS_PER_BAR).floor() + 1.0),
                Some(_) => "count-in".into(),
                None => "-.-".into(),
            });
            let mut nudge = c.nudge_ms.get();
            let r = ui.add(egui::DragValue::new(&mut nudge).range(-200.0..=200.0).suffix(" ms"));
            if r.on_hover_text("Latency nudge: shift your audio later (+) or earlier (−). Not shared.").changed() {
                c.nudge_ms.set(nudge);
                c.post_anchor();
            }
            ui.separator();
            ui.label(format!("room {} · {}", self.room, c.status.borrow()));
        });
    }

    fn session(&mut self, ui: &mut egui::Ui, view: &ProjectView, beat: Option<f64>, blink: bool) {
        let now = self.c.shared_now();
        egui::ScrollArea::both().id_salt("session").show(ui, |ui| {
            ui.horizontal_top(|ui| {
                ui.vertical(|ui| {
                    ui.allocate_space(vec2(SCENE_W, HEADER_H));
                    for s in 0..SCENES {
                        let b = ui.add_sized([SCENE_W, SLOT_H], egui::Button::new(format!("▶ {}", s + 1)));
                        if b.on_hover_text("Launch scene").clicked() {
                            self.c.project.launch_scene(s, now);
                        }
                    }
                });
                for t in &view.tracks {
                    ui.vertical(|ui| {
                        ui.set_width(COL_W);
                        self.track_column(ui, view, t, beat, blink, now);
                    });
                }
                ui.vertical(|ui| {
                    if ui.button("+ Synth").clicked() {
                        self.c.project.add_track(Kind::Synth);
                    }
                    if ui.button("+ Drums").clicked() {
                        self.c.project.add_track(Kind::Drums);
                    }
                });
            });
        });
    }

    fn track_column(&mut self, ui: &mut egui::Ui, view: &ProjectView, t: &TrackView, beat: Option<f64>, blink: bool, now: f64) {
        let p = &self.c.project;
        ui.horizontal(|ui| {
            ui.set_height(HEADER_H);
            ui.label(egui::RichText::new(&t.name).strong());
            if ui.small_button("✕").on_hover_text("Delete track").clicked() {
                p.delete_track(&t.id);
            }
        });
        for s in 0..SCENES {
            let (rect, resp) = ui.allocate_exact_size(vec2(COL_W, SLOT_H), Sense::click());
            let painter = ui.painter_at(rect);
            match &t.slots[s] {
                None => {
                    painter.rect_filled(rect.shrink(1.0), 3.0, Color32::from_gray(38));
                    if resp.double_clicked() {
                        self.selected = p.create_clip(&t.id, s);
                    }
                }
                Some(cid) => {
                    let len = view.clips.get(cid).map_or(BEATS_PER_BAR, |c| c.length_beats);
                    let state = clip_state(&t.launch, cid, len, beat);
                    let fill = match state {
                        ClipState::Playing(_) => Color32::from_rgb(60, 150, 90),
                        ClipState::Queued if blink => Color32::from_rgb(90, 190, 120),
                        _ => Color32::from_rgb(70, 90, 125),
                    };
                    painter.rect_filled(rect.shrink(1.0), 3.0, fill);
                    if let ClipState::Playing(prog) = state {
                        let bar = Rect::from_min_size(rect.left_bottom() - vec2(0.0, 3.0), vec2(rect.width() * prog as f32, 3.0));
                        painter.rect_filled(bar, 0.0, Color32::WHITE);
                    }
                    let font = egui::FontId::proportional(13.0);
                    painter.text(rect.left_center() + vec2(6.0, 0.0), egui::Align2::LEFT_CENTER, "▶", font.clone(), Color32::WHITE);
                    let bars = len / BEATS_PER_BAR;
                    let label = format!("{} bar{}", bars, if bars == 1.0 { "" } else { "s" });
                    painter.text(rect.left_center() + vec2(PLAY_W + 4.0, 0.0), egui::Align2::LEFT_CENTER, label, font, Color32::WHITE);
                    if self.selected.as_deref() == Some(cid.as_str()) {
                        painter.rect_stroke(rect.shrink(1.0), 3.0, Stroke::new(2.0, Color32::WHITE), StrokeKind::Inside);
                    }
                    if resp.clicked() {
                        let on_play = resp.interact_pointer_pos().is_some_and(|pos| pos.x < rect.left() + PLAY_W);
                        if on_play {
                            p.launch(&t.id, Some(cid), now);
                        }
                        self.selected = Some(cid.clone());
                    }
                }
            }
        }
        if ui.add_sized([COL_W, 20.0], egui::Button::new("■")).on_hover_text("Stop track at the next bar").clicked() {
            p.launch(&t.id, None, now);
        }
        ui.horizontal(|ui| {
            ui.spacing_mut().slider_width = COL_W - 40.0;
            let mut vol = t.volume;
            if ui.add(egui::Slider::new(&mut vol, 0.0..=1.0).show_value(false)).changed() {
                p.set_volume(&t.id, vol);
            }
            if ui.selectable_label(t.mute, "M").on_hover_text("Mute").clicked() {
                p.set_mute(&t.id, !t.mute);
            }
        });
    }

    fn detail(&mut self, ui: &mut egui::Ui, view: &ProjectView, beat: Option<f64>) {
        let found = self.selected.as_ref().and_then(|cid| {
            let track = view.tracks.iter().find(|t| t.slots.iter().flatten().any(|s| s == cid))?;
            Some((track, cid.clone(), view.clips.get(cid)?))
        });
        let Some((track, cid, clip)) = found else {
            ui.label("Double-click an empty slot to create a clip; click a clip to edit it here.");
            return;
        };
        let p = &self.c.project;
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(&track.name).strong());
            ui.separator();
            ui.label("Length");
            for bars in [1.0, 2.0, 4.0, 8.0] {
                if ui.selectable_label(clip.length_beats == bars * BEATS_PER_BAR, format!("{bars} bar")).clicked() {
                    p.set_clip_length(&cid, bars * BEATS_PER_BAR);
                }
            }
        });
        if track.kind == Kind::Synth {
            self.synth_params(ui, track);
        }
        let playing_at = beat.and_then(|b| track.launch.effective(b).filter(|(c, _)| *c == cid).map(|(_, at)| (b - at).rem_euclid(clip.length_beats)));
        egui::ScrollArea::both().id_salt("roll").show(ui, |ui| self.piano_roll(ui, track, &cid, clip, playing_at));
    }

    fn synth_params(&self, ui: &mut egui::Ui, track: &TrackView) {
        let (p, id, sp) = (&self.c.project, &track.id, track.params);
        ui.horizontal_wrapped(|ui| {
            if ui.selectable_label(!sp.square, "Saw").clicked() {
                p.set_wave(id, false);
            }
            if ui.selectable_label(sp.square, "Square").clicked() {
                p.set_wave(id, true);
            }
            let mut knob = |label: &str, key: &str, value: f32, range: std::ops::RangeInclusive<f32>, log: bool| {
                let mut v = value;
                if ui.add(egui::Slider::new(&mut v, range).logarithmic(log).text(label)).changed() {
                    p.set_param(id, key, v as f64);
                }
            };
            knob("Cutoff", "cutoff", sp.cutoff, 40.0..=12_000.0, true);
            knob("Res", "resonance", sp.resonance, 0.0..=0.95, false);
            knob("A", "attack", sp.attack, 0.001..=2.0, true);
            knob("D", "decay", sp.decay, 0.01..=2.0, true);
            knob("S", "sustain", sp.sustain, 0.0..=1.0, false);
            knob("R", "release", sp.release, 0.01..=4.0, true);
        });
    }

    fn piano_roll(&mut self, ui: &mut egui::Ui, track: &TrackView, cid: &str, clip: &ClipView, playhead: Option<f64>) {
        let rows: Vec<(u8, String)> = match track.kind {
            Kind::Drums => DRUM_NOTES.iter().zip(DRUM_NAMES).map(|(&p, n)| (p, n.to_string())).collect(),
            Kind::Synth => SYNTH_PITCHES.rev().map(|p| (p, note_name(p))).collect(),
        };
        let row_h = if track.kind == Kind::Drums { DRUM_ROW_H } else { SYNTH_ROW_H };
        let steps = (clip.length_beats * 4.0).round() as usize;
        let size = vec2(LABEL_W + steps as f32 * STEP_W, rows.len() as f32 * row_h + 6.0 + VEL_H);
        let (rect, resp) = ui.allocate_exact_size(size, Sense::click_and_drag());
        let grid = Rect::from_min_size(rect.min + vec2(LABEL_W, 0.0), vec2(steps as f32 * STEP_W, rows.len() as f32 * row_h));
        let vel = Rect::from_min_size(Pos2::new(grid.left(), grid.bottom() + 6.0), vec2(grid.width(), VEL_H));
        let painter = ui.painter_at(rect);

        let row_of = |pitch: u8| rows.iter().position(|(p, _)| *p == pitch);
        let note_rect = |n: &Note| {
            row_of(n.pitch).map(|r| {
                Rect::from_min_size(
                    Pos2::new(grid.left() + n.start as f32 * 4.0 * STEP_W, grid.top() + r as f32 * row_h),
                    vec2((n.len as f32 * 4.0 * STEP_W).max(3.0), row_h),
                )
            })
        };

        // background: rows, labels, step lines
        for (i, (pitch, name)) in rows.iter().enumerate() {
            let r = Rect::from_min_size(Pos2::new(grid.left(), grid.top() + i as f32 * row_h), vec2(grid.width(), row_h));
            let black = track.kind == Kind::Synth && matches!(pitch % 12, 1 | 3 | 6 | 8 | 10);
            painter.rect_filled(r, 0.0, if black { Color32::from_gray(30) } else { Color32::from_gray(40) });
            if track.kind == Kind::Drums || pitch % 12 == 0 {
                painter.text(Pos2::new(rect.left() + 4.0, r.center().y), egui::Align2::LEFT_CENTER, name, egui::FontId::monospace(10.0), Color32::GRAY);
            }
        }
        for s in 0..=steps {
            let x = grid.left() + s as f32 * STEP_W;
            let shade = if s % 16 == 0 { 110 } else if s % 4 == 0 { 70 } else { 50 };
            painter.line_segment([Pos2::new(x, grid.top()), Pos2::new(x, vel.bottom())], Stroke::new(1.0, Color32::from_gray(shade)));
        }
        painter.rect_filled(vel, 0.0, Color32::from_gray(25));

        for (_, n) in &clip.notes {
            let Some(r) = note_rect(n) else { continue };
            let alpha = (80.0 + 175.0 * n.vel) as u8;
            painter.rect_filled(r.shrink(1.0), 2.0, Color32::from_rgba_unmultiplied(255, 140, 40, alpha));
            let h = VEL_H * n.vel;
            let bar = Rect::from_min_size(Pos2::new(r.left() + 1.0, vel.bottom() - h), vec2(4.0, h));
            painter.rect_filled(bar, 0.0, Color32::from_rgb(255, 140, 40));
        }
        if let Some(b) = playhead {
            let x = grid.left() + b as f32 * 4.0 * STEP_W;
            painter.line_segment([Pos2::new(x, grid.top()), Pos2::new(x, vel.bottom())], Stroke::new(2.0, Color32::WHITE));
        }

        // interactions
        let p = &self.c.project;
        let hit = |pos: Pos2| clip.notes.iter().find(|(_, n)| note_rect(n).is_some_and(|r| r.contains(pos)));
        let under_x = |x: f32| clip.notes.iter().find(|(_, n)| note_rect(n).is_some_and(|r| x >= r.left() && x <= r.right()));
        let pos = resp.interact_pointer_pos();

        // hit-test where the button went down, not where egui recognised the drag a few px later
        if resp.drag_started()
            && let Some(pos) = ui.input(|i| i.pointer.press_origin()).or(pos)
        {
            self.drag = if vel.contains(pos) {
                under_x(pos.x).map(|(id, n)| Drag { note_id: id.clone(), mode: DragMode::Velocity, orig: *n, start: pos, last: *n })
            } else {
                hit(pos).map(|(id, n)| {
                    let edge = note_rect(n).is_some_and(|r| pos.x > r.right() - RESIZE_EDGE);
                    Drag { note_id: id.clone(), mode: if edge { DragMode::Resize } else { DragMode::Move }, orig: *n, start: pos, last: *n }
                })
            };
        }
        if resp.dragged()
            && let (Some(d), Some(pos)) = (self.drag.as_mut(), pos)
        {
            let delta: Vec2 = pos - d.start;
            let dsteps = (delta.x / STEP_W).round() as f64 * 0.25;
            let mut n = d.orig;
            match d.mode {
                DragMode::Move => {
                    n.start = (d.orig.start + dsteps).clamp(0.0, clip.length_beats - 0.25);
                    if let Some(r) = row_of(d.orig.pitch) {
                        let r = (r as i32 + (delta.y / row_h).round() as i32).clamp(0, rows.len() as i32 - 1);
                        n.pitch = rows[r as usize].0;
                    }
                }
                DragMode::Resize => n.len = (d.orig.len + dsteps).max(0.25),
                DragMode::Velocity => n.vel = (1.0 - (pos.y - vel.top()) / VEL_H).clamp(0.0, 1.0),
            }
            if n != d.last {
                p.set_note(cid, &d.note_id, n);
                d.last = n;
            }
        }
        if resp.drag_stopped()
            && let Some(d) = self.drag.take()
            && matches!(d.mode, DragMode::Resize)
        {
            self.new_note_len = d.last.len;
        }
        if resp.clicked()
            && let Some(pos) = pos
            && grid.contains(pos)
            && hit(pos).is_none()
        {
            let step = ((pos.x - grid.left()) / STEP_W).floor() as f64;
            let row = ((pos.y - grid.top()) / row_h).floor() as usize;
            if let Some((pitch, _)) = rows.get(row) {
                p.add_note(cid, Note { pitch: *pitch, start: step * 0.25, len: self.new_note_len, vel: 0.8 });
            }
        }
        if resp.secondary_clicked()
            && let Some((id, _)) = pos.and_then(hit)
        {
            p.delete_note(cid, id);
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.c.flush();
        let view = self.c.view.borrow().clone();
        let beat = view.beat(self.c.shared_now());
        let blink = ((crate::web::local_now() / 250.0) as u64).is_multiple_of(2);

        egui::Panel::top("top").show(ui, |ui| self.top_bar(ui, &view, beat));
        egui::Panel::bottom("detail").resizable(true).min_size(300.0).show(ui, |ui| self.detail(ui, &view, beat));
        egui::CentralPanel::default().show(ui, |ui| self.session(ui, &view, beat, blink));

        // edits made this frame reach the engine now, and get drawn on an immediate next frame
        if self.c.flush() || view.transport.playing {
            ui.ctx().request_repaint(); // also keeps playhead, progress bars and blinking moving
        }
    }
}
