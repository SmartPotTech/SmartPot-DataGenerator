//! Dispositivo simulado que habla el contrato MQTT v1 de SmartPot.

use chrono::Utc;
use rumqttc::{AsyncClient, Event, LastWill, MqttOptions, Packet, QoS, Transport};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, RwLock};
use tokio::task::JoinHandle;
use tracing::{info, warn};

use crate::config::{DeviceConfig, Settings};
use crate::domain::actuator::{Action, Actuator};
use crate::domain::environment::Environment;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandPayload {
    pub id: serde_json::Value,
    pub actuator: Option<String>,
    pub action: Option<String>,
    #[serde(rename = "durationSeconds")]
    pub duration_seconds: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CommandAck {
    pub id: String,
    pub status: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandRecord {
    pub id: String,
    pub status: String,
    pub message: String,
    pub at: f64,
}

#[derive(Debug, Clone)]
pub struct DeviceTopics {
    pub telemetry: String,
    pub commands: String,
    pub ack: String,
    pub status: String,
    pub capabilities: String,
}

impl DeviceTopics {
    pub fn new(prefix: &str, crop_id: &str) -> Self {
        let base = format!("{prefix}/{crop_id}");
        Self {
            telemetry: format!("{base}/telemetry"),
            commands: format!("{base}/commands"),
            ack: format!("{base}/commands/ack"),
            status: format!("{base}/status"),
            capabilities: format!("{base}/capabilities"),
        }
    }
}

/// Publicación retenida de capacidades: tópico, QoS, retención y carga.
/// Se separa del bucle MQTT para poder validar el contrato sin un broker real.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilitiesPublication {
    pub topic: String,
    pub qos: QoS,
    pub retain: bool,
    pub payload: String,
}

pub fn capabilities_publication(topics: &DeviceTopics) -> CapabilitiesPublication {
    CapabilitiesPublication {
        topic: topics.capabilities.clone(),
        qos: QoS::AtLeastOnce,
        retain: true,
        payload: capabilities_payload(),
    }
}

/// Carga retenida de capacidades declaradas por el dispositivo virtual (Audit_06, esquema 1).
/// Declara los mismos siete sensores y seis actuadores que implementa el simulador.
pub fn capabilities_payload() -> String {
    serde_json::json!({
        "schema": 1,
        "device": {
            "firmware": "smartpot-datagenerator/2.0.0",
            "hardware": "virtual",
            "telemetry": {
                "formats": ["json-v1", "compact-v2"],
                "compactBytes": COMPACT_V2_SIZE,
            },
        },
        "sensors": [
            {"key": "temperature", "unit": "C", "resolution": 0.01, "range": [-20.0, 60.0]},
            {"key": "humidity", "unit": "%", "resolution": 0.01, "range": [0.0, 100.0]},
            {"key": "atmosphere", "unit": "hPa", "resolution": 0.1, "range": [300.0, 1100.0]},
            {"key": "brightness", "unit": "lux", "resolution": 1.0, "range": [0.0, 2000.0]},
            {"key": "ph", "unit": "pH", "resolution": 0.01, "range": [0.0, 14.0]},
            {"key": "tds", "unit": "ppm", "resolution": 1.0, "range": [0.0, 3000.0]},
            {"key": "soilMoisture", "unit": "%", "resolution": 0.01, "range": [0.0, 100.0]},
        ],
        "actuators": [
            "WATER_PUMP",
            "UV_LIGHT",
            "FAN",
            "HUMIDIFIER",
            "NUTRIENT_DOSER",
            "PH_DOSER",
        ],
    })
    .to_string()
}

pub struct SimulatedDevice {
    pub device: DeviceConfig,
    pub topics: DeviceTopics,
    pub environment: Arc<RwLock<Environment>>,
    pub client: Option<AsyncClient>,
    pub connected: Arc<AtomicBool>,
    pub last_step: Arc<RwLock<f64>>,
    pub last_reading: Arc<RwLock<Option<HashMap<String, f64>>>>,
    pub last_published_at: Arc<RwLock<Option<f64>>>,
    pub last_command: Arc<RwLock<Option<CommandRecord>>>,
    pub on_executed_tx: Option<mpsc::Sender<()>>,
    pub payload_format: String,
    pub seq_counter: Arc<std::sync::atomic::AtomicU32>,
    event_loop_task: Option<JoinHandle<()>>,
}

impl SimulatedDevice {
    pub fn new(
        device: DeviceConfig,
        settings: &Settings,
        on_executed_tx: Option<mpsc::Sender<()>>,
    ) -> Self {
        let topics = DeviceTopics::new(&settings.topic_prefix, &device.crop_id);
        let environment = Arc::new(RwLock::new(Environment::new(
            &device.crop_type,
            None,
            settings.utc_offset_hours,
        )));

        Self {
            device,
            topics,
            environment,
            client: None,
            connected: Arc::new(AtomicBool::new(false)),
            last_step: Arc::new(RwLock::new(Utc::now().timestamp() as f64)),
            last_reading: Arc::new(RwLock::new(None)),
            last_published_at: Arc::new(RwLock::new(None)),
            last_command: Arc::new(RwLock::new(None)),
            on_executed_tx,
            payload_format: settings.payload_format.clone(),
            seq_counter: Arc::new(std::sync::atomic::AtomicU32::new(1)),
            event_loop_task: None,
        }
    }

    pub fn start(&mut self, settings: &Settings) {
        let client_id = format!("smartpot-sim-{}", self.device.crop_id);
        let mut mqttoptions = MqttOptions::new(&client_id, &settings.host, settings.port);
        mqttoptions.set_credentials(&self.device.crop_id, &self.device.key);
        mqttoptions.set_keep_alive(Duration::from_secs(60));
        mqttoptions.set_clean_session(true);

        mqttoptions.set_last_will(LastWill::new(
            &self.topics.status,
            "offline",
            QoS::AtLeastOnce,
            true,
        ));

        if settings.tls {
            // La verificación la hace rustls 0.23; el PEM se lee con pki-types (rustls-pemfile ya no se mantiene).
            use rumqttc::tokio_rustls::rustls::pki_types::{pem::PemObject, CertificateDer};
            let mut root_cert_store = rumqttc::tokio_rustls::rustls::RootCertStore::empty();
            if let Some(ref ca_file) = settings.ca_file {
                match CertificateDer::pem_file_iter(ca_file) {
                    Ok(certs) => {
                        for cert in certs.flatten() {
                            let _ = root_cert_store.add(cert);
                        }
                    }
                    Err(error) => warn!("Could not read CA {}: {}", ca_file, error),
                }
            } else {
                for cert in rustls_native_certs::load_native_certs().certs {
                    let _ = root_cert_store.add(cert);
                }
            }

            let client_config = rumqttc::tokio_rustls::rustls::ClientConfig::builder()
                .with_root_certificates(root_cert_store)
                .with_no_client_auth();

            mqttoptions.set_transport(Transport::tls_with_config(client_config.into()));
        }

        let (client, mut event_loop) = AsyncClient::new(mqttoptions, 64);
        self.client = Some(client.clone());

        let connected = Arc::clone(&self.connected);
        let topics = self.topics.clone();
        let crop_id = self.device.crop_id.clone();
        let crop_type = self.device.crop_type.clone();
        let env_arc = Arc::clone(&self.environment);
        let last_cmd_arc = Arc::clone(&self.last_command);
        let on_executed = self.on_executed_tx.clone();

        let task = tokio::spawn(async move {
            loop {
                match event_loop.poll().await {
                    Ok(Event::Incoming(Packet::ConnAck(connack))) => {
                        if connack.code == rumqttc::ConnectReturnCode::Success {
                            connected.store(true, Ordering::SeqCst);
                            info!("Crop {crop_id} ({crop_type}) connected");

                            if let Err(e) = client
                                .publish(&topics.status, QoS::AtLeastOnce, true, "online")
                                .await
                            {
                                warn!("Error publishing online status for {crop_id}: {e}");
                            }

                            // Las capacidades se publican retenidas después del estado
                            // online y antes de escuchar comandos, para que la API conozca
                            // el hardware antes de la primera orden. Un fallo aquí no debe
                            // impedir la suscripción: el contrato v1 sigue funcionando.
                            let publication = capabilities_publication(&topics);
                            match client
                                .publish(
                                    &publication.topic,
                                    publication.qos,
                                    publication.retain,
                                    publication.payload,
                                )
                                .await
                            {
                                Ok(()) => info!("Crop {crop_id} published its capabilities"),
                                Err(e) => {
                                    warn!("Error publishing capabilities for {crop_id}: {e}")
                                }
                            }

                            if let Err(e) =
                                client.subscribe(&topics.commands, QoS::AtLeastOnce).await
                            {
                                warn!("Error subscribing to commands for {crop_id}: {e}");
                            }
                        } else {
                            warn!("Crop {crop_id} rejected by broker: {:?}", connack.code);
                        }
                    }
                    Ok(Event::Incoming(Packet::Publish(publish))) => {
                        if publish.topic == topics.commands {
                            let now = Utc::now().timestamp() as f64;
                            let payload = String::from_utf8_lossy(&publish.payload);
                            let ack = Self::process_command(
                                &payload,
                                now,
                                &env_arc,
                                &last_cmd_arc,
                                &crop_id,
                            )
                            .await;

                            if let Some(ack) = ack {
                                if let Ok(ack_json) = serde_json::to_string(&ack) {
                                    if let Err(e) = client
                                        .publish(&topics.ack, QoS::AtLeastOnce, false, ack_json)
                                        .await
                                    {
                                        warn!("Error publishing ACK for {crop_id}: {e}");
                                    }
                                }

                                if ack.status == "EXECUTED" {
                                    if let Some(ref tx) = on_executed {
                                        let _ = tx.send(()).await;
                                    }
                                }
                            }
                        }
                    }
                    Ok(_) => {}
                    Err(e) => {
                        connected.store(false, Ordering::SeqCst);
                        warn!("MQTT event_loop error for {crop_id}: {e}");
                        tokio::time::sleep(Duration::from_secs(2)).await;
                    }
                }
            }
        });

        self.event_loop_task = Some(task);
    }

    pub async fn process_command(
        payload: &str,
        now: f64,
        env_arc: &Arc<RwLock<Environment>>,
        last_cmd_arc: &Arc<RwLock<Option<CommandRecord>>>,
        crop_id: &str,
    ) -> Option<CommandAck> {
        let cmd: CommandPayload = match serde_json::from_str(payload) {
            Ok(c) => c,
            Err(_) => {
                warn!(
                    "Unreadable command discarded: {}",
                    &payload[..payload.len().min(120)]
                );
                return None;
            }
        };

        let cmd_id = match &cmd.id {
            serde_json::Value::String(s) => s.clone(),
            serde_json::Value::Number(n) => n.to_string(),
            other => other.to_string(),
        };

        let actuator_str = match &cmd.actuator {
            Some(a) => a,
            None => return None,
        };

        let action_str = match &cmd.action {
            Some(a) => a,
            None => "ACTIVATE",
        };

        let actuator = match actuator_str.parse::<Actuator>() {
            Ok(a) => a,
            Err(err) => {
                let ack = CommandAck {
                    id: cmd_id.clone(),
                    status: "FAILED".to_string(),
                    message: err,
                };
                let mut last = last_cmd_arc.write().await;
                *last = Some(CommandRecord {
                    id: ack.id.clone(),
                    status: ack.status.clone(),
                    message: ack.message.clone(),
                    at: now,
                });
                return Some(ack);
            }
        };

        let action = match action_str.parse::<Action>() {
            Ok(a) => a,
            Err(err) => {
                let ack = CommandAck {
                    id: cmd_id.clone(),
                    status: "FAILED".to_string(),
                    message: err,
                };
                let mut last = last_cmd_arc.write().await;
                *last = Some(CommandRecord {
                    id: ack.id.clone(),
                    status: ack.status.clone(),
                    message: ack.message.clone(),
                    at: now,
                });
                return Some(ack);
            }
        };

        let mut env = env_arc.write().await;
        let ack = match env.apply(actuator, action, cmd.duration_seconds, now) {
            Ok(msg) => {
                info!("Crop {crop_id} executed {msg}");
                CommandAck {
                    id: cmd_id,
                    status: "EXECUTED".to_string(),
                    message: msg,
                }
            }
            Err(err) => CommandAck {
                id: cmd_id,
                status: "FAILED".to_string(),
                message: err,
            },
        };

        let mut last = last_cmd_arc.write().await;
        *last = Some(CommandRecord {
            id: ack.id.clone(),
            status: ack.status.clone(),
            message: ack.message.clone(),
            at: now,
        });

        Some(ack)
    }

    pub async fn stop(&mut self) {
        if let Some(task) = self.event_loop_task.take() {
            task.abort();
        }
        if let Some(ref client) = self.client {
            let _ = client
                .publish(&self.topics.status, QoS::AtLeastOnce, true, "offline")
                .await;
            let _ = client.disconnect().await;
        }
        self.connected.store(false, Ordering::SeqCst);
    }

    pub fn is_connected(&self) -> bool {
        self.connected.load(Ordering::SeqCst)
    }

    pub async fn publish_reading(&self, now: f64) -> HashMap<String, f64> {
        let mut last_step = self.last_step.write().await;
        let delta = (now - *last_step).min(900.0);
        *last_step = now;

        let reading = {
            let mut env = self.environment.write().await;
            env.step(delta, now);
            env.reading()
        };

        if self.is_connected() {
            if let Some(ref client) = self.client {
                if self.payload_format == "compact_binary" || self.payload_format == "binary" {
                    let seq = self.seq_counter.fetch_add(1, Ordering::Relaxed) as u16;
                    let packet = encode_compact_binary(&reading, seq, now);
                    if let Err(e) = client
                        .publish(
                            &self.topics.telemetry,
                            QoS::AtMostOnce,
                            false,
                            packet.to_vec(),
                        )
                        .await
                    {
                        warn!(
                            "Error publishing binary telemetry for {}: {e}",
                            self.device.crop_id
                        );
                    } else {
                        let mut pub_at = self.last_published_at.write().await;
                        *pub_at = Some(now);
                    }
                } else if let Ok(payload) = serde_json::to_string(&reading) {
                    if let Err(e) = client
                        .publish(&self.topics.telemetry, QoS::AtMostOnce, false, payload)
                        .await
                    {
                        warn!(
                            "Error publishing telemetry for {}: {e}",
                            self.device.crop_id
                        );
                    } else {
                        let mut pub_at = self.last_published_at.write().await;
                        *pub_at = Some(now);
                    }
                }
            }
        }

        let mut last_r = self.last_reading.write().await;
        *last_r = Some(reading.clone());

        reading
    }
}

/// Calcula el checksum CRC-8 ATM (polinomio 0x07) sobre el buffer dado.
pub fn calc_crc8(data: &[u8]) -> u8 {
    let mut crc: u8 = 0x00;
    for &b in data {
        crc ^= b;
        for _ in 0..8 {
            if (crc & 0x80) != 0 {
                crc = (crc << 1) ^ 0x07;
            } else {
                crc <<= 1;
            }
        }
    }
    crc
}

/// Tamaño del paquete compacto v2 (ver el formato en `encode_compact_binary`).
pub const COMPACT_V2_SIZE: usize = 27;

/// Codifica una lectura en el paquete compacto v2 (27 bytes, big-endian), la alternativa binaria al JSON v1:
/// `[0]` 'P' · `[1]` versión 2 · `[2-3]` secuencia · `[4-7]` época en s · `[8-9]` máscara de presencia (bit 0
/// temperatura, 1 humedad, 2 luz, 3 pH, 4 TDS, 5 presión, 6 sustrato) · `[10-11]` temperatura i16 ×100 ·
/// `[12-13]` humedad ×100 · `[14-17]` luz u32 lux · `[18-19]` pH ×100 · `[20-21]` TDS · `[22-23]` presión ×10 hPa ·
/// `[24-25]` sustrato ×100 · `[26]` CRC-8 (0x07). Un valor ausente deja su bit en 0; un 0 real lleva su bit.
pub fn encode_compact_binary(
    reading: &HashMap<String, f64>,
    seq_id: u16,
    now: f64,
) -> [u8; COMPACT_V2_SIZE] {
    let mut out = [0u8; COMPACT_V2_SIZE];
    out[0] = 0x50;
    out[1] = 0x02;
    out[2..4].copy_from_slice(&seq_id.to_be_bytes());
    out[4..8].copy_from_slice(&(now.max(0.0) as u32).to_be_bytes());

    let mut mask: u16 = 0;
    let mut field = |key: &str, bit: u16| -> Option<f64> {
        let value = reading.get(key).copied().filter(|v| v.is_finite())?;
        mask |= 1 << bit;
        Some(value)
    };
    if let Some(v) = field("temperature", 0) {
        out[10..12].copy_from_slice(
            &((v * 100.0).round().clamp(i16::MIN as f64, i16::MAX as f64) as i16).to_be_bytes(),
        );
    }
    if let Some(v) = field("humidity", 1) {
        out[12..14].copy_from_slice(&((v.clamp(0.0, 100.0) * 100.0).round() as u16).to_be_bytes());
    }
    if let Some(v) = field("brightness", 2) {
        out[14..18].copy_from_slice(&(v.clamp(0.0, u32::MAX as f64).round() as u32).to_be_bytes());
    }
    if let Some(v) = field("ph", 3) {
        out[18..20].copy_from_slice(&((v.clamp(0.0, 14.0) * 100.0).round() as u16).to_be_bytes());
    }
    if let Some(v) = field("tds", 4) {
        out[20..22].copy_from_slice(&(v.clamp(0.0, u16::MAX as f64).round() as u16).to_be_bytes());
    }
    if let Some(v) = field("atmosphere", 5) {
        out[22..24].copy_from_slice(&((v.clamp(0.0, 6553.5) * 10.0).round() as u16).to_be_bytes());
    }
    if let Some(v) = field("soilMoisture", 6) {
        out[24..26].copy_from_slice(&((v.clamp(0.0, 100.0) * 100.0).round() as u16).to_be_bytes());
    }
    out[8..10].copy_from_slice(&mask.to_be_bytes());
    out[COMPACT_V2_SIZE - 1] = calc_crc8(&out[..COMPACT_V2_SIZE - 1]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encode_compact_binary_v2_marks_present_sensors() {
        let mut r = HashMap::new();
        r.insert("temperature".to_string(), 0.0);
        r.insert("brightness".to_string(), 98_000.0);
        r.insert("atmosphere".to_string(), 1013.2);

        let packet = encode_compact_binary(&r, 10, 1_700_000_000.0);
        assert_eq!(packet.len(), COMPACT_V2_SIZE);
        assert_eq!(packet[0], 0x50);
        assert_eq!(packet[1], 0x02);
        assert_eq!(u16::from_be_bytes([packet[2], packet[3]]), 10);
        // temperatura (bit 0), luz (bit 2) y presión (bit 5); el resto ausente
        assert_eq!(u16::from_be_bytes([packet[8], packet[9]]), 0b0010_0101);
        assert_eq!(i16::from_be_bytes([packet[10], packet[11]]), 0);
        assert_eq!(
            u32::from_be_bytes([packet[14], packet[15], packet[16], packet[17]]),
            98_000
        );
        assert_eq!(u16::from_be_bytes([packet[22], packet[23]]), 10_132);
        assert_eq!(
            calc_crc8(&packet[..COMPACT_V2_SIZE - 1]),
            packet[COMPACT_V2_SIZE - 1]
        );
    }
}
