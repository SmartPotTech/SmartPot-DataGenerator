"""Punto de entrada: python -m simulator."""

import logging
import os
import signal
import threading
import time

from simulator.config import load
from simulator.device import SimulatedDevice


def main() -> None:
    logging.basicConfig(level=os.environ.get("LOG_LEVEL", "INFO"),
                        format="%(asctime)s %(levelname)s %(name)s - %(message)s")
    settings = load()
    devices = [SimulatedDevice(device, settings) for device in settings.devices]
    for device in devices:
        device.start()
    logging.info("Simulador iniciado con %d macetas; lectura cada %.0f s", len(devices), settings.interval_seconds)

    stop = threading.Event()
    signal.signal(signal.SIGTERM, lambda *_: stop.set())
    signal.signal(signal.SIGINT, lambda *_: stop.set())
    while not stop.is_set():
        now = time.time()
        for device in devices:
            device.publish_reading(now)
        stop.wait(settings.interval_seconds)

    for device in devices:
        device.stop()
    logging.info("Simulador detenido")


if __name__ == "__main__":
    main()
