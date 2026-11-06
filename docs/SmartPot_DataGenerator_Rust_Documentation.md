<!-- portada
eyebrow: Component Documentation · Documentación del componente
titulo: SmartPot-DataGenerator
acento: DataGenerator
subtitulo: High-Performance SmartPot Simulator in Rust · El simulador de alto rendimiento de SmartPot en Rust
bajada: High-performance, low-memory virtual crops speaking the MQTT v1 contract: physical model, Tokio runtime, Axum 0.8, rumqttc, strict memory safety, and internal control API. / Cultivos virtuales de alto rendimiento y bajo consumo que hablan el contrato MQTT v1: modelo físico, Tokio runtime, Axum 0.8, rumqttc, seguridad de memoria estricta y API de control interna.
documento: SmartPot-DataGenerator
version: 2.0 · November 2026 / noviembre 2026
equipo: SmartPotTech
proyecto: smartpot.app
-->

# SmartPot-DataGenerator

[English](#english) | [Español](#español)

---

# English

## Document Sheet

| Field              | Value                                                                                                                               |
|--------------------|-------------------------------------------------------------------------------------------------------------------------------------|
| Project            | SmartPot · [smartpot.app](https://smartpot.app)                                                                                     |
| Component          | [SmartPot-DataGenerator](https://github.com/SmartPotTech/SmartPot-DataGenerator)                                          |
| Version            | 2.0.0 · November 2026                                                                                                                |
| Language & Runtime | Rust 1.85+ · Tokio · Axum 0.8 · rumqttc                                                                                             |
| Scope              | Virtual crops, static demo/QA crops, agronomic physical model, MQTT capabilities, internal REST control API, memory safety, tests, and deployment |
| Status             | Canonical production simulator (Stack B). Legacy Python simulator archived at SmartPot-DataGenerator-Python                         |

---

## 1. Purpose and Migration Motivation

### In Simple Terms

The simulator brings **virtual crops** to life: crops that a user creates in SmartPot without physical hardware. It runs continuously as a platform support service and performs the exact same job as a physical ESP32 microcontroller:
1. Publishes periodic telemetry over MQTT using the crop's credentials.
2. Listens for actuator commands and executes them on the physical model.
3. Responds immediately with readable acknowledgements (*ACK*) over MQTT.
4. Maintains the retained connection status (`online` / `offline`) via LWT.

The rewrite in **Rust** introduces:
- **Zero-cost abstractions** and cooperative concurrency on Tokio: memory footprint < 15 MB RAM.
- **Strict memory safety** with `#![deny(unsafe_code)]`.
- **Timing attack protection** using `subtle::ConstantTimeEq` to validate the Bearer token.
- **Built-in native healthcheck** in the binary (`--healthcheck`), eliminating shell or external tool dependencies inside the container.

---

## 2. Internal Architecture

The component is divided into decoupled modular layers:

1. **Configuration (`config.rs`)**: Loads and validates environment variables (`Settings`, `DeviceConfig`).
2. **Agronomic Domain (`domain/`)**:
   - `crop.rs`: Baseline species profiles (Lettuce, Tomato, Strawberry, Basil, Spinach, Pepper).
   - `limits.rs`: Physical sensor measurement bounds and clamping functions.
   - `actuator.rs`: Actuator types and human-readable durations ("15 s", "10 min", "2 h").
   - `environment.rs`: Physical simulation engine with target calculation, exponential relaxation, evaporation, irrigation, and Gaussian noise.
   - `weather.rs`: Open-Meteo client with WMO code mapping and 10-minute TTL in-memory geographic grid cache.
3. **Simulated Device (`device.rs`)**: MQTT client connection via `rumqttc`, async event loop, retained capabilities publication, command subscription, and telemetry publication (JSON v1 and 27-byte compact binary v2).
4. **Crop Manager (`pots.rs`)**: `PotManager` manages `Arc<RwLock<VirtualPot>>`, orchestrates the publication clock, and interacts with the weather service.
5. **HTTP Control API (`api.rs`)**: Axum server with `/health`, `/v1/pots`, `/v1/places`, and `/v1/weather` routes, protected by constant-time Bearer authentication.

**Dependency hygiene**: the runtime dependency set no longer includes `async-trait`, `tower-http` or `thiserror`; they became unused after the Axum rewrite. `tower` remains a dev-dependency only, used by the router tests through `oneshot`, so the release image and compile graph stay smaller.

---

## 3. Physical Simulation Model

The physical engine in `Environment::step(seconds, now)` updates the hydroponic state variables:
- **Temperature and humidity**: Smoothly relax toward the target environment; the fan (`FAN`) cools (-0.35°C/min) and dehumidifies (-1.5%/min); the humidifier (`HUMIDIFIER`) adds moisture (+2.5%/min).
- **Sunlight and artificial light**: In `AUTO` mode, follows a day/night sinusoidal curve with UTC offset. The UV lamp (`UV_LIGHT`) adds light radiation (+900 lx) and a slight thermal increase.
- **Substrate moisture (AUTO/WEATHER)**: Models evaporation as a function of temperature and ambient drying factor, offset by water pump irrigation (`WATER_PUMP`, +1.2%/s) or outdoor rain.
- **Chemical evolution**: pH dosing (`PH_DOSER`) lowers pH by 0.12 units per dosed second; the nutrient doser (`NUTRIENT_DOSER`) increases TDS by 70 ppm per dosed second.

**MANUAL substrate moisture**: in `MANUAL` mode, `Targets.soil_moisture` carries the configured setpoint. Evaporation is not applied; the water pump (or rain, when available) pushes moisture above the setpoint, and with no irrigation each step relaxes it back toward the setpoint with `rate = min(0.1 * minutes, 1.0)`. `AUTO` and `WEATHER` keep the physical evaporation plus irrigation model.

---

## 4. Capabilities Topic

After `status=online` and before subscribing to `.../commands`, each simulated device publishes a retained QoS 1 message on `.../capabilities`. It follows schema 1 and declares:

- firmware `smartpot-datagenerator/2.0.0` and hardware `virtual`;
- telemetry formats `json-v1` and `compact-v2` (27 bytes);
- seven sensors: `temperature`, `humidity`, `atmosphere`, `brightness`, `ph`, `tds` and `soilMoisture`, with API-aligned ranges;
- six actuators: `WATER_PUMP`, `UV_LIGHT`, `FAN`, `HUMIDIFIER`, `NUTRIENT_DOSER` and `PH_DOSER`.

The payload stays under 2 KB. A publish failure is logged and never blocks the command subscription, so the v1 contract keeps working.

---

## 5. Tests and Validation

The component includes a comprehensive unit and integration test suite:
- `tests/test_environment.rs` (12 tests): Sensor scale bounds, fan cooling, night-time UV lighting, chemical dosing, weather filtering by solar exposure, and the `MANUAL` soil setpoint that holds, is pushed by irrigation and relaxes back.
- `tests/test_device.rs` (6 tests): MQTT v1 contract, schema 1 capabilities, command validation, invalid command discarding, and device list parsing.
- `tests/test_virtual_pots.rs` (7 tests): Weather scenes, coordinate cache, static crop protection, control API authorization, and full virtual crop CRUD lifecycle.
- `src/device.rs` unit test (1): compact v2 presence mask, epoch, sequence and CRC-8.

The suite has 26 tests and runs natively with `cargo test`.

---

# Español

## Ficha del documento

| Campo | Valor |
| --- | --- |
| Proyecto | SmartPot · [smartpot.app](https://smartpot.app) |
| Componente | [SmartPot-DataGenerator](https://github.com/SmartPotTech/SmartPot-DataGenerator) |
| Versión | 2.0.0 · noviembre 2026 |
| Lenguaje y runtime | Rust 1.85+ · Tokio · Axum 0.8 · rumqttc |
| Alcance | Cultivos virtuales, cultivos fijos para demo y QA, modelo físico agronómico, capacidades MQTT, API de control REST interna, seguridad de memoria, pruebas y despliegue |
| Estado | Simulador canónico en producción (Stack B). Implementación histórica en Python archivada en SmartPot-DataGenerator-Python |

---

## 1. Propósito y motivación de la migración

### En palabras simples

El simulador da vida a los **cultivos virtuales**: los que un usuario crea en SmartPot sin hardware. Corre permanentemente como servicio de soporte en la plataforma y realiza el mismo trabajo que un microcontrolador ESP32 físico:
1. Publica telemetría periódica por MQTT con las credenciales del cultivo.
2. Escucha comandos hacia actuadores y los ejecuta en el modelo físico.
3. Responde inmediatamente con confirmaciones legibles (*ACK*) por MQTT.
4. Mantiene el estado retenido de conexión (`online` / `offline`) mediante LWT.

La versión original en Python 3.13 cumplía la funcionalidad básica pero presentaba limitaciones de rendimiento y consumo:
- Bloqueos de intérprete por GIL (*Global Interpreter Lock*) y multihilo tradicional.
- Alto consumo de memoria RAM (80-120 MB en reposo).
- Sobrecarga de deserialización y sincronización.

La reescritura en **Rust** introduce:
- **Zero-cost abstractions** y concurrencia cooperativa sobre Tokio: huella de memoria < 15 MB RAM.
- **Seguridad de memoria estricta** con `#![deny(unsafe_code)]`.
- **Protección contra ataques de temporización** (*timing attacks*) usando `subtle::ConstantTimeEq` para validar el token Bearer.
- **Healthcheck nativo** integrado en el binario (`--healthcheck`) eliminando dependencias de shell o herramientas externas en el contenedor.

---

## 2. Arquitectura interna

El componente se divide en capas modulares desacopladas:

1. **Configuración (`config.rs`)**: Carga y valida variables de entorno (`Settings`, `DeviceConfig`).
2. **Dominio agronómico (`domain/`)**:
   - `crop.rs`: Perfiles basales de especies (Lechuga, Tomate, Fresa, Albahaca, Espinaca, Pimentón).
   - `limits.rs`: Límites de medición física de los sensores y funciones de acotamiento (*clamping*).
   - `actuator.rs`: Tipos de actuadores y duraciones legibles ("15 s", "10 min", "2 h").
   - `environment.rs`: Motor de simulación física con cálculo de objetivos, relajación exponencial, evaporación, riego y ruido gaussiano.
   - `weather.rs`: Cliente Open-Meteo con mapeo de códigos WMO y caché geográfica en memoria por cuadrícula con TTL de 10 minutos.
3. **Dispositivo simulado (`device.rs`)**: Conexión MQTT cliente mediante `rumqttc`, bucle de eventos asíncrono, publicación retenida de capacidades, suscripción a órdenes y publicación de telemetría.
4. **Gestor de cultivos (`pots.rs`)**: `PotManager` administra `Arc<RwLock<VirtualPot>>`, orquesta el reloj de publicación e interactúa con el servicio de clima.
5. **API de control HTTP (`api.rs`)**: Servidor Axum con rutas `/health`, `/v1/pots`, `/v1/places` y `/v1/weather`, protegido por autenticación Bearer en tiempo constante.

**Higiene de dependencias**: el conjunto de dependencias de ejecución ya no incluye `async-trait`, `tower-http` ni `thiserror`; quedaron sin uso tras la reescritura con Axum. `tower` sigue solo como dependencia de desarrollo, usada por las pruebas del router mediante `oneshot`, así la imagen de release y el grafo de compilación son más pequeños.

---

## 3. Modelo físico de simulación

El motor físico en `Environment::step(seconds, now)` actualiza las variables de estado hidropónico:
- **Temperatura y humedad**: Relajan suavemente hacia el ambiente objetivo; el ventilador (`FAN`) enfría (-0.35°C/min) y deshumidifica (-1.5%/min); el humidificador (`HUMIDIFIER`) aporta humedad (+2.5%/min).
- **Luz solar y artificial**: En modo `AUTO`, sigue una curva senoidal día/noche con desfase UTC. La lámpara ultravioleta (`UV_LIGHT`) suma radiación lumínica (+900 lx) y un ligero incremento térmico.
- **Humedad del sustrato (AUTO/WEATHER)**: Modela la evaporación en función de la temperatura y el factor de secado ambiental, compensada por el riego de la bomba (`WATER_PUMP`, +1.2%/s) o la lluvia en cultivos al aire libre.
- **Evolución química**: La dosificación de pH (`PH_DOSER`) reduce 0.12 unidades por segundo dosificado; el dosificador de nutrientes (`NUTRIENT_DOSER`) incrementa 70 ppm por segundo de dosis.

**Humedad del sustrato en `MANUAL`**: en modo `MANUAL`, `Targets.soil_moisture` lleva el punto de ajuste configurado. No se aplica evaporación; la bomba de agua (o la lluvia, cuando existe) empuja la humedad por encima del punto de ajuste y, sin riego, cada paso la relaja de vuelta hacia ese valor con `rate = min(0.1 * minutes, 1.0)`. `AUTO` y `WEATHER` conservan el modelo físico de evaporación más riego.

---

## 4. Tópico de capacidades

Después de `status=online` y antes de suscribirse a `.../commands`, cada dispositivo simulado publica un mensaje retenido QoS 1 en `.../capabilities`. Sigue el esquema 1 y declara:

- firmware `smartpot-datagenerator/2.0.0` y hardware `virtual`;
- formatos de telemetría `json-v1` y `compact-v2` (27 bytes);
- siete sensores: `temperature`, `humidity`, `atmosphere`, `brightness`, `ph`, `tds` y `soilMoisture`, con rangos alineados con la API;
- seis actuadores: `WATER_PUMP`, `UV_LIGHT`, `FAN`, `HUMIDIFIER`, `NUTRIENT_DOSER` y `PH_DOSER`.

La carga ocupa menos de 2 KB. Un fallo al publicar se registra y nunca bloquea la suscripción a comandos, así el contrato v1 sigue funcionando.

---

## 5. Pruebas y validación

El componente cuenta con una suite completa de pruebas unitarias e integración:
- `tests/test_environment.rs` (12 pruebas): Verificación de escalas de sensores, enfriamiento por ventilador, iluminación nocturna por UV, dosificaciones, filtrado de clima por exposición solar y el punto de ajuste de sustrato en `MANUAL`, que se conserva, sube con el riego y vuelve a relajarse.
- `tests/test_device.rs` (6 pruebas): Contrato MQTT v1, capacidades con esquema 1, validación de comandos, descarte de comandos inválidos y parsing de listas de dispositivos.
- `tests/test_virtual_pots.rs` (7 pruebas): Escenas climáticas, caché de coordenadas, protección de cultivos estáticos, autorización de la API de control y ciclo completo CRUD de cultivos virtuales.
- Prueba unitaria de `src/device.rs` (1): máscara de presencia, época, secuencia y CRC-8 del paquete compacto v2.

La suite tiene 26 pruebas y se ejecuta de forma nativa con `cargo test`.
