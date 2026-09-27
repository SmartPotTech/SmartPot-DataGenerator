"""Configuración desde variables de entorno."""

import os
from dataclasses import dataclass


@dataclass(frozen=True)
class DeviceConfig:
    crop_id: str
    key: str
    crop_type: str


@dataclass(frozen=True)
class Settings:
    host: str
    port: int
    tls: bool
    ca_file: str | None
    topic_prefix: str
    interval_seconds: float
    devices: tuple[DeviceConfig, ...]
    utc_offset_hours: float = -5.0
    api_token: str | None = None
    api_port: int = 8081


def parse_devices(raw: str, required: bool = True) -> tuple[DeviceConfig, ...]:
    """Formato: cropId:clave:TIPO separados por comas. Con la API de control activa la lista puede ir vacía."""
    devices = []
    for chunk in filter(None, (part.strip() for part in raw.split(","))):
        parts = chunk.split(":")
        if len(parts) != 3 or not all(parts):
            raise ValueError(f"Dispositivo mal definido: «{chunk}». Usa cropId:clave:TIPO")
        devices.append(DeviceConfig(crop_id=parts[0], key=parts[1], crop_type=parts[2].upper()))
    if not devices and required:
        raise ValueError("Define al menos un dispositivo en SIMULATOR_DEVICES o activa la API con SIMULATOR_TOKEN")
    return tuple(devices)


def load() -> Settings:
    env = os.environ
    tls = env.get("MQTT_TLS", "false").lower() == "true"
    token = env.get("SIMULATOR_TOKEN") or None
    if token is not None and len(token) < 24:
        raise ValueError("SIMULATOR_TOKEN debe tener al menos 24 caracteres")
    return Settings(
        host=env.get("MQTT_HOST", "localhost"),
        port=int(env.get("MQTT_PORT", "8883" if tls else "1883")),
        tls=tls,
        ca_file=env.get("MQTT_CA_FILE") or None,
        topic_prefix=env.get("MQTT_TOPIC_PREFIX", "smartpot/v1"),
        interval_seconds=float(env.get("SIMULATOR_INTERVAL_SECONDS", "30")),
        devices=parse_devices(env.get("SIMULATOR_DEVICES", ""), required=token is None),
        utc_offset_hours=float(env.get("SIMULATOR_UTC_OFFSET", "-5")),
        api_token=token,
        api_port=int(env.get("SIMULATOR_PORT", "8081")),
    )
