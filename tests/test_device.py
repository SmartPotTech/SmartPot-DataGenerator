import json
from unittest.mock import MagicMock

import pytest

from simulator.config import DeviceConfig, Settings, parse_devices
from simulator.device import SimulatedDevice

CROP = "66f5a1000000000000000101"
SETTINGS = Settings(host="localhost", port=1883, tls=False, ca_file=None, topic_prefix="smartpot/v1",
                    interval_seconds=30, devices=())


def device() -> SimulatedDevice:
    client = MagicMock()
    client.is_connected.return_value = True
    return SimulatedDevice(DeviceConfig(CROP, "clave", "LETTUCE"), SETTINGS, client=client)


def test_topics_follow_the_v1_contract():
    topics = device().topics
    assert topics == {
        "telemetry": f"smartpot/v1/{CROP}/telemetry",
        "commands": f"smartpot/v1/{CROP}/commands",
        "ack": f"smartpot/v1/{CROP}/commands/ack",
        "status": f"smartpot/v1/{CROP}/status",
    }


def test_commands_are_acknowledged_as_executed():
    ack = device().handle_command(json.dumps({"id": "c1", "actuator": "WATER_PUMP", "action": "ACTIVATE",
                                              "durationSeconds": 15}), now=0)
    assert ack == {"id": "c1", "status": "EXECUTED", "message": "WATER_PUMP encendido 15 s"}


def test_unsupported_actuators_fail_with_a_reason():
    ack = device().handle_command(json.dumps({"id": "c2", "actuator": "HEATER", "action": "ACTIVATE"}), now=0)
    assert ack["status"] == "FAILED"
    assert "HEATER" in ack["message"]


def test_unreadable_commands_are_ignored():
    assert device().handle_command("no es json", now=0) is None
    assert device().handle_command(json.dumps({"actuator": "FAN"}), now=0) is None


def test_telemetry_is_published_as_json():
    sim = device()
    reading = sim.publish_reading(now=sim.last_step + 30)
    topic, payload = sim.client.publish.call_args.args[:2]
    assert topic == f"smartpot/v1/{CROP}/telemetry"
    assert json.loads(payload) == reading
    assert set(reading) == {"temperature", "humidity", "brightness", "ph", "tds", "soilMoisture", "atmosphere"}


def test_device_list_parsing():
    devices = parse_devices(f"{CROP}:k1:lettuce, 66f5a1000000000000000102:k2:TOMATO")
    assert [d.crop_type for d in devices] == ["LETTUCE", "TOMATO"]
    with pytest.raises(ValueError):
        parse_devices("sin-formato")
    with pytest.raises(ValueError):
        parse_devices("")
