# SmartPot-DataGenerator

[![Rust CI](https://github.com/SmartPotTech/SmartPot-DataGenerator/actions/workflows/ci.yml/badge.svg)](https://github.com/SmartPotTech/SmartPot-DataGenerator/actions/workflows/ci.yml)
[![Publish Docker Images](https://github.com/SmartPotTech/SmartPot-DataGenerator/actions/workflows/packaging.yml/badge.svg)](https://github.com/SmartPotTech/SmartPot-DataGenerator/actions/workflows/packaging.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-00B074.svg)](LICENSE)

[English](#english) | [Español](#español)

---

## English

Virtual hydroponic crop and device simulator for the **SmartPot** platform, rewritten in **Rust** for maximum performance, garbage-collection-free memory safety, and high-efficiency asynchronous concurrency.

It speaks the SmartPot **MQTT v1** contract identically to a physical ESP32 device, and exposes a constant-time protected internal REST API for dynamic runtime crop management.

### Key Features

- **Extreme Performance & Efficiency**: Built on the **Tokio** async runtime and **Axum 0.8**, with a memory footprint under 15 MB RAM and sub-millisecond response times.
- **Strict Memory Safety**: `#![deny(unsafe_code)]` across the entire project. Zero raw pointers or `unsafe` blocks.
- **Side-Channel Protection**: Validates the service token (`SIMULATOR_TOKEN`) using constant-time comparison (`subtle::ConstantTimeEq`) to prevent timing attacks.
- **Strict MQTT v1 Contract**:
  - `smartpot/v1/{cropId}/telemetry`: Periodic QoS 0 publication with physical readings and realistic Gaussian noise (or 27-byte compact binary v2).
  - `smartpot/v1/{cropId}/commands`: Receives and executes actuator commands (`WATER_PUMP`, `UV_LIGHT`, `FAN`, `HUMIDIFIER`, `NUTRIENT_DOSER`, `PH_DOSER`).
  - `smartpot/v1/{cropId}/commands/ack`: Immediate QoS 1 acknowledgement with readable status messages in English.
  - `smartpot/v1/{cropId}/status`: Retained QoS 1 status with LWT (`online`/`offline`).
  - `smartpot/v1/{cropId}/capabilities`: Retained QoS 1 declaration with the **Audit_06 schema 1** payload, published after `status=online` and before subscribing to commands. It reports the firmware, virtual hardware, telemetry formats, the seven sensor ranges, and the six actuators; a publish failure never aborts the command subscription.
- **Continuous Physical Simulation**:
  - Modes: `AUTO` (circadian day/night cycle by time zone), `MANUAL` (configurable gauges), and `WEATHER` (real-world weather integration).
  - In `MANUAL` mode the soil moisture holds its configured setpoint: evaporation is not applied, the water pump or rain can push it above the setpoint, and it then relaxes gently back to that value.
  - Environmental filtering by placement (`INDOOR` / `OUTDOOR`) and solar exposure (`FULL_SUN`, `PARTIAL_SUN`, `SHADE`).
  - Actuator dynamics: cooling, humidification, light radiation, irrigation, evaporation, and chemical dosing.
- **Open-Meteo Integration**: In-memory cache with 10-minute TTL and geographic grid resolution for keyless geocoding and weather forecasts.
- **Minimal & Hardened Container**: Multi-stage image on `debian:bookworm-slim` without privileges (`uid: 1000`, `cap_drop: ALL`, `read_only: true`) and built-in native healthcheck (`--healthcheck`).

### Environment Variables

| Variable                     | Type    | Default                    | Description                                                                   |
|------------------------------|---------|----------------------------|-------------------------------------------------------------------------------|
| `MQTT_HOST`                  | String  | `localhost`                | MQTT broker host (e.g., `broker-smartpot` or `mqtt.smartpot.app`).            |
| `MQTT_PORT`                  | Integer | `1883` (or `8883` w/ TLS)  | MQTT connection port.                                                         |
| `MQTT_TLS`                   | Boolean | `false`                    | Enables TLS for the broker connection.                                        |
| `MQTT_CA_FILE`               | String  | Empty                      | Path to PEM CA certificate (optional).                                        |
| `MQTT_TOPIC_PREFIX`          | String  | `smartpot/v1`              | MQTT topic prefix.                                                            |
| `SIMULATOR_DEVICES`          | String  | Empty                      | Protected static devices formatted as comma-separated `cropId:key:TYPE`.      |
| `SIMULATOR_INTERVAL_SECONDS` | Float   | `30`                       | Base telemetry publication interval in seconds.                               |
| `SIMULATOR_UTC_OFFSET`       | Float   | `-5`                       | UTC hour offset for the solar cycle.                                          |
| `SIMULATOR_TOKEN`            | String  | Empty                      | Bearer token for the control API (minimum 24 characters).                     |
| `SIMULATOR_PORT`             | Integer | `8081`                     | Internal HTTP control API port.                                               |
| `SIMULATOR_PAYLOAD_FORMAT`   | String  | `json`                     | Telemetry format: `json` (contract v1) or `binary` (27-byte compact v2).      |
| `RUST_LOG`                   | String  | `info`                     | Structured log level (`error`, `warn`, `info`, `debug`, `trace`).             |

### Control REST API

The API listens on port `8081` (configurable via `SIMULATOR_PORT`).

- `GET /health`: Public. Container health check and connected crop count.
- `GET /v1/pots`: Requires `Authorization: Bearer <TOKEN>`. Lists full snapshots of all simulated crops.
- `GET /v1/pots/{crop_id}`: Requires Bearer. Returns the snapshot of the specified crop (or 404).
- `PUT /v1/pots/{crop_id}`: Requires Bearer. Creates or updates a dynamic virtual crop. Returns 409 if the crop is static in `SIMULATOR_DEVICES`.
- `DELETE /v1/pots/{crop_id}`: Requires Bearer. Stops and removes a dynamic virtual crop (204 No Content).
- `GET /v1/places?q={name}`: Requires Bearer. Geographic place search via geocoding.
- `GET /v1/weather?latitude={lat}&longitude={lon}`: Requires Bearer. Queries current weather for the given coordinates.

### One-Command Deployment

```bash
curl -fsSL https://raw.githubusercontent.com/SmartPotTech/.github/main/docker/demo/compose.yaml | docker compose -f - up -d --wait
```

### Build and Test

```bash
cargo check
cargo test
cargo clippy --all-targets -- -D warnings
cargo run
```

### Telemetry Formats

- **JSON v1** (default): `{"temperature","humidity","brightness","ph","tds","atmosphere","soilMoisture"}`.
- **27-byte compact v2** with `SIMULATOR_PAYLOAD_FORMAT=binary`, on the same topic: magic `0x50`, version, sequence, epoch, per-sensor presence mask, pressure, 32-bit light and CRC-8.

### Interactive Diagrams

The [interactive diagrams](docs/diagrams/interactive/README.md#english) follow the MQTT session of a virtual crop from `CONNECT` to the command acknowledgement, in English and Spanish, with links to the exact source lines.

---

## Español

Simulador de cultivos y dispositivos hidropónicos virtuales para la plataforma **SmartPot**, reescrito en **Rust** para máximo rendimiento, seguridad de memoria sin recolector de basura (*garbage collection*) y concurrencia asíncrona de alta eficiencia.

Habla el contrato **MQTT v1** de SmartPot exactamente igual que un dispositivo físico ESP32, y expone una API REST interna protegida con tiempo constante (*constant-time comparison*) para administración dinámica de cultivos en tiempo de ejecución.

### Características principales

- **Rendimiento y eficiencia extrema**: Implementado sobre el runtime asíncrono **Tokio** y el framework web **Axum 0.8**, con una huella de memoria inferior a 15 MB RAM y tiempos de respuesta sub-milisegundo.
- **Seguridad de memoria estricta**: `#![deny(unsafe_code)]` en todo el proyecto. Sin punteros crudos ni bloques `unsafe`.
- **Protección contra ataques de canal lateral**: Validación del token de servicio (`SIMULATOR_TOKEN`) mediante comparación en tiempo constante (`subtle::ConstantTimeEq`) para prevenir *timing attacks*.
- **Contrato MQTT v1 estricto**:
  - `smartpot/v1/{cropId}/telemetry`: Publicación periódica QoS 0 con lecturas físicas y ruido gaussiano realista (o paquete binario compacto v2 de 27 bytes).
  - `smartpot/v1/{cropId}/commands`: Recepción y ejecución de órdenes a actuadores (`WATER_PUMP`, `UV_LIGHT`, `FAN`, `HUMIDIFIER`, `NUTRIENT_DOSER`, `PH_DOSER`).
  - `smartpot/v1/{cropId}/commands/ack`: Confirmación inmediata QoS 1 con mensajes legibles en inglés.
  - `smartpot/v1/{cropId}/status`: Estado retenido (*retained*) QoS 1 con LWT (*Last Will and Testament*) `online`/`offline`.
  - `smartpot/v1/{cropId}/capabilities`: Declaración retenida QoS 1 con la carga del **esquema 1 de Audit_06**, publicada después de `status=online` y antes de suscribirse a comandos. Informa el firmware, el hardware virtual, los formatos de telemetría, los rangos de los siete sensores y los seis actuadores; un fallo al publicar nunca cancela la suscripción a comandos.
- **Simulación física continua**:
  - Modos: `AUTO` (ciclo circadiano día/noche según zona horaria), `MANUAL` (medidores configurables) y `WEATHER` (integración con el clima real).
  - En modo `MANUAL` la humedad del sustrato conserva su punto de ajuste: no se aplica evaporación, la bomba de agua o la lluvia pueden empujarla por encima y después vuelve suavemente a ese valor.
  - Filtrado ambiental por ubicación (`INDOOR` / `OUTDOOR`) y exposición solar (`FULL_SUN`, `PARTIAL_SUN`, `SHADE`).
  - Dinámica de actuadores: enfriamiento, humidificación, radiación de luz, riego, evaporación y dosificación química.
- **Integración con Open-Meteo**: Caché en memoria con TTL de 10 minutos y resolución por cuadrícula geográfica para geocodificación y pronósticos meteorológicos sin claves ni cuotas.
- **Contenedor seguro y mínimo**: Imagen multi-etapa basada en `debian:bookworm-slim` sin privilegios (`uid: 1000`, `cap_drop: ALL`, `read_only: true`) y healthcheck nativo integrado (`--healthcheck`).

### Variables de entorno

| Variable | Tipo | Por defecto | Descripción |
| --- | --- | --- | --- |
| `MQTT_HOST` | String | `localhost` | Host del broker MQTT (ej. `broker-smartpot` o `mqtt.smartpot.app`). |
| `MQTT_PORT` | Entero | `1883` (o `8883` con TLS) | Puerto de conexión MQTT. |
| `MQTT_TLS` | Booleano | `false` | Habilita TLS para la conexión al broker. |
| `MQTT_CA_FILE` | String | Vacío | Ruta al certificado CA en formato PEM (opcional). |
| `MQTT_TOPIC_PREFIX` | String | `smartpot/v1` | Prefijo de tópicos MQTT. |
| `SIMULATOR_DEVICES` | String | Vacío | Dispositivos estáticos protegidos en formato `cropId:clave:TIPO` separados por comas. |
| `SIMULATOR_INTERVAL_SECONDS` | Float | `30` | Intervalo base de publicación de lecturas de telemetría. |
| `SIMULATOR_UTC_OFFSET` | Float | `-5` | Diferencia horaria respecto a UTC para el ciclo solar. |
| `SIMULATOR_TOKEN` | String | Vacío | Token Bearer para la API de control (mínimo 24 caracteres). |
| `SIMULATOR_PORT` | Entero | `8081` | Puerto de escucha de la API HTTP interna. |
| `SIMULATOR_PAYLOAD_FORMAT` | String | `json` | Formato de telemetría: `json` (contrato v1) o `binary` (compacto v2 de 27 bytes). |
| `RUST_LOG` | String | `info` | Nivel de logs estructurados (`error`, `warn`, `info`, `debug`, `trace`). |

### API REST de Control

La API escucha en el puerto `8081` (configurable vía `SIMULATOR_PORT`).

- `GET /health`: Público. Chequeo de salud del contenedor y estado de cultivos conectados.
- `GET /v1/pots`: Requiere `Authorization: Bearer <TOKEN>`. Lista el estado completo de todos los cultivos simulados.
- `GET /v1/pots/{crop_id}`: Requiere Bearer. Retorna el snapshot del cultivo indicado (o 404).
- `PUT /v1/pots/{crop_id}`: Requiere Bearer. Crea o actualiza un cultivo virtual dinámico. Retorna 409 si el cultivo es estático de `SIMULATOR_DEVICES`.
- `DELETE /v1/pots/{crop_id}`: Requiere Bearer. Detiene y retira un cultivo virtual dinámico (204 No Content).
- `GET /v1/places?q={nombre}`: Requiere Bearer. Búsqueda de lugares geográficos mediante geocodificación.
- `GET /v1/weather?latitude={lat}&longitude={lon}`: Requiere Bearer. Consulta el clima actual para las coordenadas dadas.

### Despliegue en un comando

```bash
curl -fsSL https://raw.githubusercontent.com/SmartPotTech/.github/main/docker/demo/compose.yaml | docker compose -f - up -d --wait
```

### Compilación y pruebas

```bash
cargo check
cargo test
cargo clippy --all-targets -- -D warnings
cargo run
```

### Formatos de telemetría

- **JSON del contrato v1** (por defecto): `{"temperature","humidity","brightness","ph","tds","atmosphere","soilMoisture"}`.
- **Paquete compacto v2** de 27 bytes con `SIMULATOR_PAYLOAD_FORMAT=binary`, en el mismo tópico: magic `0x50`, versión, secuencia, época, máscara de presencia por sensor, presión, luz de 32 bits y CRC-8.

### Diagramas interactivos

Los [diagramas interactivos](docs/diagrams/interactive/README.md#español) siguen la sesión MQTT de un cultivo virtual desde el `CONNECT` hasta la confirmación del comando, en español e inglés, con enlaces a las líneas exactas del código.
