# SmartPot-DataGenerator

## Estado del Proyecto

[![Python CI](https://github.com/SmartPotTech/SmartPot-DataGenerator/actions/workflows/ci.yml/badge.svg)](https://github.com/SmartPotTech/SmartPot-DataGenerator/actions/workflows/ci.yml)
[![Publish Docker Images](https://github.com/SmartPotTech/SmartPot-DataGenerator/actions/workflows/packaging.yml/badge.svg)](https://github.com/SmartPotTech/SmartPot-DataGenerator/actions/workflows/packaging.yml)

## Descripción

SmartPot-DataGenerator es el **simulador de macetas** de SmartPot: macetas virtuales que corren sin parar en un contenedor y hablan el mismo contrato MQTT v1 que una maceta física, con las mismas credenciales y tópicos. Cada maceta:

- Publica telemetría (temperatura, humedad, luz, pH, TDS, humedad del sustrato y presión) con ruido de sensor.
- Obedece los comandos: la bomba sube la humedad del sustrato, el ventilador enfría y seca el aire, la luz de cultivo suma luz, el humidificador sube la humedad y los dosificadores corrigen pH y nutrientes.
- Confirma cada comando con su ACK (`EXECUTED` o `FAILED`) y anuncia su estado `online`/`offline` con mensaje retenido y última voluntad.

Cada maceta trabaja en uno de tres modos:

| Modo | Qué refleja | Para qué sirve |
| --- | --- | --- |
| `AUTO` | Ciclo de día y noche alrededor de los valores ideales de la especie | Datos demo y pruebas de extremo a extremo |
| `MANUAL` | Los medidores que mueve la persona desde la PWA; los actuadores siguen actuando encima | Provocar una situación (sustrato seco, pH alto…) y ver cómo reacciona el asistente |
| `WEATHER` | El clima real del lugar elegido: temperatura, humedad, sol, lluvia y presión, con [Open-Meteo](https://open-meteo.com) (abierto y sin clave) | Una maceta «al aire libre» que se seca más rápido con sol y aire seco y se moja cuando llueve |

En modo clima la PWA dibuja la escena según la condición (despejado, nublado, niebla, llovizna, lluvia, tormenta o nieve) y si es de día o de noche.

### Simulador o Wokwi

| | Este simulador | [SmartPot-IoT](https://github.com/SmartPotTech/SmartPot-IoT) en Wokwi |
| --- | --- | --- |
| Qué ejecuta | Modelo físico en Python | El firmware MicroPython real sobre un ESP32 simulado |
| Cómo corre | Contenedor siempre encendido, desplegado con la plataforma | A mano, en el navegador, mientras la pestaña esté abierta |
| Uso | Demo, macetas virtuales de la PWA y QA | Validar el firmware antes de pasar a hardware físico |

## Estructura del Proyecto

```text
SmartPot-DataGenerator/
├── simulator/
│   ├── __main__.py         # Arranca las macetas fijas, el hilo de publicación y la API de control
│   ├── api.py              # API interna (FastAPI) para crear, cambiar y retirar macetas virtuales
│   ├── config.py           # Variables de entorno y lista de macetas fijas
│   ├── device.py           # Cliente MQTT: telemetría, comandos, ACK y estado
│   ├── environment.py      # Modelo físico de la maceta, modos y efecto de los actuadores
│   ├── pots.py             # Administrador de macetas: altas y cambios en caliente, clima y publicación
│   └── weather.py          # Clima actual y buscador de lugares (Open-Meteo)
├── tests/                  # Física, modos, clima, contrato de tópicos, comandos y API de control
├── Dockerfile              # Imagen sin privilegios con uv y chequeo de salud
├── pyproject.toml / uv.lock
└── .env.example
```

## API de Control

Es interna: solo [SmartPot-API](https://github.com/SmartPotTech/SmartPot-API) la consume, con `Authorization: Bearer <SIMULATOR_TOKEN>`, y nunca se publica en Internet. La PWA habla con la API de SmartPot, que comprueba que el cultivo sea de quien lo pide y descifra la clave de la maceta antes de pedir la maceta virtual.

| Método | Ruta | Descripción |
| --- | --- | --- |
| GET | `/health` | Pública: macetas y conexiones |
| GET | `/v1/pots` | Todas las macetas y su estado |
| GET | `/v1/pots/{cropId}` | Estado: modo, última lectura, medidores, clima, actuadores encendidos y último comando |
| PUT | `/v1/pots/{cropId}` | Crea o cambia una maceta: `key`, `cropType`, `mode`, `manual`, `location`, `intervalSeconds` |
| DELETE | `/v1/pots/{cropId}` | Retira la maceta (publica `offline`) |
| GET | `/v1/places?q=` | Busca lugares para el modo clima |
| GET | `/v1/weather?latitude=&longitude=` | Clima actual de un punto |

```json
{
  "key": "<clave de la maceta>",
  "cropType": "LETTUCE",
  "mode": "WEATHER",
  "location": {"name": "Medellín", "latitude": 6.245, "longitude": -75.5715},
  "intervalSeconds": 30
}
```

Las macetas de `SIMULATOR_DEVICES` son fijas (no se pueden cambiar por la API); las que crea la API son administradas: si el simulador se reinicia, la API las vuelve a crear en menos de un minuto.

## Configuración

| Variable | Por defecto | Descripción |
| --- | --- | --- |
| `MQTT_HOST` | `localhost` | Broker |
| `MQTT_PORT` | `1883` (`8883` con TLS) | Puerto |
| `MQTT_TLS` | `false` | Conexión TLS |
| `MQTT_CA_FILE` | — | CA que firma el certificado del broker (`ca.crt`) |
| `MQTT_TOPIC_PREFIX` | `smartpot/v1` | Prefijo del contrato |
| `SIMULATOR_DEVICES` | — | Macetas fijas: `cropId:clave:TIPO` separados por comas (opcional si hay `SIMULATOR_TOKEN`) |
| `SIMULATOR_INTERVAL_SECONDS` | `30` | Segundos entre lecturas de las macetas fijas |
| `SIMULATOR_UTC_OFFSET` | `-5` | Diferencia horaria con UTC para el ciclo de día y noche |
| `SIMULATOR_TOKEN` | — | Token de la API de control (mínimo 24 caracteres); sin él la API responde 503 |
| `SIMULATOR_PORT` | `8081` | Puerto de la API de control |

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

Contra producción, como una maceta más:

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

La imagen corre como el usuario `1000`, admite sistema de archivos de solo lectura y trae chequeo de salud. Necesita salida a Internet solo para el modo clima.

Cada cambio en `main` pasa por el CI, publica la imagen en GHCR (y en Docker Hub como réplica cuando el repositorio tiene credenciales) y pide el despliegue al workflow central de [SmartPotTech/.github](https://github.com/SmartPotTech/.github), que actualiza producción de a uno y verifica `/health`.

## Documentación

El simulador corre siempre junto a la plataforma y solo la API le habla. La [documentación técnica](https://github.com/SmartPotTech/.github/blob/main/docs/SmartPot_Technical_Documentation.md) explica las macetas virtuales, sus modos y cómo las administra la API. Los superdiagramas muestran la plataforma completa en una sola imagen ampliable:

- [Operación completa](https://github.com/SmartPotTech/.github/blob/main/docs/images/superdiagrams/SmartPot_Super_02_Operation_Sequence.svg): la escena de la maceta virtual con clima real y la de rotar la clave
- [Máquinas de estado](https://github.com/SmartPotTech/.github/blob/main/docs/images/superdiagrams/SmartPot_Super_05_State_Machines.svg): los estados de una maceta virtual
- [Arquitectura completa](https://github.com/SmartPotTech/.github/blob/main/docs/images/superdiagrams/SmartPot_Super_01_Architecture.svg): dónde corre el simulador y con quién habla

## Licencia

Este proyecto está bajo la licencia MIT. Consulta el archivo [LICENSE](LICENSE) para más detalles.
