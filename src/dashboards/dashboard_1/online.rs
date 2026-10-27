/// Network-backed panels for Dashboard 1: weather and connectivity.
///
/// Every request runs on a shared background worker so rendering never blocks on
/// the network. The worker starts on first use and shuts itself down once the
/// dashboard stops asking for data.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use lazy_static::lazy_static;
use serde_json::Value;

use crate::settings::persistence;

const WEATHER_INTERVAL: Duration = Duration::from_secs(900);
const CONNECTIVITY_INTERVAL: Duration = Duration::from_secs(300);
const POLL_INTERVAL: Duration = Duration::from_secs(5);
const IDLE_TIMEOUT: Duration = Duration::from_secs(45);
const HTTP_TIMEOUT: Duration = Duration::from_secs(10);

const GEOCODE_URL: &str = "https://geocoding-api.open-meteo.com/v1/search";
const FORECAST_URL: &str = "https://api.open-meteo.com/v1/forecast";
const PUBLIC_IP_URL: &str = "https://api.ipify.org";

pub const DEFAULT_LOCATION: &str = "London";

#[derive(Clone, PartialEq)]
pub enum Status {
    Disabled,
    Loading,
    Ready,
    Error(String),
}

#[derive(Clone, Default)]
pub struct Weather {
    pub place: String,
    pub temp: f64,
    pub feels_like: f64,
    pub high: f64,
    pub low: f64,
    pub humidity: u64,
    pub wind: f64,
    pub wind_unit: String,
    pub code: u64,
    pub is_day: bool,
    pub sunrise: String,
    pub sunset: String,
    pub unit: char,
}

#[derive(Clone, Default)]
pub struct Connectivity {
    pub online: bool,
    pub public_ip: Option<String>,
    pub latency_ms: Option<u64>,
}

#[derive(Clone)]
pub struct OnlineState {
    pub weather: Option<Weather>,
    pub weather_status: Status,
    pub weather_age: Option<Duration>,
    pub conn: Option<Connectivity>,
    pub conn_status: Status,
}

impl Default for OnlineState {
    fn default() -> Self {
        Self {
            weather: None,
            weather_status: Status::Loading,
            weather_age: None,
            conn: None,
            conn_status: Status::Loading,
        }
    }
}

struct Worker {
    weather: Option<Weather>,
    weather_status: Status,
    weather_at: Option<Instant>,
    weather_key: String,
    conn: Option<Connectivity>,
    conn_status: Status,
    conn_at: Option<Instant>,
    geo_cache: Option<(String, f64, f64, String)>,
}

lazy_static! {
    static ref STATE: Mutex<OnlineState> = Mutex::new(OnlineState::default());
    static ref LAST_READ: Mutex<Option<Instant>> = Mutex::new(None);
    static ref WORKER_RUNNING: AtomicBool = AtomicBool::new(false);
}

/// Current weather/connectivity state. Starts the worker on first call.
pub fn state() -> OnlineState {
    *LAST_READ.lock().unwrap() = Some(Instant::now());
    ensure_worker();
    STATE.lock().unwrap().clone()
}

/// Settings-configured location, falling back to the default.
pub fn configured_location(settings: &Value) -> String {
    let raw = persistence::get_str(settings, "dashboard.weather_location", DEFAULT_LOCATION);
    let trimmed = raw.trim().to_string();
    if trimmed.is_empty() {
        DEFAULT_LOCATION.to_string()
    } else {
        trimmed
    }
}

fn ensure_worker() {
    if WORKER_RUNNING.swap(true, Ordering::SeqCst) {
        return;
    }
    std::thread::spawn(|| {
        let mut worker = Worker {
            weather: None,
            weather_status: Status::Loading,
            weather_at: None,
            weather_key: String::new(),
            conn: None,
            conn_status: Status::Loading,
            conn_at: None,
            geo_cache: None,
        };
        loop {
            worker.tick();
            *STATE.lock().unwrap() = worker.publish();
            std::thread::sleep(POLL_INTERVAL);

            let idle = LAST_READ
                .lock()
                .unwrap()
                .map_or(true, |t| t.elapsed() > IDLE_TIMEOUT);
            if idle {
                WORKER_RUNNING.store(false, Ordering::SeqCst);
                return;
            }
        }
    });
}

impl Worker {
    fn publish(&self) -> OnlineState {
        OnlineState {
            weather: self.weather.clone(),
            weather_status: self.weather_status.clone(),
            weather_age: self.weather_at.map(|t| t.elapsed()),
            conn: self.conn.clone(),
            conn_status: self.conn_status.clone(),
        }
    }

    fn tick(&mut self) {
        let settings = persistence::load();
        if !persistence::get_bool(&settings, "dashboard.online_panels", true) {
            self.weather_status = Status::Disabled;
            self.conn_status = Status::Disabled;
            return;
        }

        let location = configured_location(&settings);
        let fahrenheit =
            persistence::get_str(&settings, "dashboard.weather_units", "Celsius") == "Fahrenheit";
        let key = format!("{}|{}", location, fahrenheit);
        let stale = self
            .weather_at
            .map_or(true, |t| t.elapsed() >= WEATHER_INTERVAL);

        if key != self.weather_key || stale {
            if key != self.weather_key {
                self.weather_key = key;
                self.weather_status = Status::Loading;
                self.geo_cache = None;
            }
            match self.fetch_weather(&location, fahrenheit) {
                Ok(w) => {
                    self.weather = Some(w);
                    self.weather_status = Status::Ready;
                    self.weather_at = Some(Instant::now());
                }
                Err(e) => {
                    self.weather_status = Status::Error(e);
                    // Retry on the normal cadence instead of hammering a failing endpoint.
                    self.weather_at = Some(Instant::now());
                }
            }
        }

        if self
            .conn_at
            .map_or(true, |t| t.elapsed() >= CONNECTIVITY_INTERVAL)
        {
            self.conn = Some(check_connectivity());
            self.conn_status = Status::Ready;
            self.conn_at = Some(Instant::now());
        }
    }

    fn fetch_weather(&mut self, location: &str, fahrenheit: bool) -> Result<Weather, String> {
        let (lat, lon, place) = match &self.geo_cache {
            Some((cached, lat, lon, place)) if cached == location => {
                (*lat, *lon, place.clone())
            }
            _ => {
                let found = geocode(location)?;
                self.geo_cache =
                    Some((location.to_string(), found.0, found.1, found.2.clone()));
                found
            }
        };

        let (temp_unit, wind_unit, symbol) = if fahrenheit {
            ("fahrenheit", "mph", 'F')
        } else {
            ("celsius", "kmh", 'C')
        };
        let url = format!(
            "{}?latitude={:.4}&longitude={:.4}&current=temperature_2m,relative_humidity_2m,\
apparent_temperature,is_day,weather_code,wind_speed_10m&daily=temperature_2m_max,\
temperature_2m_min,sunrise,sunset&timezone=auto&forecast_days=1&temperature_unit={}&\
wind_speed_unit={}",
            FORECAST_URL, lat, lon, temp_unit, wind_unit
        );
        let body: Value = get_json(&url)?;
        let current = body
            .get("current")
            .ok_or_else(|| "forecast response had no current block".to_string())?;
        let daily = body.get("daily");

        Ok(Weather {
            place,
            temp: num(current, "temperature_2m"),
            feels_like: num(current, "apparent_temperature"),
            high: daily.map(|d| first_num(d, "temperature_2m_max")).unwrap_or(0.0),
            low: daily.map(|d| first_num(d, "temperature_2m_min")).unwrap_or(0.0),
            humidity: num(current, "relative_humidity_2m") as u64,
            wind: num(current, "wind_speed_10m"),
            wind_unit: wind_unit.to_string(),
            code: num(current, "weather_code") as u64,
            is_day: num(current, "is_day") >= 1.0,
            sunrise: daily
                .map(|d| first_str(d, "sunrise"))
                .unwrap_or_default(),
            sunset: daily.map(|d| first_str(d, "sunset")).unwrap_or_default(),
            unit: symbol,
        })
    }
}

fn geocode(location: &str) -> Result<(f64, f64, String), String> {
    let url = format!(
        "{}?name={}&count=1&language=en&format=json",
        GEOCODE_URL,
        percent_encode(location)
    );
    let body: Value = get_json(&url)?;
    let first = body
        .get("results")
        .and_then(|r| r.as_array())
        .and_then(|a| a.first())
        .ok_or_else(|| format!("no match for \"{}\"", location))?;

    let lat = first.get("latitude").and_then(|v| v.as_f64());
    let lon = first.get("longitude").and_then(|v| v.as_f64());
    let (lat, lon) = match (lat, lon) {
        (Some(a), Some(b)) => (a, b),
        _ => return Err("geocoder returned no coordinates".to_string()),
    };

    let name = first
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or(location);
    let country = first
        .get("country_code")
        .and_then(|v| v.as_str())
        .or_else(|| first.get("country").and_then(|v| v.as_str()));
    let place = match country {
        Some(c) => format!("{}, {}", name, c),
        None => name.to_string(),
    };
    Ok((lat, lon, place))
}

fn check_connectivity() -> Connectivity {
    let started = Instant::now();
    match agent().get(PUBLIC_IP_URL).call() {
        Ok(resp) => {
            let latency = started.elapsed().as_millis() as u64;
            let ip = resp.into_string().ok().map(|s| s.trim().to_string());
            Connectivity {
                online: true,
                public_ip: ip.filter(|s| !s.is_empty() && s.len() <= 45),
                latency_ms: Some(latency),
            }
        }
        Err(_) => Connectivity::default(),
    }
}

fn agent() -> ureq::Agent {
    ureq::builder().timeout(HTTP_TIMEOUT).build()
}

fn get_json(url: &str) -> Result<Value, String> {
    let body = agent()
        .get(url)
        .call()
        .map_err(|e| short_error(&e.to_string()))?
        .into_string()
        .map_err(|e| short_error(&e.to_string()))?;
    serde_json::from_str(&body).map_err(|e| short_error(&e.to_string()))
}

fn short_error(msg: &str) -> String {
    let first = msg.lines().next().unwrap_or(msg);
    if first.len() > 60 {
        format!("{}…", &first[..60])
    } else {
        first.to_string()
    }
}

/// Percent-encode a settings-supplied string so it cannot alter the query string.
fn percent_encode(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for byte in input.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            _ => out.push_str(&format!("%{:02X}", byte)),
        }
    }
    out
}

fn num(v: &Value, key: &str) -> f64 {
    v.get(key).and_then(|x| x.as_f64()).unwrap_or(0.0)
}

fn first_num(v: &Value, key: &str) -> f64 {
    v.get(key)
        .and_then(|x| x.as_array())
        .and_then(|a| a.first())
        .and_then(|x| x.as_f64())
        .unwrap_or(0.0)
}

fn first_str(v: &Value, key: &str) -> String {
    v.get(key)
        .and_then(|x| x.as_array())
        .and_then(|a| a.first())
        .and_then(|x| x.as_str())
        // Open-Meteo returns "2026-10-26T07:12"; only the clock time is useful here.
        .and_then(|s| s.split('T').nth(1))
        .unwrap_or("—")
        .to_string()
}

/// WMO weather interpretation code → (icon, description).
pub fn describe(code: u64, is_day: bool) -> (&'static str, &'static str) {
    match code {
        0 => (if is_day { "☀" } else { "☾" }, "Clear"),
        1 => (if is_day { "☀" } else { "☾" }, "Mainly clear"),
        2 => ("⛅", "Partly cloudy"),
        3 => ("☁", "Overcast"),
        45 | 48 => ("≡", "Fog"),
        51 | 53 | 55 => ("☂", "Drizzle"),
        56 | 57 => ("☂", "Freezing drizzle"),
        61 => ("☂", "Light rain"),
        63 => ("☂", "Rain"),
        65 => ("☂", "Heavy rain"),
        66 | 67 => ("☂", "Freezing rain"),
        71 => ("❄", "Light snow"),
        73 => ("❄", "Snow"),
        75 => ("❄", "Heavy snow"),
        77 => ("❄", "Snow grains"),
        80 | 81 => ("☂", "Rain showers"),
        82 => ("☂", "Violent showers"),
        85 | 86 => ("❄", "Snow showers"),
        95 => ("⚡", "Thunderstorm"),
        96 | 99 => ("⚡", "Storm with hail"),
        _ => ("•", "Unknown"),
    }
}
