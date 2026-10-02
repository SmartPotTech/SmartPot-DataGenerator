"""Modelo físico simplificado de un cultivo hidropónico.

Cada variable se acerca al ambiente que marca el modo y los actuadores encendidos la empujan por su cuenta,
en todos los modos: el ventilador enfría y seca el aire, el humidificador lo humedece, la luz ultravioleta suma
luz y la bomba moja el sustrato. Al apagarlos, el ambiente vuelve poco a poco. Los valores quedan siempre dentro
de la escala de los sensores del dispositivo, con ruido gaussiano.

- AUTO: ciclo de día y noche alrededor de la línea base de la especie.
- MANUAL: el ambiente son los medidores que mueve la persona.
- WEATHER: el clima real del lugar (temperatura, humedad, sol, lluvia y presión), filtrado por dónde está el
  cultivo: bajo techo o al aire libre, a pleno sol, en media sombra o en sombra.
"""

import math
import random
from dataclasses import dataclass, field

from simulator.weather import Weather

BASELINES = {
    "LETTUCE": {"temperature": 19, "humidity": 62, "brightness": 850, "ph": 6.0, "tds": 700, "soilMoisture": 70},
    "TOMATO": {"temperature": 25, "humidity": 68, "brightness": 1300, "ph": 6.1, "tds": 2000, "soilMoisture": 64},
    "STRAWBERRY": {"temperature": 22, "humidity": 66, "brightness": 1100, "ph": 5.9, "tds": 840, "soilMoisture": 70},
    "BASIL": {"temperature": 25, "humidity": 50, "brightness": 1100, "ph": 6.0, "tds": 900, "soilMoisture": 60},
    "SPINACH": {"temperature": 19, "humidity": 60, "brightness": 850, "ph": 6.5, "tds": 1400, "soilMoisture": 70},
    "PEPPER": {"temperature": 25, "humidity": 60, "brightness": 1300, "ph": 6.1, "tds": 1750, "soilMoisture": 64},
}

LIMITS = {
    "temperature": (-20.0, 60.0),
    "humidity": (0.0, 100.0),
    "brightness": (0.0, 2000.0),
    "ph": (0.0, 14.0),
    "tds": (0.0, 3000.0),
    "soilMoisture": (0.0, 100.0),
    "atmosphere": (300.0, 1100.0),
}
MODES = ("AUTO", "MANUAL", "WEATHER")
SETTINGS = ("INDOOR", "OUTDOOR")
EXPOSURES = ("FULL_SUN", "PARTIAL_SUN", "SHADE")
# Luz del sensor (0 a 2000) por cada W/m² de radiación solar.
LIGHT_PER_WATT = 2.2
# Fracción del sol que llega a la planta: al aire libre según su exposición; bajo techo, según su ventana.
SUNLIGHT = {
    "OUTDOOR": {"FULL_SUN": 1.0, "PARTIAL_SUN": 0.55, "SHADE": 0.25},
    "INDOOR": {"FULL_SUN": 0.45, "PARTIAL_SUN": 0.25, "SHADE": 0.08},
}
# Día y noche: cuánto cambia la luz típica de la especie según el lugar.
DAYLIGHT_FACTOR = {
    "OUTDOOR": {"FULL_SUN": 1.3, "PARTIAL_SUN": 1.0, "SHADE": 0.6},
    "INDOOR": {"FULL_SUN": 0.9, "PARTIAL_SUN": 0.7, "SHADE": 0.35},
}
# Efecto de cada actuador por minuto encendido (la bomba, por segundo).
FAN_COOLING = 0.35
FAN_DRYING = 1.5
HUMIDIFIER_RATE = 2.5
LAMP_LIGHT = 900
LAMP_HEAT = 0.05
PUMP_RATE = 1.2

# Nombre en español y terminación para concordar el mensaje de confirmación.
ACTUATOR_NAMES = {
    "WATER_PUMP": ("Bomba de agua", "a"),
    "UV_LIGHT": ("Luz ultravioleta", "a"),
    "FAN": ("Ventilador", "o"),
    "HUMIDIFIER": ("Humidificador", "o"),
    "NUTRIENT_DOSER": ("Dosificador de nutrientes", "o"),
    "PH_DOSER": ("Dosificador de pH", "o"),
}
SUPPORTED_ACTUATORS = set(ACTUATOR_NAMES)
# Los dosificadores sueltan una dosis: sin duración, la de 3 s.
DOSERS = {"NUTRIENT_DOSER", "PH_DOSER"}
DOSE_SECONDS = 3


def clamp(name: str, value: float) -> float:
    low, high = LIMITS[name]
    return max(low, min(high, value))


def readable(seconds: int) -> str:
    """La duración como la lee una persona: 15 s, 10 min, 2 h."""
    if seconds % 3600 == 0:
        return f"{seconds // 3600} h"
    if seconds % 60 == 0:
        return f"{seconds // 60} min"
    return f"{seconds} s"


@dataclass
class Environment:
    crop_type: str
    seed: int | None = None
    utc_offset_hours: float = 0.0
    state: dict[str, float] = field(default_factory=dict)
    active: dict[str, tuple[float, float | None]] = field(default_factory=dict)
    mode: str = "AUTO"
    manual: dict[str, float] = field(default_factory=dict)
    weather: Weather | None = None
    # Sin lugar definido, día y noche usa la línea base y el clima supone aire libre a pleno sol.
    setting: str | None = None
    exposure: str | None = None

    def __post_init__(self) -> None:
        self.random = random.Random(self.seed)
        self.base = BASELINES.get(self.crop_type.upper(), BASELINES["LETTUCE"])
        if not self.state:
            self.state = {**self.base, "atmosphere": 1012.0}
        if not self.manual:
            self.manual = dict(self.state)

    def set_mode(self, mode: str) -> None:
        if mode not in MODES:
            raise ValueError(f"Modo desconocido: {mode}")
        self.mode = mode

    def set_placement(self, setting: str | None, exposure: str | None) -> None:
        if setting is not None and setting not in SETTINGS:
            raise ValueError(f"Lugar desconocido: {setting}")
        if exposure is not None and exposure not in EXPOSURES:
            raise ValueError(f"Exposición desconocida: {exposure}")
        self.setting, self.exposure = setting, exposure

    def set_manual(self, values: dict[str, float | None]) -> None:
        """Mueve los medidores: el valor cambia de inmediato y luego evoluciona con la física y los actuadores."""
        for name, value in values.items():
            if value is None or name not in LIMITS:
                continue
            self.manual[name] = clamp(name, float(value))
            self.state[name] = self.manual[name]

    def active_actuators(self, now: float) -> dict[str, float | None]:
        """Actuadores encendidos y el momento (epoch) en que se apagan; None si siguen hasta apagarlos."""
        return {name: window[1] for name, window in self.active.items() if self.is_active(name, now)}

    def is_active(self, actuator: str, now: float) -> bool:
        window = self.active.get(actuator)
        return window is not None and window[0] <= now and (window[1] is None or now < window[1])

    def active_fraction(self, actuator: str, start: float, end: float) -> float:
        """Fracción del intervalo [start, end] en la que el actuador estuvo encendido."""
        window = self.active.get(actuator)
        if window is None or end <= start:
            return 0.0
        stop = end if window[1] is None else min(end, window[1])
        return max(0.0, stop - max(start, window[0])) / (end - start)

    def apply(self, actuator: str, action: str, duration: int | None, now: float) -> str:
        """Aplica un comando y devuelve el mensaje del ACK. Sin duración, sigue encendido hasta apagarlo."""
        if actuator not in SUPPORTED_ACTUATORS:
            raise ValueError(f"El actuador {actuator} no existe en este cultivo")
        name, ending = ACTUATOR_NAMES[actuator]
        if action == "DEACTIVATE":
            self.active.pop(actuator, None)
            return f"{name} apagad{ending}"
        if actuator in DOSERS:
            duration = duration or DOSE_SECONDS
        self.active[actuator] = (now, now + duration if duration else None)
        if actuator == "PH_DOSER":
            self.state["ph"] = clamp("ph", self.state["ph"] - 0.12 * duration)
        elif actuator == "NUTRIENT_DOSER":
            self.state["tds"] = clamp("tds", self.state["tds"] + 70 * duration)
        return f"{name} encendid{ending}" + (f" por {readable(duration)}" if duration else "")

    def step(self, seconds: float, now: float) -> None:
        minutes = seconds / 60.0
        start = now - seconds
        fan = self.active_fraction("FAN", start, now) * minutes
        humidifier = self.active_fraction("HUMIDIFIER", start, now) * minutes
        uv_light = self.active_fraction("UV_LIGHT", start, now)
        pump_seconds = self.active_fraction("WATER_PUMP", start, now) * seconds

        temperature, humidity, light, pressure, drying, rain = self._targets(now)
        self._relax("temperature", temperature, 0.08 * minutes)
        self._relax("humidity", humidity, 0.1 * minutes)
        # La luz cambia casi al instante: la lámpara se nota en la lectura siguiente.
        self._relax("brightness", light + LAMP_LIGHT * uv_light, 2.0 * minutes)
        self._relax("atmosphere", pressure, 0.05 * minutes)
        self._push("temperature", -FAN_COOLING * fan + LAMP_HEAT * uv_light * minutes)
        self._push("humidity", -FAN_DRYING * fan + HUMIDIFIER_RATE * humidifier)

        evaporation = 0.03 * minutes * max(0.5, self.state["temperature"] / 20) * drying
        irrigation = PUMP_RATE * pump_seconds + rain * minutes
        self.state["soilMoisture"] = clamp("soilMoisture", self.state["soilMoisture"] - evaporation + irrigation)
        if self.mode == "MANUAL":
            self._relax("ph", self.manual["ph"], 0.02 * minutes)
            self._relax("tds", self.manual["tds"], 0.02 * minutes)
        else:
            self.state["ph"] = clamp("ph", self.state["ph"] + 0.002 * minutes)
            self.state["tds"] = clamp("tds", self.state["tds"] - 0.4 * minutes)

    def _targets(self, now: float) -> tuple[float, float, float, float, float, float]:
        """Temperatura, humedad, luz y presión objetivo; factor de secado y lluvia (% de sustrato por minuto)."""
        if self.mode == "MANUAL":
            m = self.manual
            return m["temperature"], m["humidity"], m["brightness"], m["atmosphere"], 1.0, 0.0
        if self.mode == "WEATHER" and self.weather is not None:
            return self._weather_targets(self.weather)
        hour = (now / 3600.0 + self.utc_offset_hours) % 24
        daylight = math.sin((hour - 6) / 12 * math.pi)
        base = self.base
        factor = DAYLIGHT_FACTOR[self.setting][self.exposure or "PARTIAL_SUN"] if self.setting else 1.0
        return (base["temperature"] + 3 * daylight, base["humidity"] - 6 * daylight,
                base["brightness"] * factor * max(0.05, daylight + 0.3), 1012.0, 1.0, 0.0)

    def _weather_targets(self, w: Weather) -> tuple[float, float, float, float, float, float]:
        setting = self.setting or "OUTDOOR"
        sun = SUNLIGHT[setting][self.exposure or "FULL_SUN"]
        light = min(LIMITS["brightness"][1], w.radiation * LIGHT_PER_WATT * sun)
        if setting == "INDOOR":
            # Bajo techo el clima de afuera llega amortiguado, una ventana soleada calienta y no llueve.
            temperature = 21 + (w.temperature - 21) * 0.35 + 2.0 * sun * w.radiation / 1000
            humidity = 55 + (w.humidity - 55) * 0.4
            return temperature, humidity, light, w.pressure, 0.8 * (1 + light / 1500), 0.0
        # Al sol la planta se calienta sobre la temperatura del aire; el aire seco y el sol secan más rápido y la
        # lluvia moja el sustrato (unos 0,8 % por mm y hora).
        temperature = w.temperature + 4.0 * sun * w.radiation / 1000
        drying = (1 + sun * w.radiation / 600) * max(0.3, 1.3 - w.humidity / 100)
        return temperature, w.humidity, light, w.pressure, drying, w.precipitation * 0.8 / 60

    def _relax(self, name: str, target: float, rate: float) -> None:
        current = self.state[name]
        self.state[name] = clamp(name, current + (target - current) * min(1.0, rate))

    def _push(self, name: str, delta: float) -> None:
        self.state[name] = clamp(name, self.state[name] + delta)

    def reading(self) -> dict[str, float]:
        noise = {"temperature": 0.2, "humidity": 0.6, "brightness": 15, "ph": 0.03, "tds": 8, "soilMoisture": 0.4,
                 "atmosphere": 0.4}
        decimals = {"brightness": 0, "tds": 0, "ph": 2}
        return {
            name: round(clamp(name, value + self.random.gauss(0, noise[name])), decimals.get(name, 1))
            for name, value in self.state.items()
        }
