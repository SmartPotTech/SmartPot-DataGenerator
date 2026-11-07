//! Pruebas de integración del administrador de cultivos virtuales y la API HTTP.

use axum::body::{to_bytes, Body};
use axum::http::{header, Request, StatusCode};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use tower::ServiceExt;

use smartpot_datagenerator::api::{create_router, AppState};
use smartpot_datagenerator::config::{DeviceConfig, Settings};
use smartpot_datagenerator::domain::actuator::{Action, Actuator};
use smartpot_datagenerator::domain::environment::Environment;
use smartpot_datagenerator::domain::weather::{condition, Weather, WeatherClient};
use smartpot_datagenerator::pots::{PotConfig, PotManager};

const TOKEN: &str = "test-simulator-token-0123456789";
const CROP: &str = "66f5a1000000000000000201";
const KEY: &str = "device-key-for-tests-0001";

fn test_settings() -> Settings {
    Settings {
        host: "localhost".to_string(),
        port: 1883,
        tls: false,
        ca_file: None,
        topic_prefix: "smartpot/v1".to_string(),
        interval_seconds: 30.0,
        devices: vec![DeviceConfig {
            crop_id: "66f5a1000000000000000101".to_string(),
            key: "demo-key".to_string(),
            crop_type: "LETTUCE".to_string(),
        }],
        utc_offset_hours: -5.0,
        api_token: Some(TOKEN.to_string()),
        api_port: 8081,
        payload_format: "json".to_string(),
    }
}

#[test]
fn test_weather_codes_map_to_scenes() {
    assert_eq!(condition(0), ("CLEAR", "Clear"));
    assert_eq!(condition(63).0, "RAIN");
    assert_eq!(condition(95).0, "STORM");
    assert_eq!(condition(1234), ("CLOUDY", "Cloudy"));
}

#[tokio::test]
async fn test_manual_gauges_change_the_reading_and_actuators_still_act() {
    let mut env = Environment::new("LETTUCE", Some(1), 0.0);
    let _ = env.set_mode("MANUAL");

    let mut manual = HashMap::new();
    manual.insert("soilMoisture".to_string(), 35.0);
    manual.insert("temperature".to_string(), 30.0);
    env.set_manual(&manual);

    assert_eq!(env.state.get("soilMoisture").copied().unwrap(), 35.0);

    let _ = env.apply(Actuator::WaterPump, Action::Activate, Some(15), 100.0);
    env.step(60.0, 160.0);
    assert!(env.state.get("soilMoisture").copied().unwrap() > 45.0);

    for m in 1..60 {
        env.step(60.0, 160.0 + m as f64 * 60.0);
    }
    let temp = env.state.get("temperature").copied().unwrap();
    assert!((temp - 30.0).abs() < 1.0);
}

#[tokio::test]
async fn test_repeating_the_configuration_keeps_the_effect_of_actuators() {
    let settings = test_settings();
    let manager = PotManager::new(settings, WeatherClient::default());

    let mut manual = HashMap::new();
    manual.insert("soilMoisture".to_string(), 40.0);

    let config = PotConfig {
        crop_id: CROP.to_string(),
        key: KEY.to_string(),
        crop_type: "LETTUCE".to_string(),
        mode: "MANUAL".to_string(),
        manual: manual.clone(),
        location: None,
        interval_seconds: 30.0,
        managed: true,
        setting: None,
        exposure: None,
    };

    let pot = manager.upsert(config.clone()).await.unwrap();
    {
        let p = pot.read().await;
        let mut env = p.device.environment.write().await;
        env.state.insert("soilMoisture".to_string(), 70.0);
    }

    let _ = manager.upsert(config).await.unwrap();

    let moisture = {
        let p = pot.read().await;
        let env = p.device.environment.read().await;
        env.state.get("soilMoisture").copied().unwrap()
    };

    assert_eq!(moisture, 70.0);
}

#[tokio::test]
async fn test_control_api_requires_the_token() {
    let settings = test_settings();
    let manager = Arc::new(PotManager::new(settings, WeatherClient::default()));
    manager.load_static().await.unwrap();

    let state = AppState {
        manager: Arc::clone(&manager),
        api_token: Some(TOKEN.to_string()),
    };
    let app = create_router(state);

    // /health no requiere token
    let req = Request::builder()
        .uri("/health")
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let body: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(body["pots"], 1);

    // /v1/pots sin cabecera -> 401 con desafío Bearer
    let req = Request::builder()
        .uri("/v1/pots")
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(resp.headers()[header::WWW_AUTHENTICATE], "Bearer");
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let body: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(body["detail"], "Invalid service token");

    // /v1/pots con token erróneo -> 401
    let req = Request::builder()
        .uri("/v1/pots")
        .header(header::AUTHORIZATION, "Bearer invalid-token")
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

    // Un prefijo del token válido tampoco se acepta: la comparación no es por prefijo.
    let req = Request::builder()
        .uri("/v1/pots")
        .header(header::AUTHORIZATION, format!("Bearer {}", &TOKEN[..12]))
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

    // El esquema es obligatorio y distingue mayúsculas.
    for value in ["Token valid", "bearer valid", TOKEN] {
        let req = Request::builder()
            .uri("/v1/pots")
            .header(header::AUTHORIZATION, value)
            .body(Body::empty())
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    // /v1/pots con token válido -> 200
    let req = Request::builder()
        .uri("/v1/pots")
        .header(header::AUTHORIZATION, format!("Bearer {TOKEN}"))
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_control_api_creates_updates_and_removes_a_pot() {
    let settings = test_settings();
    let manager = Arc::new(PotManager::new(settings, WeatherClient::default()));
    manager.load_static().await.unwrap();

    let state = AppState {
        manager: Arc::clone(&manager),
        api_token: Some(TOKEN.to_string()),
    };
    let app = create_router(state);

    // PUT crea cultivo virtual en modo AUTO
    let create_body = serde_json::json!({
        "key": KEY,
        "cropType": "LETTUCE",
        "mode": "AUTO",
        "intervalSeconds": 15
    });

    let req = Request::builder()
        .method("PUT")
        .uri(format!("/v1/pots/{CROP}"))
        .header(header::AUTHORIZATION, format!("Bearer {TOKEN}"))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(create_body.to_string()))
        .unwrap();

    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let body: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(body["mode"], "AUTO");
    assert_eq!(body["managed"], true);

    // GET obtiene el cultivo creado
    let req = Request::builder()
        .uri(format!("/v1/pots/{CROP}"))
        .header(header::AUTHORIZATION, format!("Bearer {TOKEN}"))
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // DELETE elimina el cultivo creado -> 204
    let req = Request::builder()
        .method("DELETE")
        .uri(format!("/v1/pots/{CROP}"))
        .header(header::AUTHORIZATION, format!("Bearer {TOKEN}"))
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);

    // GET posterior -> 404
    let req = Request::builder()
        .uri(format!("/v1/pots/{CROP}"))
        .header(header::AUTHORIZATION, format!("Bearer {TOKEN}"))
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_control_api_updates_manual_and_weather_modes() {
    let settings = test_settings();
    let manager = Arc::new(PotManager::new(settings, WeatherClient::default()));
    manager.load_static().await.unwrap();

    let state = AppState {
        manager: Arc::clone(&manager),
        api_token: Some(TOKEN.to_string()),
    };
    let app = create_router(state);

    let location = serde_json::json!({
        "name": "Medellin",
        "latitude": 6.2442,
        "longitude": -75.5812
    });

    // Creación en MANUAL con todos los medidores y ubicación.
    let create_body = serde_json::json!({
        "key": KEY,
        "cropType": "LETTUCE",
        "mode": "MANUAL",
        "intervalSeconds": 15,
        "setting": "INDOOR",
        "exposure": "PARTIAL_SUN",
        "location": location,
        "manual": {
            "temperature": 31.5,
            "humidity": 55.0,
            "brightness": 900.0,
            "ph": 6.2,
            "tds": 700.0,
            "atmosphere": 1010.0,
            "soilMoisture": 45.0
        }
    });
    let req = Request::builder()
        .method("PUT")
        .uri(format!("/v1/pots/{CROP}"))
        .header(header::AUTHORIZATION, format!("Bearer {TOKEN}"))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(create_body.to_string()))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let body: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(body["mode"], "MANUAL");
    assert_eq!(body["managed"], true);
    assert_eq!(body["intervalSeconds"], 15.0);
    assert_eq!(body["setting"], "INDOOR");
    assert_eq!(body["exposure"], "PARTIAL_SUN");
    assert_eq!(body["manual"]["temperature"], 31.5);
    assert_eq!(body["manual"]["humidity"], 55.0);
    assert_eq!(body["manual"]["brightness"], 900.0);
    assert_eq!(body["manual"]["ph"], 6.2);
    assert_eq!(body["manual"]["tds"], 700.0);
    assert_eq!(body["manual"]["atmosphere"], 1010.0);
    assert_eq!(body["manual"]["soilMoisture"], 45.0);

    // Actualización parcial en MANUAL: cambia solo lo enviado.
    let update_body = serde_json::json!({
        "key": KEY,
        "cropType": "LETTUCE",
        "mode": "MANUAL",
        "intervalSeconds": 20,
        "location": location,
        "manual": { "temperature": 27.5, "soilMoisture": 48.0 }
    });
    let req = Request::builder()
        .method("PUT")
        .uri(format!("/v1/pots/{CROP}"))
        .header(header::AUTHORIZATION, format!("Bearer {TOKEN}"))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(update_body.to_string()))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let body: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(body["mode"], "MANUAL");
    assert_eq!(body["intervalSeconds"], 20.0);
    assert_eq!(body["manual"]["temperature"], 27.5);
    assert_eq!(body["manual"]["soilMoisture"], 48.0);
    assert_eq!(body["manual"]["humidity"], 55.0);

    // Se siembra el clima en el entorno para no depender de Open-Meteo; la
    // ubicación no cambia, así que la actualización a WEATHER no hace red.
    {
        let pot = manager.get(CROP).await.unwrap();
        let pot = pot.write().await;
        let mut env = pot.device.environment.write().await;
        env.weather = Some(Weather {
            temperature: 18.4,
            humidity: 72.0,
            cloud_cover: 30.0,
            radiation: 420.0,
            precipitation: 0.0,
            pressure: 1009.5,
            wind_speed: 3.2,
            is_day: true,
            code: 2,
            condition: "PARTLY_CLOUDY".to_string(),
            label: "Partly cloudy".to_string(),
            observed_at: "2026-11-06T12:00:00Z".to_string(),
        });
    }

    let weather_body = serde_json::json!({
        "key": KEY,
        "cropType": "LETTUCE",
        "mode": "WEATHER",
        "intervalSeconds": 30,
        "location": {
            "name": "Medellin",
            "latitude": 6.2442,
            "longitude": -75.5812
        }
    });
    let req = Request::builder()
        .method("PUT")
        .uri(format!("/v1/pots/{CROP}"))
        .header(header::AUTHORIZATION, format!("Bearer {TOKEN}"))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(weather_body.to_string()))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let body: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(body["mode"], "WEATHER");
    assert_eq!(body["weather"]["temperature"], 18.4);
    assert_eq!(body["weather"]["condition"], "PARTLY_CLOUDY");
    assert!(body["weatherError"].is_null());
}

#[tokio::test]
async fn test_control_api_rejects_invalid_configuration() {
    let settings = test_settings();
    let manager = Arc::new(PotManager::new(settings, WeatherClient::default()));
    manager.load_static().await.unwrap();

    let state = AppState {
        manager: Arc::clone(&manager),
        api_token: Some(TOKEN.to_string()),
    };
    let app = create_router(state);

    let cases = [
        (
            serde_json::json!({
                "key": "short",
                "cropType": "LETTUCE",
                "mode": "AUTO",
                "intervalSeconds": 30
            }),
            StatusCode::UNPROCESSABLE_ENTITY,
            "Key must be between 16 and 128 characters",
        ),
        (
            serde_json::json!({
                "key": KEY,
                "cropType": "lettuce",
                "mode": "AUTO",
                "intervalSeconds": 30
            }),
            StatusCode::UNPROCESSABLE_ENTITY,
            "cropType must contain 3 to 20 uppercase letters or underscores",
        ),
        (
            serde_json::json!({
                "key": KEY,
                "cropType": "LETTUCE",
                "mode": "AUTO",
                "intervalSeconds": 5
            }),
            StatusCode::UNPROCESSABLE_ENTITY,
            "intervalSeconds must be between 10 and 300",
        ),
        (
            serde_json::json!({
                "key": KEY,
                "cropType": "LETTUCE",
                "mode": "PAUSED",
                "intervalSeconds": 30
            }),
            StatusCode::BAD_REQUEST,
            "Unknown mode: PAUSED",
        ),
        (
            serde_json::json!({
                "key": KEY,
                "cropType": "LETTUCE",
                "mode": "WEATHER",
                "intervalSeconds": 30
            }),
            StatusCode::BAD_REQUEST,
            "Weather mode requires a location",
        ),
    ];

    for (payload, expected_status, expected_detail) in cases {
        let req = Request::builder()
            .method("PUT")
            .uri(format!("/v1/pots/{CROP}"))
            .header(header::AUTHORIZATION, format!("Bearer {TOKEN}"))
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(payload.to_string()))
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), expected_status, "payload: {payload}");
        let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let body: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(body["detail"], expected_detail);
    }

    // Ningún intento inválido debe haber creado el cultivo.
    let req = Request::builder()
        .uri(format!("/v1/pots/{CROP}"))
        .header(header::AUTHORIZATION, format!("Bearer {TOKEN}"))
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_control_api_removal_is_idempotent_and_stops_the_crop() {
    let settings = test_settings();
    let manager = Arc::new(PotManager::new(settings, WeatherClient::default()));
    manager.load_static().await.unwrap();

    let state = AppState {
        manager: Arc::clone(&manager),
        api_token: Some(TOKEN.to_string()),
    };
    let app = create_router(state);

    let create_body = serde_json::json!({
        "key": KEY,
        "cropType": "LETTUCE",
        "mode": "AUTO",
        "intervalSeconds": 30
    });
    let req = Request::builder()
        .method("PUT")
        .uri(format!("/v1/pots/{CROP}"))
        .header(header::AUTHORIZATION, format!("Bearer {TOKEN}"))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(create_body.to_string()))
        .unwrap();
    assert_eq!(
        app.clone().oneshot(req).await.unwrap().status(),
        StatusCode::OK
    );

    for _ in 0..2 {
        let req = Request::builder()
            .method("DELETE")
            .uri(format!("/v1/pots/{CROP}"))
            .header(header::AUTHORIZATION, format!("Bearer {TOKEN}"))
            .body(Body::empty())
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NO_CONTENT);
    }

    let req = Request::builder()
        .uri(format!("/v1/pots/{CROP}"))
        .header(header::AUTHORIZATION, format!("Bearer {TOKEN}"))
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);

    // El cultivo estático sigue existiendo; solo se detuvo el dinámico.
    let req = Request::builder()
        .uri("/v1/pots")
        .header(header::AUTHORIZATION, format!("Bearer {TOKEN}"))
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let body: Value = serde_json::from_slice(&bytes).unwrap();
    let pots = body.as_array().unwrap();
    assert_eq!(pots.len(), 1);
    assert_eq!(pots[0]["cropId"], "66f5a1000000000000000101");
}

#[tokio::test]
async fn test_without_token_the_control_api_is_disabled() {
    let mut settings = test_settings();
    settings.api_token = None;
    let manager = Arc::new(PotManager::new(settings, WeatherClient::default()));

    let state = AppState {
        manager: Arc::clone(&manager),
        api_token: None,
    };
    let app = create_router(state);

    let req = Request::builder()
        .uri("/health")
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let body: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(body["api"], "DISABLED");

    let req = Request::builder()
        .uri("/v1/pots")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test]
async fn test_static_pots_are_protected() {
    let settings = test_settings();
    let manager = Arc::new(PotManager::new(settings, WeatherClient::default()));
    manager.load_static().await.unwrap();

    let state = AppState {
        manager: Arc::clone(&manager),
        api_token: Some(TOKEN.to_string()),
    };
    let app = create_router(state);

    // El cultivo estático se puede consultar y sigue siendo no gestionado.
    let req = Request::builder()
        .uri("/v1/pots/66f5a1000000000000000101")
        .header(header::AUTHORIZATION, format!("Bearer {TOKEN}"))
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let body: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(body["managed"], false);

    // Actualizarlo con PUT también se rechaza: no se puede convertir en dinámico.
    let update_body = serde_json::json!({
        "key": "another-device-key-0001",
        "cropType": "LETTUCE",
        "mode": "AUTO",
        "intervalSeconds": 30
    });
    let req = Request::builder()
        .method("PUT")
        .uri("/v1/pots/66f5a1000000000000000101")
        .header(header::AUTHORIZATION, format!("Bearer {TOKEN}"))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(update_body.to_string()))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::CONFLICT);
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let body: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(body["detail"], "This crop is defined in SIMULATOR_DEVICES");

    // Intento de borrar el cultivo estático 66f5a1000000000000000101 -> 409 Conflict
    let req = Request::builder()
        .method("DELETE")
        .uri("/v1/pots/66f5a1000000000000000101")
        .header(header::AUTHORIZATION, format!("Bearer {TOKEN}"))
        .body(Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::CONFLICT);
}
