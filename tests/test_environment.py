import pytest

from simulator.environment import LIMITS, Environment

NOON = 12 * 3600.0
MIDNIGHT = 24 * 3600.0


def advance(env: Environment, minutes: int, start: float = NOON, step: float = 30.0) -> float:
    now = start
    for _ in range(int(minutes * 60 / step)):
        now += step
        env.step(step, now)
    return now


def test_readings_stay_inside_the_sensor_scale():
    env = Environment("TOMATO", seed=1)
    advance(env, 600)
    for name, value in env.reading().items():
        low, high = LIMITS[name]
        assert low <= value <= high


def test_water_pump_raises_soil_moisture():
    env = Environment("LETTUCE", seed=1)
    env.state["soilMoisture"] = 30
    env.apply("WATER_PUMP", "ACTIVATE", 20, NOON)
    advance(env, 1)
    assert env.state["soilMoisture"] > 45


def test_fan_cools_the_air():
    with_fan, without_fan = Environment("BASIL", seed=1), Environment("BASIL", seed=1)
    with_fan.apply("FAN", "ACTIVATE", 3600, NOON)
    advance(with_fan, 60)
    advance(without_fan, 60)
    assert with_fan.state["temperature"] < without_fan.state["temperature"]


def test_uv_light_adds_light_at_night():
    env = Environment("LETTUCE", seed=1)
    advance(env, 60, start=MIDNIGHT)
    dark = env.state["brightness"]
    env.apply("UV_LIGHT", "ACTIVATE", 3600, MIDNIGHT + 3600)
    advance(env, 30, start=MIDNIGHT + 3600)
    assert env.state["brightness"] > dark + 300


def test_ph_doser_lowers_ph_and_deactivate_stops_actuators():
    env = Environment("TOMATO", seed=1)
    before = env.state["ph"]
    env.apply("PH_DOSER", "ACTIVATE", 3, NOON)
    assert env.state["ph"] == pytest.approx(before - 0.36)
    env.apply("FAN", "ACTIVATE", 600, NOON)
    env.apply("FAN", "DEACTIVATE", None, NOON + 1)
    assert not env.is_active("FAN", NOON + 2)


def test_unknown_actuators_are_rejected():
    with pytest.raises(ValueError, match="no existe"):
        Environment("LETTUCE").apply("HEATER", "ACTIVATE", 10, NOON)


def test_daylight_follows_the_local_time_zone():
    local, utc = Environment("TOMATO", seed=1, utc_offset_hours=-5), Environment("TOMATO", seed=1)
    five_pm_utc = 17 * 3600.0
    advance(local, 120, start=five_pm_utc)
    advance(utc, 120, start=five_pm_utc)
    assert local.state["brightness"] > utc.state["brightness"] + 300
