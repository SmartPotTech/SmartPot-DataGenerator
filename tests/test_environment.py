import pytest

from simulator.environment import LIMITS, Environment
from simulator.weather import Weather

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


def test_an_actuator_without_duration_stays_on_until_turned_off():
    env = Environment("BASIL", seed=1)
    assert env.apply("FAN", "ACTIVATE", None, NOON) == "Ventilador encendido"
    assert env.is_active("FAN", NOON + 5 * 3600)
    assert env.active_actuators(NOON + 5 * 3600) == {"FAN": None}
    assert env.active_fraction("FAN", NOON + 3600, NOON + 3660) == 1.0
    env.apply("FAN", "DEACTIVATE", None, NOON + 6 * 3600)
    assert not env.is_active("FAN", NOON + 6 * 3600 + 1)


def test_acknowledgements_read_durations_like_a_person():
    env = Environment("TOMATO", seed=1)
    assert env.apply("WATER_PUMP", "ACTIVATE", 15, NOON) == "Bomba de agua encendida por 15 s"
    assert env.apply("FAN", "ACTIVATE", 600, NOON) == "Ventilador encendido por 10 min"
    assert env.apply("UV_LIGHT", "ACTIVATE", 7200, NOON) == "Luz ultravioleta encendida por 2 h"


def test_a_doser_without_duration_releases_one_dose():
    env = Environment("TOMATO", seed=1)
    before = env.state["ph"]
    assert env.apply("PH_DOSER", "ACTIVATE", None, NOON) == "Dosificador de pH encendido por 3 s"
    assert env.state["ph"] == pytest.approx(before - 0.36)
    assert not env.is_active("PH_DOSER", NOON + 4)


def test_unknown_actuators_are_rejected():
    with pytest.raises(ValueError, match="no existe"):
        Environment("LETTUCE").apply("HEATER", "ACTIVATE", 10, NOON)


def test_daylight_follows_the_local_time_zone():
    local, utc = Environment("TOMATO", seed=1, utc_offset_hours=-5), Environment("TOMATO", seed=1)
    five_pm_utc = 17 * 3600.0
    advance(local, 120, start=five_pm_utc)
    advance(utc, 120, start=five_pm_utc)
    assert local.state["brightness"] > utc.state["brightness"] + 300


@pytest.mark.parametrize("mode", ["AUTO", "MANUAL", "WEATHER"])
def test_every_mode_feels_the_fan_and_the_humidifier(mode):
    def run(actuator: str | None) -> Environment:
        env = Environment("TOMATO", seed=1)
        env.set_mode(mode)
        if mode == "WEATHER":
            env.weather = Weather(24, 60, 20, 500, 0, 850, 5, True, 1, "MOSTLY_CLEAR", "Mayormente despejado",
                                  "", 0)
        advance(env, 30)
        if actuator:
            env.apply(actuator, "ACTIVATE", 600, NOON + 1800)
        advance(env, 10, start=NOON + 1800)
        return env

    still, fan, humidifier = run(None), run("FAN"), run("HUMIDIFIER")
    assert fan.state["temperature"] < still.state["temperature"] - 1.5
    assert fan.state["humidity"] < still.state["humidity"] - 5
    assert humidifier.state["humidity"] > still.state["humidity"] + 8


def test_uv_light_shows_in_the_next_reading():
    env = Environment("LETTUCE", seed=1)
    advance(env, 60, start=MIDNIGHT)
    before = env.state["brightness"]
    env.apply("UV_LIGHT", "ACTIVATE", 900, MIDNIGHT + 3600)
    env.step(30, MIDNIGHT + 3630)
    assert env.state["brightness"] > before + 600


def test_the_air_goes_back_when_the_fan_stops():
    env = Environment("BASIL", seed=1)
    advance(env, 30)
    ambient = env.state["temperature"]
    env.apply("FAN", "ACTIVATE", 600, NOON + 1800)
    advance(env, 10, start=NOON + 1800)
    cooled = env.state["temperature"]
    advance(env, 60, start=NOON + 2400)
    assert cooled < env.state["temperature"] and abs(env.state["temperature"] - ambient) < 1.5


def test_placement_filters_the_weather():
    sunny = Weather(30, 45, 0, 900, 0, 850, 5, True, 0, "CLEAR", "Despejado", "", 0)

    def settle(setting: str | None, exposure: str | None) -> Environment:
        env = Environment("TOMATO", seed=1)
        env.set_mode("WEATHER")
        env.set_placement(setting, exposure)
        env.weather = sunny
        advance(env, 180)
        return env

    full_sun, shade, indoor = settle("OUTDOOR", "FULL_SUN"), settle("OUTDOOR", "SHADE"), settle("INDOOR", "PARTIAL_SUN")
    assert full_sun.state["temperature"] > shade.state["temperature"] + 2
    assert full_sun.state["brightness"] > shade.state["brightness"] + 800
    assert indoor.state["temperature"] < 27 and indoor.state["brightness"] < 600
    assert settle(None, None).state["brightness"] == pytest.approx(full_sun.state["brightness"], abs=1)
    with pytest.raises(ValueError, match="Lugar"):
        Environment("TOMATO").set_placement("GARAGE", None)
