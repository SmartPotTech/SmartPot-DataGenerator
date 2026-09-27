"""Clima actual de un lugar con Open-Meteo: servicio abierto, sin clave y sin registro.

La maceta virtual en modo clima copia la temperatura, la humedad, la luz (radiación solar), la lluvia y la
presión del lugar, y la PWA usa la condición (despejado, nublado, lluvia…) para ilustrar la escena.
"""

import json
import logging
import threading
import time
import urllib.parse
import urllib.request
from collections.abc import Callable
from dataclasses import asdict, dataclass

log = logging.getLogger(__name__)

WEATHER_URL = "https://api.open-meteo.com/v1/forecast"
GEOCODING_URL = "https://geocoding-api.open-meteo.com/v1/search"
CURRENT = ("temperature_2m,relative_humidity_2m,is_day,precipitation,weather_code,cloud_cover,surface_pressure,"
           "wind_speed_10m,shortwave_radiation")
TIMEOUT_SECONDS = 8
USER_AGENT = "SmartPot-Simulator/2.0 (+https://github.com/SmartPotTech/SmartPot-DataGenerator)"

# Códigos WMO agrupados en las escenas que dibuja la PWA.
CONDITIONS = (
    ((0,), "CLEAR", "Despejado"),
    ((1,), "MOSTLY_CLEAR", "Mayormente despejado"),
    ((2,), "PARTLY_CLOUDY", "Parcialmente nublado"),
    ((3,), "CLOUDY", "Nublado"),
    ((45, 48), "FOG", "Niebla"),
    ((51, 53, 55, 56, 57), "DRIZZLE", "Llovizna"),
    ((61, 63, 65, 66, 67, 80, 81, 82), "RAIN", "Lluvia"),
    ((71, 73, 75, 77, 85, 86), "SNOW", "Nieve"),
    ((95, 96, 99), "STORM", "Tormenta"),
)

Fetch = Callable[[str], dict]


def condition(code: int) -> tuple[str, str]:
    for codes, key, label in CONDITIONS:
        if code in codes:
            return key, label
    return "CLOUDY", "Nublado"


@dataclass(frozen=True)
class Place:
    name: str
    latitude: float
    longitude: float
    country: str | None = None
    region: str | None = None

    def as_dict(self) -> dict:
        return asdict(self)


@dataclass(frozen=True)
class Weather:
    temperature: float
    humidity: float
    cloud_cover: float
    radiation: float
    precipitation: float
    pressure: float
    wind_speed: float
    is_day: bool
    code: int
    condition: str
    label: str
    observed_at: str
    fetched_at: float

    def as_dict(self) -> dict:
        data = asdict(self)
        data.pop("fetched_at")
        return data


def _http_json(url: str) -> dict:
    request = urllib.request.Request(url, headers={"User-Agent": USER_AGENT, "Accept": "application/json"})
    with urllib.request.urlopen(request, timeout=TIMEOUT_SECONDS) as response:  # noqa: S310 - URL fija https
        return json.loads(response.read().decode("utf-8"))


class WeatherClient:
    """Consulta con caché por coordenadas redondeadas: una llamada cada `ttl` segundos por lugar."""

    def __init__(self, fetch: Fetch | None = None, ttl: float = 600.0):
        self.fetch = fetch or _http_json
        self.ttl = ttl
        self._cache: dict[tuple[float, float], Weather] = {}
        self._lock = threading.Lock()

    def current(self, latitude: float, longitude: float) -> Weather:
        key = (round(latitude, 2), round(longitude, 2))
        with self._lock:
            cached = self._cache.get(key)
        if cached and time.time() - cached.fetched_at < self.ttl:
            return cached
        query = urllib.parse.urlencode({"latitude": key[0], "longitude": key[1], "current": CURRENT,
                                        "timezone": "auto"})
        data = self.fetch(f"{WEATHER_URL}?{query}")["current"]
        code = int(data.get("weather_code") or 0)
        key_name, label = condition(code)
        weather = Weather(
            temperature=float(data["temperature_2m"]),
            humidity=float(data["relative_humidity_2m"]),
            cloud_cover=float(data.get("cloud_cover") or 0),
            radiation=float(data.get("shortwave_radiation") or 0),
            precipitation=float(data.get("precipitation") or 0),
            pressure=float(data.get("surface_pressure") or 1013),
            wind_speed=float(data.get("wind_speed_10m") or 0),
            is_day=bool(data.get("is_day")),
            code=code,
            condition=key_name,
            label=label,
            observed_at=str(data.get("time", "")),
            fetched_at=time.time(),
        )
        with self._lock:
            self._cache[key] = weather
        return weather

    def search(self, query: str, count: int = 5) -> list[Place]:
        params = urllib.parse.urlencode({"name": query, "count": count, "language": "es", "format": "json"})
        results = self.fetch(f"{GEOCODING_URL}?{params}").get("results") or []
        return [Place(name=item["name"], latitude=round(float(item["latitude"]), 4),
                      longitude=round(float(item["longitude"]), 4), country=item.get("country"),
                      region=item.get("admin1")) for item in results]
