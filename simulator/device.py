"""Dispositivo simulado que habla el contrato MQTT v1 de SmartPot."""

import json
import logging
import ssl
import time
from collections.abc import Callable

import paho.mqtt.client as mqtt

from simulator.config import DeviceConfig, Settings
from simulator.environment import Environment

log = logging.getLogger(__name__)


class SimulatedDevice:
    def __init__(self, device: DeviceConfig, settings: Settings, client: mqtt.Client | None = None):
        self.device = device
        self.settings = settings
        self.environment = Environment(device.crop_type, utc_offset_hours=settings.utc_offset_hours)
        base = f"{settings.topic_prefix}/{device.crop_id}"
        self.topics = {
            "telemetry": f"{base}/telemetry",
            "commands": f"{base}/commands",
            "ack": f"{base}/commands/ack",
            "status": f"{base}/status",
        }
        self.client = client or self._build_client()
        self.last_step = time.time()
        self.last_reading: dict[str, float] | None = None
        self.last_published_at: float | None = None
        self.last_command: dict | None = None
        # Aviso de que un comando se ejecutó: el administrador adelanta la próxima lectura.
        self.on_executed: Callable[[], None] | None = None

    def _build_client(self) -> mqtt.Client:
        client = mqtt.Client(mqtt.CallbackAPIVersion.VERSION2, client_id=f"smartpot-sim-{self.device.crop_id}",
                             protocol=mqtt.MQTTv311)
        client.username_pw_set(self.device.crop_id, self.device.key)
        client.will_set(self.topics["status"], "offline", qos=1, retain=True)
        if self.settings.tls:
            context = ssl.create_default_context(cafile=self.settings.ca_file)
            client.tls_set_context(context)
        client.on_connect = self._on_connect
        client.on_message = self._on_message
        client.reconnect_delay_set(min_delay=1, max_delay=30)
        return client

    def start(self) -> None:
        self.client.connect_async(self.settings.host, self.settings.port, keepalive=60)
        self.client.loop_start()

    def stop(self) -> None:
        self.client.publish(self.topics["status"], "offline", qos=1, retain=True)
        self.client.loop_stop()
        self.client.disconnect()

    def _on_connect(self, client, userdata, flags, reason_code, properties=None) -> None:
        if reason_code.is_failure:
            log.warning("Cultivo %s rechazado por el broker: %s", self.device.crop_id, reason_code)
            return
        client.publish(self.topics["status"], "online", qos=1, retain=True)
        client.subscribe(self.topics["commands"], qos=1)
        log.info("Cultivo %s (%s) conectado", self.device.crop_id, self.device.crop_type)

    def _on_message(self, client, userdata, message) -> None:
        ack = self.handle_command(message.payload.decode("utf-8", errors="replace"), time.time())
        if ack:
            client.publish(self.topics["ack"], json.dumps(ack), qos=1)

    def handle_command(self, payload: str, now: float) -> dict | None:
        try:
            command = json.loads(payload)
            command_id = str(command["id"])
        except (ValueError, KeyError, TypeError):
            log.warning("Comando ilegible descartado: %s", payload[:120])
            return None
        try:
            message = self.environment.apply(str(command.get("actuator", "")).upper(),
                                             str(command.get("action", "")).upper(),
                                             command.get("durationSeconds"), now)
            log.info("Cultivo %s ejecutó %s", self.device.crop_id, message)
            ack = {"id": command_id, "status": "EXECUTED", "message": message}
        except ValueError as error:
            ack = {"id": command_id, "status": "FAILED", "message": str(error)}
        self.last_command = {**ack, "at": now}
        if ack["status"] == "EXECUTED" and self.on_executed:
            self.on_executed()
        return ack

    @property
    def connected(self) -> bool:
        return self.client.is_connected()

    def publish_reading(self, now: float) -> dict[str, float]:
        # Tras una pausa larga la física avanza como máximo 15 minutos de golpe.
        self.environment.step(min(now - self.last_step, 900.0), now)
        self.last_step = now
        reading = self.environment.reading()
        if self.client.is_connected():
            self.client.publish(self.topics["telemetry"], json.dumps(reading), qos=0)
            self.last_published_at = now
        self.last_reading = reading
        return reading
