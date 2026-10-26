/// Boot intro animation, shown on startup unless disabled in Settings → Appearance.
/// The styles share the loading bar, stage text and skip handling.

pub mod modern;
pub mod retro;

use std::cell::RefCell;
use std::io;
use std::time::{Duration, Instant};

use crossterm::event::{self, Event, KeyEventKind};
use ratatui::{
    backend::Backend,
    layout::Rect,
    style::{Color, Style},
    text::Span,
    Frame, Terminal,
};

/// Partial-block characters for sub-cell progress bar resolution.
const PARTIALS: [char; 7] = ['▏', '▎', '▍', '▌', '▋', '▊', '▉'];

const STAGES: &[&str] = &[
    "INITIALISING KERNEL",
    "MOUNTING CONFIGURATION",
    "LOADING DASHBOARDS",
    "REGISTERING APPLICATIONS",
    "STARTING SHELL",
];

const DURATION_MS: u64 = 2600;
const FRAME_MS: u64 = 33;
const HOLD_MS: u64 = 600;

/// Boot animation choices from Settings → Appearance.
#[derive(Clone, Copy)]
pub struct Options<'a> {
    pub style: &'a str,
    pub colour: &'a str,
    pub logo: &'a str,
    pub reveal: &'a str,
    pub duration: &'a str,
    pub effect: bool,
}

impl Options<'_> {
    fn is_retro(&self) -> bool {
        self.style.eq_ignore_ascii_case("Retro")
    }

    fn look(&self) -> modern::Look {
        modern::Look {
            tint: modern::Tint::from_name(self.colour),
            logo: modern::Logo::from_name(self.logo),
            reveal: modern::Reveal::from_name(self.reveal),
            effect: self.effect,
            duration_ms: duration_ms(self.duration),
        }
    }
}

/// How long the intro runs before the hold, in milliseconds.
pub fn duration_ms(name: &str) -> u64 {
    match name.to_ascii_lowercase().as_str() {
        "short" => 1400,
        "long" => 4600,
        _ => DURATION_MS,
    }
}

/// Play the intro animation in the configured style. Returns early if the user
/// presses a key.
pub fn play<B: Backend>(
    terminal: &mut Terminal<B>,
    accent: Color,
    options: Options<'_>,
) -> io::Result<()> {
    let duration = duration_ms(options.duration);
    if options.is_retro() {
        retro::play(terminal, accent, duration)
    } else {
        modern::play(terminal, accent, options.look(), duration)
    }
}

/// What the preview was last built for; a change replays it from the start.
#[derive(PartialEq)]
struct PreviewKey {
    retro: bool,
    look: modern::Look,
    duration: u64,
    area: (u16, u16),
}

struct PreviewState {
    key: PreviewKey,
    started: Instant,
    splash: Option<modern::Splash>,
}

thread_local! {
    static PREVIEW: RefCell<Option<PreviewState>> = const { RefCell::new(None) };
}

/// Draw the intro into `area` for the Settings preview. It runs through once
/// and then holds on the finished frame until the options change.
pub fn draw_preview(frame: &mut Frame, area: Rect, accent: Color, options: Options<'_>) {
    if area.width < 8 || area.height < 4 {
        return;
    }
    let key = PreviewKey {
        retro: options.is_retro(),
        look: options.look(),
        duration: duration_ms(options.duration),
        area: (area.width, area.height),
    };

    PREVIEW.with(|cell| {
        let mut slot = cell.borrow_mut();
        if !matches!(&*slot, Some(state) if state.key == key) {
            let splash = if key.retro {
                None
            } else {
                modern::build_splash(area.width, area.height, key.look)
            };
            *slot = Some(PreviewState { key, started: Instant::now(), splash });
        }

        let Some(state) = slot.as_mut() else { return };
        let duration = state.key.duration;
        let tick = (state.started.elapsed().as_millis() as u64).min(duration + HOLD_MS);
        let ready = tick >= duration;
        let progress = if ready { 1.0 } else { eased(tick as f64 / duration as f64) };

        if state.key.retro {
            retro::render(frame, area, progress, tick, accent, ready);
        } else {
            let look = state.key.look;
            if let Some(splash) = state.splash.as_mut() {
                modern::update(splash, look, tick);
            }
            let accent = modern::frame_accent(accent, look.tint, tick);
            modern::render(frame, area, state.splash.as_mut(), progress, tick, accent, ready);
        }
    });
}

fn skip_requested() -> io::Result<bool> {
    if event::poll(Duration::from_millis(FRAME_MS))? {
        if let Event::Key(key) = event::read()? {
            return Ok(key.kind == KeyEventKind::Press);
        }
    }
    Ok(false)
}

/// Nudge the linear ratio so the bar stutters between stages like a real boot.
fn eased(t: f64) -> f64 {
    let t = t.clamp(0.0, 1.0);
    let wobble = (t * STAGES.len() as f64 * std::f64::consts::PI).sin() * 0.035;
    (t + wobble).clamp(0.0, 1.0)
}

fn center(span: Span<'_>, width: u16) -> Vec<Span<'_>> {
    let pad = (width as usize).saturating_sub(span.content.chars().count()) / 2;
    vec![Span::raw(" ".repeat(pad)), span]
}

fn bar_spans(width: usize, ratio: f64, accent: Color, trough: Color) -> Vec<Span<'static>> {
    let eighths = (width as f64 * 8.0 * ratio).round() as usize;
    let full = eighths / 8;
    let remainder = eighths % 8;

    let mut filled = "█".repeat(full.min(width));
    let mut used = full.min(width);
    if remainder > 0 && used < width {
        filled.push(PARTIALS[remainder - 1]);
        used += 1;
    }

    let mut spans = vec![Span::styled(filled, Style::default().fg(accent))];
    if used < width {
        spans.push(Span::styled(
            "░".repeat(width - used),
            Style::default().fg(trough),
        ));
    }
    spans
}

