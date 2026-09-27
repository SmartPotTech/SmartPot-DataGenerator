# SmartPot-DataGenerator

## Estado del Proyecto

[![Python CI](https://github.com/SmartPotTech/SmartPot-DataGenerator/actions/workflows/ci.yml/badge.svg)](https://github.com/SmartPotTech/SmartPot-DataGenerator/actions/workflows/ci.yml)
[![Publish Docker Images](https://github.com/SmartPotTech/SmartPot-DataGenerator/actions/workflows/packaging.yml/badge.svg)](https://github.com/SmartPotTech/SmartPot-DataGenerator/actions/workflows/packaging.yml)

## Descripción

SmartPot-DataGenerator es el **simulador de macetas** de SmartPot. Se conecta al broker como uno o varios dispositivos, con las mismas credenciales y tópicos que una maceta real, y:

- Publica telemetría (temperatura, humedad, luz, pH, TDS, humedad del sustrato y presión) con ciclo día/noche y ruido de sensor.
- Obedece los comandos: la bomba sube la humedad del sustrato, el ventilador enfría y seca el aire, la luz UV suma luz, el humidificador sube la humedad y los dosificadores corrigen pH y nutrientes.
- Confirma cada comando con su ACK (`EXECUTED` o `FAILED`) y anuncia su estado `online`/`offline` con mensaje retenido y última voluntad.

Sirve para ver SmartPot funcionando sin hardware, para la demo de un solo comando y para las pruebas de extremo a extremo de la organización.

## Estructura del Proyecto

```text
SmartPot-DataGenerator/
├── simulator/
│   ├── __main__.py         # Bucle principal: una lectura por maceta cada N segundos
│   ├── config.py           # Variables de entorno y lista de macetas
│   ├── device.py           # Cliente MQTT: telemetría, comandos, ACK y estado
│   └── environment.py      # Modelo físico de la maceta y efecto de los actuadores
├── tests/                  # Modelo físico, contrato de tópicos y manejo de comandos
├── Dockerfile              # Imagen sin privilegios con uv
├── pyproject.toml / uv.lock
└── .env.example
```

## Configuración

| Variable | Por defecto | Descripción |
| --- | --- | --- |
| `MQTT_HOST` | `localhost` | Broker |
| `MQTT_PORT` | `1883` (`8883` con TLS) | Puerto |
| `MQTT_TLS` | `false` | Conexión TLS |
| `MQTT_CA_FILE` | — | CA que firma el certificado del broker (`ca.crt`) |
| `MQTT_TOPIC_PREFIX` | `smartpot/v1` | Prefijo del contrato |
| `SIMULATOR_DEVICES` | — | `cropId:clave:TIPO` separados por comas |
| `SIMULATOR_INTERVAL_SECONDS` | `30` | Segundos entre lecturas |
| `SIMULATOR_UTC_OFFSET` | `-5` | Diferencia horaria con UTC para el ciclo de día y noche |

El usuario MQTT de cada maceta es el id de su cultivo y la clave es la que entrega la API al crear el cultivo (`POST /api/v1/crops`) o al rotarla (`POST /api/v1/crops/{id}/device/key`). Los datos demo de SmartPot-DB traen dos macetas listas para simular.

## Guía de Instalación

### Requisitos Previos

- [uv](https://docs.astral.sh/uv/)
- Un broker de SmartPot con las cuentas de dispositivo aprovisionadas por la API

### Ejecución local

```bash
git clone https://github.com/SmartPotTech/SmartPot-DataGenerator.git
cd SmartPot-DataGenerator
uv sync
set -a; . ./.env.example; set +a    # o exporta las variables a mano
uv run python -m simulator
```

Contra producción:

```bash
MQTT_HOST=mqtt.smartpot.app MQTT_TLS=true MQTT_CA_FILE=ca.crt \
SIMULATOR_DEVICES=<cropId>:<clave>:LETTUCE uv run python -m simulator
```

### Pruebas

```bash
uv run ruff check .
uv run pytest
```

### Imagen Docker

```bash
docker pull ghcr.io/smartpottech/smartpot-datagenerator:latest
```

## Licencia

Este proyecto está bajo la licencia MIT. Consulta el archivo [LICENSE](LICENSE) para más detalles.
