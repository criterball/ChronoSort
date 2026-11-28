//! Test driver, for click-testing without touching the real mouse and keyboard of the PC.
//!
//! Only active when ChronoSort is started with `CHRONOSORT_DRIVE=<file>`. Commands written to that file are
//! read, the file is deleted, and the commands are fed into ChronoSort's own window as input events — one
//! step per frame, like a person would. It works while the window is in the background, so the user can keep
//! working. Coordinates are window points (inside the frame). Commands, one per line:
//!   click X Y · dblclick X Y · rclick X Y · move X Y · drag X1 Y1 X2 Y2 · key NAME · ctrl NAME · shift NAME ·
//!   alt NAME · type TEXT · wheel DY · ctrlwheel DY · wait MS · state
//! `state` writes a short summary of what the app shows to `<file>.out` (for checks in scripts).

use egui::{Event, Key, Modifiers, PointerButton, Pos2, RawInput, pos2};
use std::collections::VecDeque;
use std::path::PathBuf;
use std::time::{Duration, Instant};

enum Step {
    Move(Pos2),
    Button(PointerButton, bool),
    Key(Key, Modifiers, bool),
    Text(String),
    Wheel(f32, Modifiers),
    Wait(Duration),
    State,
}

pub struct Driver {
    file: PathBuf,
    queue: VecDeque<Step>,
    wait_until: Instant,
    pos: Pos2,
    /// Set for one frame when a `state` step ran: the app writes its summary.
    pub want_state: bool,
}

impl Driver {
    pub fn from_env() -> Option<Self> {
        let file = PathBuf::from(std::env::var_os("CHRONOSORT_DRIVE")?);
        Some(Self { file, queue: VecDeque::new(), wait_until: Instant::now(), pos: pos2(0.0, 0.0), want_state: false })
    }

    pub fn out_file(&self) -> PathBuf {
        let mut s = self.file.clone().into_os_string();
        s.push(".out");
        PathBuf::from(s)
    }

    fn load(&mut self) {
        let Ok(text) = std::fs::read_to_string(&self.file) else { return };
        let _ = std::fs::remove_file(&self.file);
        for line in text.lines() {
            let w: Vec<&str> = line.split_whitespace().collect();
            let n = |i: usize| w.get(i).and_then(|s| s.parse::<f32>().ok()).unwrap_or(0.0);
            let key_with = |m: Modifiers, q: &mut VecDeque<Step>| {
                if let Some(k) = w.get(1).and_then(|s| Key::from_name(s)) {
                    q.push_back(Step::Key(k, m, true));
                    q.push_back(Step::Key(k, m, false));
                }
            };
            let q = &mut self.queue;
            match w.first().copied() {
                Some("move") => q.push_back(Step::Move(pos2(n(1), n(2)))),
                Some(c @ ("click" | "dblclick" | "rclick")) => {
                    let b = if c == "rclick" { PointerButton::Secondary } else { PointerButton::Primary };
                    q.push_back(Step::Move(pos2(n(1), n(2))));
                    for _ in 0..if c == "dblclick" { 2 } else { 1 } {
                        q.push_back(Step::Button(b, true));
                        q.push_back(Step::Button(b, false));
                    }
                }
                Some("drag") => {
                    let (a, b) = (pos2(n(1), n(2)), pos2(n(3), n(4)));
                    q.push_back(Step::Move(a));
                    q.push_back(Step::Button(PointerButton::Primary, true));
                    for i in 1..=12 {
                        q.push_back(Step::Move(a + (b - a) * (i as f32 / 12.0)));
                    }
                    q.push_back(Step::Button(PointerButton::Primary, false));
                }
                Some("key") => key_with(Modifiers::NONE, q),
                Some("ctrl") => key_with(Modifiers::COMMAND, q),
                Some("shift") => key_with(Modifiers::SHIFT, q),
                Some("alt") => key_with(Modifiers::ALT, q),
                Some("type") => q.push_back(Step::Text(
                    line.trim_start().strip_prefix("type").unwrap_or("").trim_start().to_string(),
                )),
                Some("wheel") => q.push_back(Step::Wheel(n(1), Modifiers::NONE)),
                Some("ctrlwheel") => q.push_back(Step::Wheel(n(1), Modifiers::COMMAND)),
                Some("wait") => q.push_back(Step::Wait(Duration::from_millis(n(1) as u64))),
                Some("state") => q.push_back(Step::State),
                _ => {}
            }
        }
    }

    /// Called before every frame: add the next step's events.
    pub fn feed(&mut self, raw: &mut RawInput, ctx: &egui::Context) {
        self.want_state = false;
        if self.queue.is_empty() {
            self.load();
        }
        // Look for new commands a few times a second even when nothing else happens.
        ctx.request_repaint_after(Duration::from_millis(250));
        if Instant::now() < self.wait_until {
            return;
        }
        let Some(step) = self.queue.pop_front() else { return };
        match step {
            Step::Move(p) => {
                self.pos = p;
                raw.events.push(Event::PointerMoved(p));
            }
            Step::Button(button, pressed) => {
                raw.events.push(Event::PointerButton { pos: self.pos, button, pressed, modifiers: Modifiers::NONE })
            }
            Step::Key(key, modifiers, pressed) => {
                raw.events.push(Event::Key { key, physical_key: Some(key), pressed, repeat: false, modifiers });
                if pressed && modifiers.is_none() {
                    // Plain letters also arrive as text (like a real keyboard).
                    let name = key.name();
                    if name.len() == 1 {
                        raw.events.push(Event::Text(name.to_lowercase()));
                    }
                }
            }
            Step::Text(t) => raw.events.push(Event::Text(t)),
            Step::Wheel(dy, modifiers) => {
                raw.events.push(Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    delta: egui::vec2(0.0, dy),
                    phase: egui::TouchPhase::Move,
                    source: egui::MouseWheelSource::Wheel,
                    modifiers,
                });
            }
            Step::Wait(d) => self.wait_until = Instant::now() + d,
            Step::State => self.want_state = true,
        }
        ctx.request_repaint();
    }
}
