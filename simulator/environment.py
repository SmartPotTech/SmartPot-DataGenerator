"""Modelo físico simplificado de un cultivo hidropónico.

Cada variable se acerca a un objetivo que depende del modo y de los actuadores encendidos, con ruido
gaussiano. Los valores quedan siempre dentro de la escala de los sensores del dispositivo.

- AUTO: ciclo de día y noche alrededor de la línea base de la especie.
- MANUAL: los objetivos son los medidores que mueve la persona.
- WEATHER: el cultivo está al aire libre y sigue el clima real del lugar (temperatura, humedad, sol,
  lluvia y presión).
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
# Luz del sensor (0 a 2000) por cada W/m² de radiación solar.
LIGHT_PER_WATT = 2.2

# Nombre en español y terminación para concordar el mensaje de confirmación.
ACTUATOR_NAMES = {
    "WATER_PUMP": ("Bomba de agua", "a"),
    "UV_LIGHT": ("Luz de cultivo", "a"),
    "FAN": ("Ventilador", "o"),
    "HUMIDIFIER": ("Humidificador", "o"),
    "NUTRIENT_DOSER": ("Dosificador de nutrientes", "o"),
    "PH_DOSER": ("Dosificador de pH", "o"),
}
SUPPORTED_ACTUATORS = set(ACTUATOR_NAMES)


def clamp(name: str, value: float) -> float:
    low, high = LIMITS[name]
    return max(low, min(high, value))


@dataclass
class Environment:
    crop_type: str
    seed: int | None = None
    utc_offset_hours: float = 0.0
    state: dict[str, float] = field(default_factory=dict)
    active: dict[str, tuple[float, float]] = field(default_factory=dict)
    mode: str = "AUTO"
    manual: dict[str, float] = field(default_factory=dict)
    weather: Weather | None = None

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

    def set_manual(self, values: dict[str, float | None]) -> None:
        """Mueve los medidores: el valor cambia de inmediato y luego evoluciona con la física y los actuadores."""
        for name, value in values.items():
            if value is None or name not in LIMITS:
                continue
            self.manual[name] = clamp(name, float(value))
            self.state[name] = self.manual[name]

    def active_actuators(self, now: float) -> dict[str, float]:
        """Actuadores encendidos y el momento (epoch) en que se apagan."""
        return {name: window[1] for name, window in self.active.items() if window[0] <= now < window[1]}

    def is_active(self, actuator: str, now: float) -> bool:
        window = self.active.get(actuator)
        return window is not None and window[0] <= now < window[1]

    def active_fraction(self, actuator: str, start: float, end: float) -> float:
        """Fracción del intervalo [start, end] en la que el actuador estuvo encendido."""
        window = self.active.get(actuator)
        if window is None or end <= start:
            return 0.0
        overlap = min(end, window[1]) - max(start, window[0])
        return max(0.0, overlap) / (end - start)

    def apply(self, actuator: str, action: str, duration: int | None, now: float) -> str:
        """Aplica un comando y devuelve el mensaje del ACK."""
        if actuator not in SUPPORTED_ACTUATORS:
            raise ValueError(f"El actuador {actuator} no existe en este cultivo")
        if action == "DEACTIVATE":
            self.active.pop(actuator, None)
            name, ending = ACTUATOR_NAMES[actuator]
            return f"{name} apagad{ending}"
        seconds = duration if duration else 3600
        self.active[actuator] = (now, now + seconds)
        if actuator == "PH_DOSER":
            self.state["ph"] = clamp("ph", self.state["ph"] - 0.12 * seconds)
        elif actuator == "NUTRIENT_DOSER":
            self.state["tds"] = clamp("tds", self.state["tds"] + 70 * seconds)
        name, ending = ACTUATOR_NAMES[actuator]
        return f"{name} encendid{ending} por {seconds} s"

    def step(self, seconds: float, now: float) -> None:
        minutes = seconds / 60.0
        start = now - seconds
        fan = self.active_fraction("FAN", start, now)
        humidifier = self.active_fraction("HUMIDIFIER", start, now)
        uv_light = self.active_fraction("UV_LIGHT", start, now)
        pump_seconds = self.active_fraction("WATER_PUMP", start, now) * seconds

        temperature, humidity, light, pressure, drying, rain = self._targets(now)
        self._relax("temperature", temperature - 2.0 * fan, 0.08 * minutes)
        self._relax("humidity", humidity - 6.0 * fan + 12.0 * humidifier, 0.1 * minutes)
        self._relax("brightness", light + 700 * uv_light, 0.5 * minutes)
        self._relax("atmosphere", pressure, 0.05 * minutes)

        evaporation = 0.03 * minutes * max(0.5, self.state["temperature"] / 20) * drying
        irrigation = 1.2 * pump_seconds + rain * minutes
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
            w = self.weather
            light = min(LIMITS["brightness"][1], w.radiation * LIGHT_PER_WATT)
            # Sol y aire seco secan más rápido; la lluvia moja el sustrato (unos 0,8 % por mm y hora).
            drying = (1 + w.radiation / 600) * max(0.3, 1.3 - w.humidity / 100)
            rain = w.precipitation * 0.8 / 60
            return w.temperature, w.humidity, light, w.pressure, drying, rain
        hour = (now / 3600.0 + self.utc_offset_hours) % 24
        daylight = math.sin((hour - 6) / 12 * math.pi)
        base = self.base
        return (base["temperature"] + 3 * daylight, base["humidity"] - 6 * daylight,
                base["brightness"] * max(0.05, daylight + 0.3), 1012.0, 1.0, 0.0)

    def _relax(self, name: str, target: float, rate: float) -> None:
        current = self.state[name]
        self.state[name] = clamp(name, current + (target - current) * min(1.0, rate))

    def reading(self) -> dict[str, float]:
        noise = {"temperature": 0.2, "humidity": 0.6, "brightness": 15, "ph": 0.03, "tds": 8, "soilMoisture": 0.4,
                 "atmosphere": 0.4}
        decimals = {"brightness": 0, "tds": 0, "ph": 2}
        return {
            name: round(clamp(name, value + self.random.gauss(0, noise[name])), decimals.get(name, 1))
            for name, value in self.state.items()
        }
