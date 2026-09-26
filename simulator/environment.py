"""Modelo físico simplificado de una maceta hidropónica.

Cada variable se acerca a un objetivo que depende de la hora del día y de los actuadores
encendidos, con ruido gaussiano. Los valores quedan siempre dentro de la escala de los
sensores de la maceta.
"""

import math
import random
from dataclasses import dataclass, field

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
    "atmosphere": (950.0, 1050.0),
}

SUPPORTED_ACTUATORS = {"WATER_PUMP", "UV_LIGHT", "FAN", "HUMIDIFIER", "NUTRIENT_DOSER", "PH_DOSER"}


def clamp(name: str, value: float) -> float:
    low, high = LIMITS[name]
    return max(low, min(high, value))


@dataclass
class Environment:
    crop_type: str
    seed: int | None = None
    state: dict[str, float] = field(default_factory=dict)
    active: dict[str, tuple[float, float]] = field(default_factory=dict)

    def __post_init__(self) -> None:
        self.random = random.Random(self.seed)
        self.base = BASELINES.get(self.crop_type.upper(), BASELINES["LETTUCE"])
        if not self.state:
            self.state = {**self.base, "atmosphere": 1012.0}

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
            raise ValueError(f"El actuador {actuator} no existe en esta maceta")
        if action == "DEACTIVATE":
            self.active.pop(actuator, None)
            return f"{actuator} apagado"
        seconds = duration if duration else 3600
        self.active[actuator] = (now, now + seconds)
        if actuator == "PH_DOSER":
            self.state["ph"] = clamp("ph", self.state["ph"] - 0.12 * seconds)
        elif actuator == "NUTRIENT_DOSER":
            self.state["tds"] = clamp("tds", self.state["tds"] + 70 * seconds)
        return f"{actuator} encendido {seconds} s"

    def step(self, seconds: float, now: float) -> None:
        hour = (now / 3600.0) % 24
        daylight = math.sin((hour - 6) / 12 * math.pi)
        minutes = seconds / 60.0
        start = now - seconds
        base = self.base
        fan = self.active_fraction("FAN", start, now)
        humidifier = self.active_fraction("HUMIDIFIER", start, now)
        uv_light = self.active_fraction("UV_LIGHT", start, now)
        pump_seconds = self.active_fraction("WATER_PUMP", start, now) * seconds

        target_temperature = base["temperature"] + 3 * daylight - 2.0 * fan
        target_humidity = base["humidity"] - 6 * daylight - 6.0 * fan + 12.0 * humidifier
        target_light = base["brightness"] * max(0.05, daylight + 0.3) + 700 * uv_light

        self._relax("temperature", target_temperature, 0.08 * minutes)
        self._relax("humidity", target_humidity, 0.1 * minutes)
        self._relax("brightness", target_light, 0.5 * minutes)
        self._relax("atmosphere", 1012.0, 0.05 * minutes)

        evaporation = 0.03 * minutes * max(0.5, self.state["temperature"] / 20)
        irrigation = 1.2 * pump_seconds
        self.state["soilMoisture"] = clamp("soilMoisture", self.state["soilMoisture"] - evaporation + irrigation)
        self.state["ph"] = clamp("ph", self.state["ph"] + 0.002 * minutes)
        self.state["tds"] = clamp("tds", self.state["tds"] - 0.4 * minutes)

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
