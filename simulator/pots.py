"""Cultivos virtuales que se crean, cambian y retiran en caliente.

Los cultivos de `SIMULATOR_DEVICES` (datos demo y pruebas) son fijas; las que pide la API para un cultivo
son administradas: la API las vuelve a crear si el simulador se reinicia. Un solo hilo publica todas las
lecturas y refresca el clima de las que están en modo WEATHER.
"""

import logging
import threading
import time
from collections.abc import Callable
from dataclasses import dataclass, field

from simulator.config import DeviceConfig, Settings
from simulator.device import SimulatedDevice
from simulator.environment import MODES
from simulator.weather import Weather, WeatherClient

log = logging.getLogger(__name__)

TICK_SECONDS = 1.0
WEATHER_RETRY_SECONDS = 120.0


@dataclass
class Location:
    name: str
    latitude: float
    longitude: float


@dataclass
class PotConfig:
    crop_id: str
    key: str
    crop_type: str
    mode: str = "AUTO"
    manual: dict[str, float] = field(default_factory=dict)
    location: Location | None = None
    interval_seconds: float = 30.0
    managed: bool = True


@dataclass
class VirtualPot:
    config: PotConfig
    device: SimulatedDevice
    next_at: float = 0.0
    weather_error: str | None = None
    weather_checked_at: float = 0.0

    @property
    def weather(self) -> Weather | None:
        return self.device.environment.weather


DeviceFactory = Callable[[DeviceConfig, Settings], SimulatedDevice]


class PotManager:
    def __init__(self, settings: Settings, weather: WeatherClient | None = None,
                 factory: DeviceFactory | None = None):
        self.settings = settings
        self.weather_client = weather or WeatherClient()
        self.factory = factory or (lambda device, cfg: SimulatedDevice(device, cfg))
        self.pots: dict[str, VirtualPot] = {}
        self._lock = threading.RLock()
        self._stop = threading.Event()
        self._thread: threading.Thread | None = None

    def load_static(self) -> None:
        for device in self.settings.devices:
            self.upsert(PotConfig(crop_id=device.crop_id, key=device.key, crop_type=device.crop_type,
                                  interval_seconds=self.settings.interval_seconds, managed=False))

    def upsert(self, config: PotConfig) -> VirtualPot:
        if config.mode not in MODES:
            raise ValueError(f"Modo desconocido: {config.mode}")
        if config.mode == "WEATHER" and config.location is None:
            raise ValueError("El modo clima necesita una ubicación")
        with self._lock:
            pot = self.pots.get(config.crop_id)
            previous: PotConfig | None = pot.config if pot else None
            if pot is None or pot.config.key != config.key or pot.config.crop_type != config.crop_type:
                if pot is not None:
                    pot.device.stop()
                device = self.factory(DeviceConfig(config.crop_id, config.key, config.crop_type), self.settings)
                device.start()
                pot = VirtualPot(config=config, device=device)
                self.pots[config.crop_id] = pot
                previous = None
                log.info("Cultivo virtual %s (%s) en modo %s", config.crop_id, config.crop_type, config.mode)
            pot.config = config
            environment = pot.device.environment
            environment.set_mode(config.mode)
            # Solo los medidores que cambiaron: repetir la configuración no borra el efecto de los actuadores.
            before = previous.manual if previous else {}
            changed = {name: value for name, value in config.manual.items() if before.get(name) != value}
            if changed:
                environment.set_manual(changed)
            location_changed = previous is None or previous.location != config.location
            if config.mode == "WEATHER" and (location_changed or environment.weather is None):
                self._refresh_weather(pot, force=True)
            pot.next_at = min(pot.next_at or float("inf"), time.time() + 2)
            return pot

    def remove(self, crop_id: str) -> bool:
        with self._lock:
            pot = self.pots.pop(crop_id, None)
        if pot is None:
            return False
        pot.device.stop()
        log.info("Cultivo virtual %s retirado", crop_id)
        return True

    def get(self, crop_id: str) -> VirtualPot | None:
        with self._lock:
            return self.pots.get(crop_id)

    def all(self) -> list[VirtualPot]:
        with self._lock:
            return list(self.pots.values())

    def tick(self, now: float) -> None:
        for pot in self.all():
            if now < pot.next_at:
                continue
            pot.next_at = now + pot.config.interval_seconds
            if pot.config.mode == "WEATHER":
                self._refresh_weather(pot)
            try:
                pot.device.publish_reading(now)
            except Exception:  # noqa: BLE001 - un cultivo con problemas no detiene a los demás
                log.exception("El cultivo %s no pudo publicar", pot.config.crop_id)

    def _refresh_weather(self, pot: VirtualPot, force: bool = False) -> None:
        location = pot.config.location
        if location is None:
            return
        now = time.time()
        if not force and pot.weather_error and now - pot.weather_checked_at < WEATHER_RETRY_SECONDS:
            return
        pot.weather_checked_at = now
        try:
            pot.device.environment.weather = self.weather_client.current(location.latitude, location.longitude)
            pot.weather_error = None
        except Exception as error:  # noqa: BLE001 - sin clima se conserva el último conocido
            pot.weather_error = "No se pudo consultar el clima; se usa el último dato conocido."
            log.warning("Clima no disponible para %s: %s", location.name, error)

    def start(self) -> None:
        if self._thread is None:
            self._thread = threading.Thread(target=self._run, name="cultivos", daemon=True)
            self._thread.start()

    def _run(self) -> None:
        while not self._stop.wait(TICK_SECONDS):
            self.tick(time.time())

    def stop(self) -> None:
        self._stop.set()
        if self._thread is not None:
            self._thread.join(timeout=5)
        for pot in self.all():
            pot.device.stop()

    def snapshot(self, pot: VirtualPot, now: float | None = None) -> dict:
        now = now or time.time()
        environment = pot.device.environment
        config = pot.config
        return {
            "cropId": config.crop_id,
            "cropType": config.crop_type,
            "mode": config.mode,
            "managed": config.managed,
            "connected": pot.device.connected,
            "intervalSeconds": config.interval_seconds,
            "lastReading": pot.device.last_reading,
            "lastPublishedAt": _iso(pot.device.last_published_at),
            "manual": {name: round(value, 2) for name, value in environment.manual.items()},
            "location": None if config.location is None else {
                "name": config.location.name, "latitude": config.location.latitude,
                "longitude": config.location.longitude},
            "weather": None if environment.weather is None else environment.weather.as_dict(),
            "weatherError": pot.weather_error,
            "activeActuators": [{"actuator": name, "until": _iso(until)}
                                for name, until in environment.active_actuators(now).items()],
            "lastCommand": None if pot.device.last_command is None else {
                **{k: v for k, v in pot.device.last_command.items() if k != "at"},
                "at": _iso(pot.device.last_command["at"])},
        }


def _iso(moment: float | None) -> str | None:
    if moment is None:
        return None
    return time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime(moment))
