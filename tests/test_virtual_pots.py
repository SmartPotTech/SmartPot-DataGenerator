import pytest
import time
from fastapi.testclient import TestClient
from unittest.mock import MagicMock

from simulator.api import create_app
from simulator.config import DeviceConfig, Settings
from simulator.device import SimulatedDevice
from simulator.environment import Environment
from simulator.pots import Location, PotConfig, PotManager
from simulator.weather import WeatherClient, condition

TOKEN = "test-simulator-token-0123456789"
CROP = "66f5a1000000000000000201"
KEY = "device-key-for-tests-0001"
SETTINGS = Settings(host="localhost", port=1883, tls=False, ca_file=None, topic_prefix="smartpot/v1",
                    interval_seconds=30, devices=(DeviceConfig("66f5a1000000000000000101", "demo-key", "LETTUCE"),))
SUNNY = {"current": {"time": "2026-09-27T12:00", "temperature_2m": 27.5, "relative_humidity_2m": 40,
                     "is_day": 1, "precipitation": 0.0, "weather_code": 0, "cloud_cover": 5,
                     "surface_pressure": 854.3, "wind_speed_10m": 6.1, "shortwave_radiation": 820}}
RAINY = {"current": {**SUNNY["current"], "temperature_2m": 16.0, "relative_humidity_2m": 97, "is_day": 1,
                     "precipitation": 6.0, "weather_code": 63, "cloud_cover": 100, "shortwave_radiation": 40}}
PLACES = {"results": [{"name": "Medellín", "latitude": 6.245, "longitude": -75.57151, "country": "Colombia",
                       "admin1": "Antioquia"}]}


def fake_fetch(responses: dict):
    def fetch(url: str) -> dict:
        if "geocoding" in url:
            return PLACES
        return responses["weather"]

    return fetch


def factory(device: DeviceConfig, settings: Settings) -> SimulatedDevice:
    client = MagicMock()
    client.is_connected.return_value = True
    return SimulatedDevice(device, settings, client=client)


@pytest.fixture
def manager():
    responses = {"weather": SUNNY}
    pots = PotManager(SETTINGS, WeatherClient(fetch=fake_fetch(responses), ttl=0), factory=factory)
    pots.responses = responses
    pots.load_static()
    yield pots
    pots.stop()


@pytest.fixture
def client(manager):
    return TestClient(create_app(manager, TOKEN))


@pytest.fixture
def auth():
    return {"Authorization": f"Bearer {TOKEN}"}


def test_weather_codes_map_to_scenes():
    assert condition(0) == ("CLEAR", "Despejado")
    assert condition(63)[0] == "RAIN"
    assert condition(95)[0] == "STORM"
    assert condition(1234)[0] == "CLOUDY"


def test_weather_client_caches_by_location():
    calls = []

    def fetch(url):
        calls.append(url)
        return SUNNY

    weather = WeatherClient(fetch=fetch, ttl=600)
    first = weather.current(6.2442, -75.5812)
    weather.current(6.2449, -75.5807)
    assert len(calls) == 1
    assert first.condition == "CLEAR" and first.pressure == 854.3


def test_weather_mode_follows_the_sun_and_the_rain(manager):
    manager.upsert(PotConfig(CROP, KEY, "LETTUCE", mode="WEATHER",
                             location=Location("Medellín", 6.245, -75.5715)))
    environment = manager.get(CROP).device.environment
    for minute in range(1, 120):
        environment.step(60, 1_790_000_000 + minute * 60)
    assert environment.state["temperature"] > 26
    assert environment.state["brightness"] > 1500
    assert 850 < environment.state["atmosphere"] < 860
    dry = environment.state["soilMoisture"]

    manager.responses["weather"] = RAINY
    manager.upsert(PotConfig(CROP, KEY, "LETTUCE", mode="WEATHER", location=Location("Guatapé", 6.23, -75.16)))
    for minute in range(1, 60):
        environment.step(60, 1_790_010_000 + minute * 60)
    assert environment.state["soilMoisture"] > dry
    assert environment.state["brightness"] < 200


def test_manual_gauges_change_the_reading_and_actuators_still_act():
    environment = Environment("LETTUCE", seed=1)
    environment.set_mode("MANUAL")
    environment.set_manual({"soilMoisture": 35, "temperature": 30})
    assert environment.state["soilMoisture"] == 35
    environment.apply("WATER_PUMP", "ACTIVATE", 15, now=100)
    environment.step(60, 160)
    assert environment.state["soilMoisture"] > 45
    for minute in range(1, 60):
        environment.step(60, 160 + minute * 60)
    assert abs(environment.state["temperature"] - 30) < 1


def test_repeating_the_configuration_keeps_the_effect_of_actuators(manager):
    config = PotConfig(CROP, KEY, "LETTUCE", mode="MANUAL", manual={"soilMoisture": 40.0})
    manager.upsert(config)
    environment = manager.get(CROP).device.environment
    environment.state["soilMoisture"] = 70
    manager.upsert(PotConfig(CROP, KEY, "LETTUCE", mode="MANUAL", manual={"soilMoisture": 40.0}))
    assert environment.state["soilMoisture"] == 70


def test_control_api_requires_the_token(client):
    assert client.get("/health").json()["pots"] == 1
    assert client.get("/v1/pots").status_code == 401
    assert client.get("/v1/pots", headers={"Authorization": "Bearer otro"}).status_code == 401


def test_control_api_creates_updates_and_removes_a_pot(client, auth, manager):
    body = {"key": KEY, "cropType": "LETTUCE", "mode": "WEATHER",
            "location": {"name": "Medellín", "latitude": 6.245, "longitude": -75.5715}, "intervalSeconds": 15}
    created = client.put(f"/v1/pots/{CROP}", json=body, headers=auth)
    assert created.status_code == 200
    state = created.json()
    assert state["mode"] == "WEATHER" and state["managed"]
    assert state["weather"]["condition"] == "CLEAR"
    assert state["weather"]["label"] == "Despejado"

    manual = client.put(f"/v1/pots/{CROP}", headers=auth, json={**body, "mode": "MANUAL",
                                                                "manual": {"soilMoisture": 30, "ph": 7.2}})
    assert manual.json()["manual"]["soilMoisture"] == 30

    manager.tick(time.time() + 60)
    reading = client.get(f"/v1/pots/{CROP}", headers=auth).json()["lastReading"]
    assert 25 < reading["soilMoisture"] < 35

    assert client.delete(f"/v1/pots/{CROP}", headers=auth).status_code == 204
    assert client.get(f"/v1/pots/{CROP}", headers=auth).status_code == 404


def test_weather_mode_needs_a_location_and_static_pots_are_protected(client, auth):
    no_place = client.put(f"/v1/pots/{CROP}", headers=auth, json={"key": KEY, "cropType": "LETTUCE",
                                                                  "mode": "WEATHER"})
    assert no_place.status_code == 400
    static = client.delete("/v1/pots/66f5a1000000000000000101", headers=auth)
    assert static.status_code == 409


def test_places_search_and_weather_preview(client, auth):
    places = client.get("/v1/places", params={"q": "Medellín"}, headers=auth).json()
    assert places[0]["region"] == "Antioquia"
    weather = client.get("/v1/weather", params={"latitude": 6.24, "longitude": -75.57}, headers=auth).json()
    assert weather["isDay"] is True
    assert weather["cloudCover"] == 5


def test_without_token_the_control_api_is_disabled(manager):
    disabled = TestClient(create_app(manager, None))
    assert disabled.get("/health").json()["api"] == "DISABLED"
    assert disabled.get("/v1/pots").status_code == 503


def test_an_executed_command_brings_the_next_reading_forward(manager):
    manager.upsert(PotConfig(CROP, KEY, "LETTUCE", mode="AUTO", interval_seconds=300))
    pot = manager.get(CROP)
    pot.next_at = time.time() + 300
    ack = pot.device.handle_command('{"id": "c1", "actuator": "FAN", "action": "ACTIVATE", "durationSeconds": 60}',
                                    time.time())
    assert ack["status"] == "EXECUTED"
    assert pot.next_at <= time.time() + 2.5


def test_the_placement_reaches_the_simulated_environment(client, auth, manager):
    response = client.put(f"/v1/pots/{CROP}", headers=auth, json={
        "key": KEY, "cropType": "TOMATO", "mode": "AUTO", "setting": "INDOOR", "exposure": "SHADE"})
    assert response.status_code == 200
    assert response.json()["setting"] == "INDOOR" and response.json()["exposure"] == "SHADE"
    environment = manager.get(CROP).device.environment
    assert (environment.setting, environment.exposure) == ("INDOOR", "SHADE")
    wrong = client.put(f"/v1/pots/{CROP}", headers=auth, json={"key": KEY, "cropType": "TOMATO", "setting": "ROOF"})
    assert wrong.status_code == 422
