//! Cliente del servicio de clima Open-Meteo con caché por coordenadas.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{watch, Mutex, RwLock};

const WEATHER_URL: &str = "https://api.open-meteo.com/v1/forecast";
const GEOCODING_URL: &str = "https://geocoding-api.open-meteo.com/v1/search";
const TIMEOUT: Duration = Duration::from_secs(8);
const PLACES_FRESH_TTL: Duration = Duration::from_secs(3600);
const PLACES_STALE_TTL: Duration = Duration::from_secs(24 * 3600);
const PLACES_CACHE_MAX_ENTRIES: usize = 2048;
const USER_AGENT: &str =
    "SmartPot-Simulator/2.0 (+https://github.com/SmartPotTech/SmartPot-DataGenerator)";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Place {
    pub name: String,
    pub latitude: f64,
    pub longitude: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub country: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub region: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Weather {
    pub temperature: f64,
    pub humidity: f64,
    pub cloud_cover: f64,
    pub radiation: f64,
    pub precipitation: f64,
    pub pressure: f64,
    pub wind_speed: f64,
    pub is_day: bool,
    pub code: i32,
    pub condition: String,
    pub label: String,
    pub observed_at: String,
}

/// Mapea los códigos WMO al identificador de escena y texto en inglés.
pub fn condition(code: i32) -> (&'static str, &'static str) {
    match code {
        0 => ("CLEAR", "Clear"),
        1 => ("MOSTLY_CLEAR", "Mostly clear"),
        2 => ("PARTLY_CLOUDY", "Partly cloudy"),
        3 => ("CLOUDY", "Cloudy"),
        45 | 48 => ("FOG", "Fog"),
        51 | 53 | 55 | 56 | 57 => ("DRIZZLE", "Drizzle"),
        61 | 63 | 65 | 66 | 67 | 80 | 81 | 82 => ("RAIN", "Rain"),
        71 | 73 | 75 | 77 | 85 | 86 => ("SNOW", "Snow"),
        95 | 96 | 99 => ("STORM", "Storm"),
        _ => ("CLOUDY", "Cloudy"),
    }
}

#[derive(Debug, Clone)]
struct CachedEntry {
    weather: Weather,
    fetched_at: Instant,
}

#[derive(Debug, Clone)]
struct CachedPlaces {
    places: Vec<Place>,
    fetched_at: Instant,
}

type PlacesFlightSender = watch::Sender<Option<Result<Vec<Place>, String>>>;

#[derive(Debug, Clone)]
pub struct WeatherClient {
    client: reqwest::Client,
    ttl: Duration,
    places_ttl: Duration,
    cache: Arc<RwLock<HashMap<(i64, i64), CachedEntry>>>,
    places_cache: Arc<RwLock<HashMap<(String, u8), CachedPlaces>>>,
    places_inflight: Arc<Mutex<HashMap<(String, u8), PlacesFlightSender>>>,
}

impl Default for WeatherClient {
    fn default() -> Self {
        Self::new(Duration::from_secs(600))
    }
}

impl WeatherClient {
    pub fn new(ttl: Duration) -> Self {
        let client = reqwest::Client::builder()
            .timeout(TIMEOUT)
            .user_agent(USER_AGENT)
            .build()
            .unwrap_or_default();

        Self {
            client,
            ttl,
            places_ttl: PLACES_FRESH_TTL,
            cache: Arc::new(RwLock::new(HashMap::new())),
            places_cache: Arc::new(RwLock::new(HashMap::new())),
            places_inflight: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Redondea las coordenadas a 2 decimales para la clave de caché.
    fn cache_key(lat: f64, lon: f64) -> (i64, i64) {
        ((lat * 100.0).round() as i64, (lon * 100.0).round() as i64)
    }

    /// Consulta el clima actual para las coordenadas dadas.
    pub async fn current(&self, latitude: f64, longitude: f64) -> Result<Weather, String> {
        let key = Self::cache_key(latitude, longitude);

        {
            let cache = self.cache.read().await;
            if let Some(entry) = cache.get(&key) {
                if entry.fetched_at.elapsed() < self.ttl {
                    return Ok(entry.weather.clone());
                }
            }
        }

        let lat_rounded = (latitude * 100.0).round() / 100.0;
        let lon_rounded = (longitude * 100.0).round() / 100.0;

        let url = format!(
            "{}?latitude={}&longitude={}&current=temperature_2m,relative_humidity_2m,is_day,precipitation,weather_code,cloud_cover,surface_pressure,wind_speed_10m,shortwave_radiation&timezone=auto",
            WEATHER_URL, lat_rounded, lon_rounded
        );

        let res = self
            .client
            .get(&url)
            .header("Accept", "application/json")
            .send()
            .await
            .map_err(|e| format!("Open-Meteo request failed: {e}"))?;

        if !res.status().is_success() {
            return Err(format!("Open-Meteo returned HTTP {}", res.status()));
        }

        let body: serde_json::Value = res
            .json()
            .await
            .map_err(|e| format!("Error deserializing Open-Meteo JSON: {e}"))?;

        let current = body
            .get("current")
            .ok_or_else(|| "Open-Meteo response missing 'current' field".to_string())?;

        let code = current
            .get("weather_code")
            .and_then(|v| v.as_i64())
            .unwrap_or(0) as i32;

        let (cond, label) = condition(code);

        let weather = Weather {
            temperature: current
                .get("temperature_2m")
                .and_then(|v| v.as_f64())
                .unwrap_or(20.0),
            humidity: current
                .get("relative_humidity_2m")
                .and_then(|v| v.as_f64())
                .unwrap_or(50.0),
            cloud_cover: current
                .get("cloud_cover")
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0),
            radiation: current
                .get("shortwave_radiation")
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0),
            precipitation: current
                .get("precipitation")
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0),
            pressure: current
                .get("surface_pressure")
                .and_then(|v| v.as_f64())
                .unwrap_or(1013.0),
            wind_speed: current
                .get("wind_speed_10m")
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0),
            is_day: current
                .get("is_day")
                .and_then(|v| v.as_i64())
                .map(|v| v != 0)
                .unwrap_or(true),
            code,
            condition: cond.to_string(),
            label: label.to_string(),
            observed_at: current
                .get("time")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
        };

        {
            let mut cache = self.cache.write().await;
            cache.insert(
                key,
                CachedEntry {
                    weather: weather.clone(),
                    fetched_at: Instant::now(),
                },
            );
        }

        Ok(weather)
    }

    /// Busca lugares geográficos por nombre con caché en memoria, fusión de consultas concurrentes y respaldo temporal.
    pub async fn search(&self, query: &str, count: u8) -> Result<Vec<Place>, String> {
        self.search_with_url(GEOCODING_URL, query, count).await
    }

    async fn search_with_url(
        &self,
        base_url: &str,
        query: &str,
        count: u8,
    ) -> Result<Vec<Place>, String> {
        let trimmed = query.trim();
        let key = (trimmed.to_lowercase(), count);

        {
            let cache = self.places_cache.read().await;
            if let Some(entry) = cache.get(&key) {
                if entry.fetched_at.elapsed() < self.places_ttl {
                    return Ok(entry.places.clone());
                }
            }
        }

        let (tx_opt, mut rx_opt) = {
            let mut inflight = self.places_inflight.lock().await;
            if let Some(existing_tx) = inflight.get(&key) {
                (None, Some(existing_tx.subscribe()))
            } else {
                let (tx, _rx) = watch::channel(None);
                inflight.insert(key.clone(), tx.clone());
                (Some(tx), None)
            }
        };

        if let Some(mut rx) = rx_opt.take() {
            let _ = rx.wait_for(|val| val.is_some()).await;
            if let Some(res) = rx.borrow().clone() {
                return res;
            }
        }

        struct InflightCleanup {
            key: (String, u8),
            inflight: Arc<Mutex<HashMap<(String, u8), PlacesFlightSender>>>,
            tx: PlacesFlightSender,
            result: Option<Result<Vec<Place>, String>>,
        }

        impl Drop for InflightCleanup {
            fn drop(&mut self) {
                let res = self
                    .result
                    .take()
                    .unwrap_or_else(|| Err("Open-Meteo geocoding flight aborted".to_string()));
                let _ = self.tx.send(Some(res));
                if let Ok(mut guard) = self.inflight.try_lock() {
                    guard.remove(&self.key);
                } else {
                    let inflight = Arc::clone(&self.inflight);
                    let key = self.key.clone();
                    tokio::spawn(async move {
                        let mut guard = inflight.lock().await;
                        guard.remove(&key);
                    });
                }
            }
        }

        let mut cleanup = InflightCleanup {
            key: key.clone(),
            inflight: Arc::clone(&self.places_inflight),
            tx: tx_opt.expect("leader always holds sender"),
            result: None,
        };

        let final_res = match self.fetch_places(base_url, trimmed, count).await {
            Ok(places) => {
                let mut cache = self.places_cache.write().await;
                if cache.len() >= PLACES_CACHE_MAX_ENTRIES {
                    cache.clear();
                }
                cache.insert(
                    key.clone(),
                    CachedPlaces {
                        places: places.clone(),
                        fetched_at: Instant::now(),
                    },
                );
                Ok(places)
            }
            Err(err) => {
                let cache = self.places_cache.read().await;
                if let Some(entry) = cache.get(&key) {
                    if entry.fetched_at.elapsed() < PLACES_STALE_TTL {
                        tracing::warn!(
                            query = trimmed,
                            error = %err,
                            "serving stale geocoding places cache after upstream error"
                        );
                        Ok(entry.places.clone())
                    } else {
                        Err(err)
                    }
                } else {
                    Err(err)
                }
            }
        };

        {
            let mut guard = self.places_inflight.lock().await;
            guard.remove(&cleanup.key);
        }
        cleanup.result = Some(final_res.clone());
        final_res
    }

    async fn fetch_places(
        &self,
        base_url: &str,
        query: &str,
        count: u8,
    ) -> Result<Vec<Place>, String> {
        let url = format!(
            "{}?name={}&count={}&language=en&format=json",
            base_url,
            urlencoding_simple(query),
            count
        );

        let res = self
            .client
            .get(&url)
            .header("Accept", "application/json")
            .send()
            .await
            .map_err(|e| format!("Open-Meteo geocoding error: {e}"))?;

        if !res.status().is_success() {
            return Err(format!("Geocoding returned HTTP {}", res.status()));
        }

        let body: serde_json::Value = res
            .json()
            .await
            .map_err(|e| format!("Error deserializing geocoding response: {e}"))?;

        let mut places = Vec::new();
        if let Some(results) = body.get("results").and_then(|v| v.as_array()) {
            for item in results {
                if let (Some(name), Some(lat), Some(lon)) = (
                    item.get("name").and_then(|v| v.as_str()),
                    item.get("latitude").and_then(|v| v.as_f64()),
                    item.get("longitude").and_then(|v| v.as_f64()),
                ) {
                    places.push(Place {
                        name: name.to_string(),
                        latitude: (lat * 10000.0).round() / 10000.0,
                        longitude: (lon * 10000.0).round() / 10000.0,
                        country: item
                            .get("country")
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_string()),
                        region: item
                            .get("admin1")
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_string()),
                    });
                }
            }
        }

        Ok(places)
    }
}

fn urlencoding_simple(s: &str) -> String {
    let mut encoded = String::new();
    for b in s.as_bytes() {
        match *b {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                encoded.push(*b as char);
            }
            b' ' => encoded.push('+'),
            other => {
                encoded.push_str(&format!("%{:02X}", other));
            }
        }
    }
    encoded
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{http::StatusCode, routing::get, Json, Router};
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use tokio::net::TcpListener;
    use tokio::sync::Notify;

    #[tokio::test]
    async fn search_caches_coalesces_and_falls_back_to_stale() {
        let calls = Arc::new(AtomicUsize::new(0));
        let fail = Arc::new(AtomicBool::new(false));
        let entered = Arc::new(Notify::new());
        let release = Arc::new(Notify::new());

        let app = Router::new().route(
            "/v1/search",
            get({
                let calls = Arc::clone(&calls);
                let fail = Arc::clone(&fail);
                let entered = Arc::clone(&entered);
                let release = Arc::clone(&release);
                move || async move {
                    calls.fetch_add(1, Ordering::SeqCst);
                    if fail.load(Ordering::SeqCst) {
                        return (StatusCode::TOO_MANY_REQUESTS, Json(serde_json::json!({})));
                    }
                    let released = release.notified();
                    entered.notify_one();
                    released.await;
                    (
                        StatusCode::OK,
                        Json(serde_json::json!({
                            "results": [{
                                "name": "Medellin",
                                "latitude": 6.25184,
                                "longitude": -75.56359,
                                "country": "Colombia",
                                "admin1": "Antioquia"
                            }]
                        })),
                    )
                }
            }),
        );

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        let base_url = format!("http://{addr}/v1/search");

        let client = WeatherClient::default();

        // 1. Concurrent callers coalesce into one upstream request.
        let c1 = client.clone();
        let u1 = base_url.clone();
        let mut leader = tokio::spawn(async move { c1.search_with_url(&u1, "Medellin", 5).await });
        tokio::select! {
            _ = entered.notified() => {}
            res = &mut leader => panic!("leader finished before reaching mock server: {res:?}"),
        }

        let mut followers = Vec::new();
        for _ in 0..5 {
            let cf = client.clone();
            let uf = base_url.clone();
            followers.push(tokio::spawn(async move {
                cf.search_with_url(&uf, "  medellin ", 5).await
            }));
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
        release.notify_waiters();

        let first = leader.await.unwrap().unwrap();
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].name, "Medellin");
        for f in followers {
            let places = f.await.unwrap().unwrap();
            assert_eq!(places, first);
        }
        assert_eq!(calls.load(Ordering::SeqCst), 1);

        // 2. Subsequent normalized query hits warm cache without calling upstream.
        let cached = client
            .search_with_url(&base_url, "MEDELLIN", 5)
            .await
            .unwrap();
        assert_eq!(cached, first);
        assert_eq!(calls.load(Ordering::SeqCst), 1);

        // 3. After fresh TTL expires, upstream 429 falls back to stale entry.
        {
            let mut guard = client.places_cache.write().await;
            let entry = guard.get_mut(&("medellin".to_string(), 5)).unwrap();
            entry.fetched_at = Instant::now() - PLACES_FRESH_TTL - Duration::from_secs(60);
        }
        fail.store(true, Ordering::SeqCst);
        let stale = client
            .search_with_url(&base_url, "Medellin", 5)
            .await
            .unwrap();
        assert_eq!(stale, first);
        assert_eq!(calls.load(Ordering::SeqCst), 2);

        server.abort();
    }
}
