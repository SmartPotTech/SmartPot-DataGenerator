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
}

impl DeviceTopics {
    pub fn new(prefix: &str, crop_id: &str) -> Self {
        let base = format!("{prefix}/{crop_id}");
        Self {
            telemetry: format!("{base}/telemetry"),
            commands: format!("{base}/commands"),
            ack: format!("{base}/commands/ack"),
            status: format!("{base}/status"),
        }
    }
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
            let mut root_cert_store = rumqttc::tokio_rustls::rustls::RootCertStore::empty();
            if let Some(ref ca_file) = settings.ca_file {
                if let Ok(file) = std::fs::File::open(ca_file) {
                    let mut reader = std::io::BufReader::new(file);
                    let certs = rustls_pemfile::certs(&mut reader).flatten();
                    for cert in certs {
                        let _ = root_cert_store.add(cert);
                    }
                }
            } else {
                for cert in rustls_native_certs::load_native_certs().unwrap_or_default() {
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
                            info!("Cultivo {crop_id} ({crop_type}) conectado");

                            if let Err(e) = client
                                .publish(&topics.status, QoS::AtLeastOnce, true, "online")
                                .await
                            {
                                warn!("Error publicando status online para {crop_id}: {e}");
                            }

                            if let Err(e) =
                                client.subscribe(&topics.commands, QoS::AtLeastOnce).await
                            {
                                warn!("Error suscribiendo a commands para {crop_id}: {e}");
                            }
                        } else {
                            warn!(
                                "Cultivo {crop_id} rechazado por el broker: {:?}",
                                connack.code
                            );
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
                                        warn!("Error publicando ACK para {crop_id}: {e}");
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
                        warn!("Error en event_loop MQTT para {crop_id}: {e}");
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
                    "Comando ilegible descartado: {}",
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
                info!("Cultivo {crop_id} ejecutó {msg}");
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
                            "Error publicando telemetría binaria para {}: {e}",
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
                            "Error publicando telemetría para {}: {e}",
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

/// Codifica lecturas en un paquete binario compacto de 22 bytes con CRC-8.
pub fn encode_compact_binary(reading: &HashMap<String, f64>, seq_id: u16, now: f64) -> [u8; 22] {
    let mut out = [0u8; 22];
    out[0] = 0x50; // 'P'
    out[1] = 0x01; // Version 1
    out[2..4].copy_from_slice(&seq_id.to_be_bytes());
    out[4..8].copy_from_slice(&(now as u32).to_be_bytes());

    let temp = (reading.get("temperature").copied().unwrap_or(0.0) * 100.0) as i16;
    out[8..10].copy_from_slice(&temp.to_be_bytes());

    let hum = (reading
        .get("humidity")
        .copied()
        .unwrap_or(0.0)
        .clamp(0.0, 100.0)
        * 100.0) as u16;
    out[10..12].copy_from_slice(&hum.to_be_bytes());

    let ph = (reading.get("ph").copied().unwrap_or(0.0).clamp(0.0, 14.0) * 100.0) as u16;
    out[12..14].copy_from_slice(&ph.to_be_bytes());

    let tds = reading
        .get("tds")
        .copied()
        .unwrap_or(0.0)
        .clamp(0.0, 65535.0) as u16;
    out[14..16].copy_from_slice(&tds.to_be_bytes());

    let moist = (reading
        .get("soilMoisture")
        .or_else(|| reading.get("soil_moisture"))
        .copied()
        .unwrap_or(0.0)
        .clamp(0.0, 100.0)
        * 100.0) as u16;
    out[16..18].copy_from_slice(&moist.to_be_bytes());

    let bright = reading
        .get("brightness")
        .copied()
        .unwrap_or(0.0)
        .clamp(0.0, 65535.0) as u16;
    out[18..20].copy_from_slice(&bright.to_be_bytes());

    out[20] = 0x00; // flags
    out[21] = calc_crc8(&out[..21]);

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encode_compact_binary_roundtrip() {
        let mut r = HashMap::new();
        r.insert("temperature".to_string(), 24.5);
        r.insert("humidity".to_string(), 60.0);
        r.insert("ph".to_string(), 6.5);
        r.insert("tds".to_string(), 500.0);
        r.insert("soilMoisture".to_string(), 45.0);
        r.insert("brightness".to_string(), 12000.0);

        let packet = encode_compact_binary(&r, 10, 1700000000.0);
        assert_eq!(packet[0], 0x50);
        assert_eq!(packet[1], 0x01);
        assert_eq!(u16::from_be_bytes([packet[2], packet[3]]), 10);
        assert_eq!(calc_crc8(&packet[..21]), packet[21]);
    }
}
