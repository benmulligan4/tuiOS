/// Modern intro — the tuiOS logo rendered as a splash screen above the loading bar.

use std::io::{self, Cursor};
use std::time::{Duration, Instant};

use image::{imageops, imageops::FilterType, DynamicImage, ImageFormat, Rgb, RgbImage};
use ratatui::{
    backend::Backend,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame, Terminal,
};
use ratatui_splash_screen::{SplashConfig, SplashScreen};

use crate::settings::pages::appearance::hue_to_rgb;

use super::{bar_spans, center, eased, skip_requested, HOLD_MS, STAGES};

const LOGO_FADED: &[u8] = include_bytes!("../tuios_logo_faded.png");
const LOGO_STATIC: &[u8] = include_bytes!("../tuios_logo_static.png");

/// Frames the splash takes to resolve from blurred/dark to the full logo.
const SPLASH_STEPS: i32 = 12;

/// Widest the centred text block under the logo is allowed to get.
const CONTENT_MAX_W: u16 = 84;
/// Detail is capped at one cell per pixel, so the logo takes most of the width it can get.
const SPLASH_MAX_W: u16 = 200;
/// Percentage of the screen width the logo is allowed to span.
const SPLASH_WIDTH_PCT: u32 = 80;
/// Below this the logo turns to mush, so the text wordmark is used instead.
const SPLASH_MIN_W: u16 = 40;
const SPLASH_MIN_ROWS: u16 = 3;
/// Cells are roughly twice as tall as they are wide.
const CELL_ASPECT: u32 = 2;
/// Anything brighter than this counts as logo rather than background padding.
const TRIM_LUMA: u16 = 96;
/// Subtitle, spacers, bar and status line rendered under the logo.
const TEXT_ROWS: u16 = 7;

/// Degrees of hue the rainbow tint spans across the logo.
const RAINBOW_SPAN: f32 = 300.0;
/// Degrees of hue the rainbow tint drifts per millisecond.
const RAINBOW_DRIFT: f32 = 0.12;

/// How long a directional wipe takes to cross the logo.
const WIPE_MS: u64 = 900;
/// Fraction of the logo the wipe's soft leading edge covers.
const WIPE_FEATHER: f32 = 0.3;

/// Hue the Nebula wheel opens on — deep blue, running up through purple and pink.
const NEBULA_START: f32 = 240.0;

/// Colour stops blended evenly along the tint axis.
type Stops = &'static [(u8, u8, u8)];

const AURORA: Stops = &[(0, 255, 163), (0, 229, 255), (138, 43, 226)];
const WARM: Stops = &[(255, 61, 0), (255, 145, 0), (255, 214, 0)];
const COOL: Stops = &[(80, 90, 220), (0, 150, 255), (0, 235, 235)];
const NEON: Stops = &[(200, 0, 255), (120, 60, 255), (0, 180, 255)];
const SUNSET: Stops = &[(255, 120, 0), (255, 0, 128), (120, 0, 210)];
const OCEAN: Stops = &[(0, 70, 190), (0, 150, 220), (0, 240, 220)];
const SYNTHWAVE: Stops = &[(255, 0, 170), (150, 0, 255), (0, 220, 255)];
const MATRIX: Stops = &[(0, 120, 30), (0, 255, 70), (190, 255, 190)];
const EMBER: Stops = &[(255, 220, 120), (255, 90, 0), (150, 0, 0)];

/// Direction a tint runs in.
#[derive(Clone, Copy, PartialEq)]
pub enum Axis {
    Across,
    Down,
}

impl Axis {
    fn at(self, x: u32, y: u32, width: f32, height: f32) -> f32 {
        match self {
            Self::Across => x as f32 / width,
            Self::Down => y as f32 / height,
        }
    }
}

/// The colour wash laid over the logo.
#[derive(Clone, Copy, PartialEq)]
pub enum Tint {
    /// Artwork left as-is, bar on the accent colour from Settings.
    None,
    /// Hue wheel from `start` degrees, optionally drifting through the spectrum.
    Rainbow { axis: Axis, drift: bool, start: f32 },
    /// Fixed palette blended along an axis.
    Gradient { stops: Stops, axis: Axis },
}

impl Tint {
    pub fn from_name(name: &str) -> Self {
        match name.to_ascii_lowercase().as_str() {
            "rainbow dynamic" | "rainbow" => {
                Self::Rainbow { axis: Axis::Across, drift: true, start: 0.0 }
            }
            "rainbow static" => Self::Rainbow { axis: Axis::Across, drift: false, start: 0.0 },
            "rainbow vertical" => Self::Rainbow { axis: Axis::Down, drift: false, start: 0.0 },
            "nebula" => Self::Rainbow { axis: Axis::Across, drift: false, start: NEBULA_START },
            "aurora" => Self::Gradient { stops: AURORA, axis: Axis::Across },
            "warm" => Self::Gradient { stops: WARM, axis: Axis::Across },
            "cool" => Self::Gradient { stops: COOL, axis: Axis::Across },
            "neon" => Self::Gradient { stops: NEON, axis: Axis::Across },
            "synthwave" => Self::Gradient { stops: SYNTHWAVE, axis: Axis::Across },
            "sunset" => Self::Gradient { stops: SUNSET, axis: Axis::Down },
            "ocean" => Self::Gradient { stops: OCEAN, axis: Axis::Down },
            "matrix" => Self::Gradient { stops: MATRIX, axis: Axis::Down },
            "ember" => Self::Gradient { stops: EMBER, axis: Axis::Down },
            _ => Self::None,
        }
    }

    fn drifts(self) -> bool {
        matches!(self, Self::Rainbow { drift: true, .. })
    }

    /// Hue offset for this frame; only the drifting rainbow moves.
    fn phase(self, tick: u64) -> f32 {
        if self.drifts() { (tick as f32 * RAINBOW_DRIFT) % 360.0 } else { 0.0 }
    }

    /// Colour for a pixel, before the artwork's own brightness is applied.
    fn colour(self, x: u32, y: u32, width: f32, height: f32, phase: f32) -> (u8, u8, u8) {
        match self {
            Self::None => (255, 255, 255),
            Self::Rainbow { axis, start, .. } => {
                hue_to_rgb((start + phase + axis.at(x, y, width, height) * RAINBOW_SPAN) % 360.0)
            }
            Self::Gradient { stops, axis } => sample(stops, axis.at(x, y, width, height)),
        }
    }
}

/// Blend between the stops either side of `t`.
fn sample(stops: Stops, t: f32) -> (u8, u8, u8) {
    match stops.len() {
        0 => (255, 255, 255),
        1 => stops[0],
        len => {
            let scaled = t.clamp(0.0, 1.0) * (len - 1) as f32;
            let i = (scaled as usize).min(len - 2);
            let f = scaled - i as f32;
            let (a, b) = (stops[i], stops[i + 1]);
            let mix = |from: u8, to: u8| (from as f32 + (to as f32 - from as f32) * f) as u8;
            (mix(a.0, b.0), mix(a.1, b.1), mix(a.2, b.2))
        }
    }
}

/// Which artwork the splash is built from.
#[derive(Clone, Copy, PartialEq)]
pub enum Logo {
    /// Badge that fades out to the left.
    Faded,
    /// Solid badge.
    Static,
}

impl Logo {
    pub fn from_name(name: &str) -> Self {
        if name.eq_ignore_ascii_case("Faded") { Self::Faded } else { Self::Static }
    }

    fn bytes(self) -> &'static [u8] {
        match self {
            Self::Faded => LOGO_FADED,
            Self::Static => LOGO_STATIC,
        }
    }
}

/// Edge the logo appears from.
#[derive(Clone, Copy, PartialEq)]
pub enum Reveal {
    /// The widget's own blur-in, with no direction to it.
    Center,
    Up,
    Down,
    Left,
    Right,
}

impl Reveal {
    pub fn from_name(name: &str) -> Self {
        match name.to_ascii_lowercase().as_str() {
            "up" => Self::Up,
            "down" => Self::Down,
            "left" => Self::Left,
            "right" => Self::Right,
            _ => Self::Center,
        }
    }

    /// How far along the wipe a pixel sits, 0 at the edge it starts from.
    fn distance(self, x: u32, y: u32, width: f32, height: f32) -> f32 {
        match self {
            Self::Up => y as f32 / height,
            Self::Down => 1.0 - y as f32 / height,
            Self::Left => x as f32 / width,
            Self::Right => 1.0 - x as f32 / width,
            Self::Center => 0.0,
        }
    }
}

/// Resolved look of the modern intro.
#[derive(Clone, Copy, PartialEq)]
pub struct Look {
    pub tint: Tint,
    pub logo: Logo,
    pub reveal: Reveal,
    /// When false the logo appears at once, with no reveal step.
    pub effect: bool,
    /// Total run time, which the reveal is scaled against.
    pub duration_ms: u64,
}

impl Look {
    fn wipes(self) -> bool {
        self.effect && self.reveal != Reveal::Center
    }

    /// The reveal keeps its share of the run, so a short boot wipes in quickly.
    fn wipe_ms(self) -> u64 {
        (WIPE_MS * self.duration_ms / super::DURATION_MS).max(200)
    }

    fn steps(self) -> i32 {
        ((SPLASH_STEPS as u64 * self.duration_ms / super::DURATION_MS) as i32).clamp(3, 40)
    }
}

/// The logo plus the cell box it was built for.
pub struct Splash {
    screen: SplashScreen,
    /// One pixel per terminal cell, kept for re-tinting.
    cells: RgbImage,
    cells_w: u16,
    cells_h: u16,
}

/// Drop the black padding so the artwork itself fills the space it is given.
fn trim_padding(logo: DynamicImage) -> DynamicImage {
    let rgb = logo.to_rgb8();
    let (mut x0, mut y0, mut x1, mut y1) = (rgb.width(), rgb.height(), 0, 0);
    for (x, y, px) in rgb.enumerate_pixels() {
        if px[0] as u16 + px[1] as u16 + px[2] as u16 > TRIM_LUMA {
            x0 = x0.min(x);
            y0 = y0.min(y);
            x1 = x1.max(x);
            y1 = y1.max(y);
        }
    }
    if x1 <= x0 || y1 <= y0 {
        return logo;
    }
    logo.crop_imm(x0, y0, x1 - x0 + 1, y1 - y0 + 1)
}

/// Resample the logo so one source pixel lands on exactly one braille dot.
///
/// The widget colours a whole cell from the last dot drawn in it, so the logo is
/// first resolved down to one pixel per cell and then blown back up to the dot
/// grid — every dot in a cell then shares a colour instead of fighting for it.
fn cell_image(logo: &DynamicImage, cells_w: u16, cells_h: u16) -> RgbImage {
    let cells = logo
        .resize_exact(cells_w as u32, cells_h as u32, FilterType::Lanczos3)
        .to_rgb8();
    // Downscaling smears the small lettering into grey, so pull the edges back
    imageops::unsharpen(&cells, 0.8, 1)
}

fn splash_screen(cells: &RgbImage, steps: i32) -> Option<SplashScreen> {
    let mut data = Vec::new();
    DynamicImage::ImageRgb8(cells.clone())
        .resize_exact(cells.width() * 2, cells.height() * 4, FilterType::Nearest)
        .write_to(&mut Cursor::new(&mut data), ImageFormat::Png)
        .ok()?;
    SplashScreen::new(SplashConfig {
        image_data: &data,
        sha256sum: None,
        render_steps: steps,
        use_colors: true,
    })
    .ok()
}

/// Lay the tint over the mark, keeping each pixel's brightness so the
/// background stays black.
fn colourise(cells: &RgbImage, tint: Tint, phase: f32) -> RgbImage {
    let width = cells.width().max(1) as f32;
    let height = cells.height().max(1) as f32;
    let mut out = cells.clone();
    for (x, y, px) in out.enumerate_pixels_mut() {
        let luma = (px[0] as u32 * 299 + px[1] as u32 * 587 + px[2] as u32 * 114) / 1000;
        let (r, g, b) = tint.colour(x, y, width, height, phase);
        *px = Rgb([
            (r as u32 * luma / 255) as u8,
            (g as u32 * luma / 255) as u8,
            (b as u32 * luma / 255) as u8,
        ]);
    }
    out
}

/// Sweep the mark in from one edge, dimming what the wipe has not reached yet.
fn wipe(mut cells: RgbImage, reveal: Reveal, progress: f32) -> RgbImage {
    let width = cells.width().max(1) as f32;
    let height = cells.height().max(1) as f32;
    // Run the edge past the far side so the last pixels reach full brightness
    let edge = progress * (1.0 + WIPE_FEATHER);
    for (x, y, px) in cells.enumerate_pixels_mut() {
        let lit = ((edge - reveal.distance(x, y, width, height)) / WIPE_FEATHER).clamp(0.0, 1.0);
        let scale = (lit * 255.0) as u32;
        *px = Rgb([
            (px[0] as u32 * scale / 255) as u8,
            (px[1] as u32 * scale / 255) as u8,
            (px[2] as u32 * scale / 255) as u8,
        ]);
    }
    cells
}

/// The logo as it should look this frame, before it is handed to the widget.
fn frame_image(cells: &RgbImage, look: Look, tick: u64) -> RgbImage {
    let tinted = if look.tint == Tint::None {
        cells.clone()
    } else {
        colourise(cells, look.tint, look.tint.phase(tick))
    };
    if look.wipes() {
        wipe(tinted, look.reveal, (tick as f32 / look.wipe_ms() as f32).min(1.0))
    } else {
        tinted
    }
}

/// Rebuild the splash when the wipe or the tint has moved on. A one-step screen
/// paints at full colour on its first render, so the reveal is not restarted.
pub(super) fn update(splash: &mut Splash, look: Look, tick: u64) {
    let wiping = look.wipes() && tick <= look.wipe_ms();
    let drifting = look.tint.drifts() && splash.screen.is_rendered();
    if !wiping && !drifting {
        return;
    }
    if let Some(screen) = splash_screen(&frame_image(&splash.cells, look, tick), 1) {
        splash.screen = screen;
    }
}

/// Fit the logo to the screen at its own aspect, leaving room for the text below.
pub(super) fn build_splash(width: u16, height: u16, look: Look) -> Option<Splash> {
    let logo =
        trim_padding(image::load_from_memory_with_format(look.logo.bytes(), ImageFormat::Png).ok()?);
    let (px_w, px_h) = (logo.width().max(1), logo.height().max(1));

    let room_w = (width as u32 * SPLASH_WIDTH_PCT / 100).min(SPLASH_MAX_W as u32);
    let room_h = height.saturating_sub(TEXT_ROWS + 2) as u32;
    let cells_w = room_w.min(room_h * CELL_ASPECT * px_w / px_h) as u16;
    let cells_h = (cells_w as u32 * px_h / (px_w * CELL_ASPECT)).max(1) as u16;
    if cells_w < SPLASH_MIN_W || cells_h < SPLASH_MIN_ROWS {
        return None;
    }

    // A directional wipe does the revealing itself, so the widget paints in one step
    let steps = if look.effect && !look.wipes() { look.steps() } else { 1 };
    let cells = cell_image(&logo, cells_w, cells_h);
    let screen = splash_screen(&frame_image(&cells, look, 0), steps)?;
    Some(Splash {
        screen,
        cells,
        cells_w,
        cells_h,
    })
}

/// Play the intro animation. Returns early if the user presses a key.
pub fn play<B: Backend>(
    terminal: &mut Terminal<B>,
    accent: Color,
    look: Look,
    duration_ms: u64,
) -> io::Result<()> {
    let start = Instant::now();
    let total = Duration::from_millis(duration_ms);

    let size = terminal.size()?;
    let mut splash = build_splash(size.width, size.height, look);

    loop {
        let elapsed = start.elapsed();
        if elapsed >= total {
            break;
        }
        let tick = elapsed.as_millis() as u64;
        let progress = eased(tick as f64 / duration_ms as f64);
        let accent = frame_accent(accent, look.tint, tick);
        if let Some(splash) = splash.as_mut() {
            update(splash, look, tick);
        }

        terminal.draw(|frame| {
            render(frame, frame.area(), splash.as_mut(), progress, tick, accent, false)
        })?;

        if skip_requested()? {
            return Ok(());
        }
    }

    // Hold on "SYSTEM READY" so the boot reads as finished
    let hold = Instant::now();
    loop {
        let elapsed = hold.elapsed();
        if elapsed >= Duration::from_millis(HOLD_MS) {
            break;
        }
        let tick = duration_ms + elapsed.as_millis() as u64;
        let accent = frame_accent(accent, look.tint, tick);
        if let Some(splash) = splash.as_mut() {
            update(splash, look, tick);
        }
        terminal.draw(|frame| {
            render(frame, frame.area(), splash.as_mut(), 1.0, tick, accent, true)
        })?;
        if skip_requested()? {
            break;
        }
    }

    Ok(())
}

/// Only the drifting rainbow takes over the bar and status text; every other
/// tint leaves them on the tuiOS colour.
pub(super) fn frame_accent(accent: Color, tint: Tint, tick: u64) -> Color {
    if !tint.drifts() {
        return accent;
    }
    let (r, g, b) = hue_to_rgb(tint.phase(tick));
    Color::Rgb(r, g, b)
}

pub(super) fn render(
    frame: &mut Frame,
    area: Rect,
    splash: Option<&mut Splash>,
    progress: f64,
    tick: u64,
    accent: Color,
    ready: bool,
) {
    let progress = progress.clamp(0.0, 1.0);

    let dim = Color::Rgb(70, 70, 70);
    let text = Color::Rgb(170, 170, 170);

    let content_w = area.width.saturating_sub(4).clamp(12, CONTENT_MAX_W);
    let (logo_w, logo_rows) = splash
        .as_ref()
        .map_or((0, 0), |s| (s.cells_w, s.cells_h));

    let mut lines: Vec<Line> = Vec::new();

    if splash.is_none() {
        let pad = (content_w as usize).saturating_sub(7) / 2;
        lines.push(Line::from(vec![
            Span::raw(" ".repeat(pad)),
            Span::styled(
                " tui ",
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                "OS",
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
        ]));
    }

    lines.push(Line::default());

    let subtitle = if content_w >= 24 {
        "Created by Ben Mulligan"
    } else {
        "Ben Mulligan"
    };
    lines.push(Line::from(center(
        Span::styled(subtitle, Style::default().fg(dim)),
        content_w,
    )));

    lines.push(Line::default());
    lines.push(Line::default());

    // Progress bar: solid accent fill, dark trough, framed by dim half-blocks
    let bar_w = content_w.saturating_sub(2) as usize;
    let mut bar: Vec<Span> = vec![Span::styled("▐", Style::default().fg(dim))];
    bar.extend(bar_spans(bar_w, progress, accent, dim));
    bar.push(Span::styled("▌", Style::default().fg(dim)));
    lines.push(Line::from(bar));

    lines.push(Line::default());

    // Status line: stage on the left with a blinking cursor, percentage on the right
    let stage = if ready {
        "SYSTEM READY"
    } else {
        STAGES[((progress * STAGES.len() as f64) as usize).min(STAGES.len() - 1)]
    };
    let percent = format!("{:>3}%", (progress * 100.0).round() as u16);
    let cursor = if (tick / 400) % 2 == 0 { "█" } else { " " };
    let used = 2 + stage.chars().count() + 1 + percent.chars().count();
    let gap = (content_w as usize).saturating_sub(used);
    lines.push(Line::from(vec![
        Span::styled("> ", Style::default().fg(accent)),
        Span::styled(stage, Style::default().fg(text)),
        Span::styled(cursor, Style::default().fg(accent)),
        Span::raw(" ".repeat(gap)),
        Span::styled(percent, Style::default().fg(accent)),
    ]));

    let text_h = lines.len() as u16;
    let top = area.y + area.height.saturating_sub(logo_rows + text_h) / 2;

    if let Some(splash) = splash {
        frame.render_widget(
            &mut splash.screen,
            Rect {
                x: area.x + area.width.saturating_sub(logo_w) / 2,
                y: top,
                width: logo_w.min(area.width),
                height: logo_rows.min(area.height),
            },
        );
    }

    frame.render_widget(
        Paragraph::new(lines),
        Rect {
            x: area.x + area.width.saturating_sub(content_w) / 2,
            y: top + logo_rows,
            width: content_w.min(area.width),
            height: text_h.min(area.height.saturating_sub(logo_rows)),
        },
    );
}









