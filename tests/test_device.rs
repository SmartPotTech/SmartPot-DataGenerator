//! Pruebas unitarias del dispositivo simulado y contrato MQTT v1.

use std::sync::Arc;
use tokio::sync::RwLock;

use smartpot_datagenerator::config::parse_devices;
use smartpot_datagenerator::device::{capabilities_payload, DeviceTopics, SimulatedDevice};
use smartpot_datagenerator::domain::environment::Environment;
use smartpot_datagenerator::domain::limits::get_limits;

const CROP: &str = "66f5a1000000000000000101";

#[test]
fn test_topics_follow_the_v1_contract() {
    let topics = DeviceTopics::new("smartpot/v1", CROP);
    assert_eq!(topics.telemetry, format!("smartpot/v1/{CROP}/telemetry"));
    assert_eq!(topics.commands, format!("smartpot/v1/{CROP}/commands"));
    assert_eq!(topics.ack, format!("smartpot/v1/{CROP}/commands/ack"));
    assert_eq!(topics.status, format!("smartpot/v1/{CROP}/status"));
    assert_eq!(
        topics.capabilities,
        format!("smartpot/v1/{CROP}/capabilities")
    );
}

#[test]
fn test_capabilities_follow_the_audit_schema() {
    let payload = capabilities_payload();
    assert!(
        payload.len() <= 2048,
        "Capabilities payload must stay under 2 KB, got: {}",
        payload.len()
    );

    let value: serde_json::Value = serde_json::from_str(&payload).unwrap();
    assert_eq!(value["schema"], 1);
    assert_eq!(value["device"]["firmware"], "smartpot-datagenerator/2.0.0");
    assert_eq!(value["device"]["hardware"], "virtual");
    assert_eq!(value["device"]["telemetry"]["formats"][0], "json-v1");
    assert_eq!(value["device"]["telemetry"]["formats"][1], "compact-v2");
    assert_eq!(value["device"]["telemetry"]["compactBytes"], 27);

    let sensors = value["sensors"].as_array().unwrap();
    assert_eq!(sensors.len(), 7);
    let expected = [
        ("temperature", -20.0, 60.0),
        ("humidity", 0.0, 100.0),
        ("atmosphere", 300.0, 1100.0),
        ("brightness", 0.0, 2000.0),
        ("ph", 0.0, 14.0),
        ("tds", 0.0, 3000.0),
        ("soilMoisture", 0.0, 100.0),
    ];
    for (sensor, (key, low, high)) in sensors.iter().zip(expected) {
        assert_eq!(sensor["key"], key);
        assert_eq!(sensor["range"][0].as_f64().unwrap(), low);
        assert_eq!(sensor["range"][1].as_f64().unwrap(), high);
        // Los rangos declarados no pueden exceder los límites que valida la API.
        let (api_low, api_high) = get_limits(key);
        assert!(
            low >= api_low && high <= api_high,
            "{key} is out of API range"
        );
    }

    let actuators: Vec<&str> = value["actuators"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a.as_str().unwrap())
        .collect();
    assert_eq!(
        actuators,
        [
            "WATER_PUMP",
            "UV_LIGHT",
            "FAN",
            "HUMIDIFIER",
            "NUTRIENT_DOSER",
            "PH_DOSER",
        ]
    );
}

#[tokio::test]
async fn test_commands_are_acknowledged_as_executed() {
    let env_arc = Arc::new(RwLock::new(Environment::new("LETTUCE", Some(1), 0.0)));
    let last_cmd_arc = Arc::new(RwLock::new(None));

    let payload =
        r#"{"id": "c1", "actuator": "WATER_PUMP", "action": "ACTIVATE", "durationSeconds": 15}"#;
    let ack = SimulatedDevice::process_command(payload, 0.0, &env_arc, &last_cmd_arc, CROP)
        .await
        .unwrap();

    assert_eq!(ack.id, "c1");
    assert_eq!(ack.status, "EXECUTED");
    assert_eq!(ack.message, "Water pump turned on for 15 s");
}

#[tokio::test]
async fn test_unsupported_actuators_fail_with_a_reason() {
    let env_arc = Arc::new(RwLock::new(Environment::new("LETTUCE", Some(1), 0.0)));
    let last_cmd_arc = Arc::new(RwLock::new(None));

    let payload = r#"{"id": "c2", "actuator": "HEATER", "action": "ACTIVATE"}"#;
    let ack = SimulatedDevice::process_command(payload, 0.0, &env_arc, &last_cmd_arc, CROP)
        .await
        .unwrap();

    assert_eq!(ack.id, "c2");
    assert_eq!(ack.status, "FAILED");
    assert!(ack.message.contains("HEATER"));
}

#[tokio::test]
async fn test_unreadable_commands_are_ignored() {
    let env_arc = Arc::new(RwLock::new(Environment::new("LETTUCE", Some(1), 0.0)));
    let last_cmd_arc = Arc::new(RwLock::new(None));

    assert!(
        SimulatedDevice::process_command("not valid json", 0.0, &env_arc, &last_cmd_arc, CROP)
            .await
            .is_none()
    );

    assert!(SimulatedDevice::process_command(
        r#"{"actuator": "FAN"}"#,
        0.0,
        &env_arc,
        &last_cmd_arc,
        CROP
    )
    .await
    .is_none());
}

#[test]
fn test_device_list_parsing() {
    let raw = format!("{CROP}:k1:lettuce, 66f5a1000000000000000102:k2:TOMATO");
    let devices = parse_devices(&raw, true).unwrap();
    assert_eq!(devices.len(), 2);
    assert_eq!(devices[0].crop_type, "LETTUCE");
    assert_eq!(devices[1].crop_type, "TOMATO");

    assert!(parse_devices("malformed", true).is_err());
    assert!(parse_devices("", true).is_err());
    assert!(parse_devices("", false).unwrap().is_empty());
}
