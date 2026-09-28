"""Punto de entrada: python -m simulator.

Arranca los cultivos fijos de SIMULATOR_DEVICES y la API interna de control (con SIMULATOR_TOKEN la API
puede simular cualquier cultivo virtual).
"""

import logging
import os
import uvicorn

from simulator.api import create_app
from simulator.config import load
from simulator.pots import PotManager


def main() -> None:
    logging.basicConfig(level=os.environ.get("LOG_LEVEL", "INFO"),
                        format="%(asctime)s %(levelname)s %(name)s - %(message)s")
    settings = load()
    manager = PotManager(settings)
    manager.load_static()
    manager.start()
    logging.info("Simulador iniciado con %d cultivos fijos; API de control %s", len(settings.devices),
                 "activa" if settings.api_token else "deshabilitada (falta SIMULATOR_TOKEN)")
    try:
        uvicorn.run(create_app(manager, settings.api_token), host="0.0.0.0", port=settings.api_port,
                    log_level="warning", server_header=False)
    finally:
        manager.stop()
        logging.info("Simulador detenido")


if __name__ == "__main__":
    main()
