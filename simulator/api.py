"""API interna de control de los cultivos virtuales. Solo la consume SmartPot-API, con un token compartido;
nunca se publica en Internet. /health es pública para el chequeo del contenedor."""

import secrets
from typing import Annotated, Literal

from fastapi import Depends, FastAPI, HTTPException, Query, Response, status
from fastapi.security import HTTPAuthorizationCredentials, HTTPBearer
from pydantic import BaseModel, ConfigDict, Field
from pydantic.alias_generators import to_camel

from simulator.pots import Location, PotConfig, PotManager

bearer = HTTPBearer(auto_error=False)


class CamelModel(BaseModel):
    model_config = ConfigDict(alias_generator=to_camel, populate_by_name=True)


class ManualValues(CamelModel):
    temperature: float | None = Field(None, ge=-20, le=60)
    humidity: float | None = Field(None, ge=0, le=100)
    brightness: float | None = Field(None, ge=0, le=2000)
    ph: float | None = Field(None, ge=0, le=14)
    tds: float | None = Field(None, ge=0, le=3000)
    atmosphere: float | None = Field(None, ge=300, le=1100)
    soil_moisture: float | None = Field(None, ge=0, le=100)

    def as_dict(self) -> dict[str, float]:
        return {k: v for k, v in self.model_dump(by_alias=True).items() if v is not None}


class LocationIn(CamelModel):
    name: str = Field(min_length=1, max_length=120)
    latitude: float = Field(ge=-90, le=90)
    longitude: float = Field(ge=-180, le=180)


class PotRequest(CamelModel):
    key: str = Field(min_length=16, max_length=128)
    crop_type: str = Field(pattern=r"^[A-Z_]{3,20}$")
    mode: Literal["AUTO", "MANUAL", "WEATHER"] = "WEATHER"
    manual: ManualValues | None = None
    location: LocationIn | None = None
    interval_seconds: float = Field(30, ge=10, le=300)
    setting: Literal["INDOOR", "OUTDOOR"] | None = None
    exposure: Literal["FULL_SUN", "PARTIAL_SUN", "SHADE"] | None = None


def create_app(manager: PotManager, token: str | None) -> FastAPI:
    app = FastAPI(title="SmartPot Simulator", version="2.0.0", docs_url=None, redoc_url=None, openapi_url=None,
                  description="Cultivos virtuales que hablan MQTT v1 como un dispositivo real.")

    def require_token(credentials: Annotated[HTTPAuthorizationCredentials | None, Depends(bearer)]) -> None:
        if token is None:
            raise HTTPException(status.HTTP_503_SERVICE_UNAVAILABLE, "La API de control está deshabilitada")
        if credentials is None or not secrets.compare_digest(credentials.credentials, token):
            raise HTTPException(status.HTTP_401_UNAUTHORIZED, "Token de servicio inválido",
                                headers={"WWW-Authenticate": "Bearer"})

    guarded = [Depends(require_token)]

    @app.get("/health")
    def health() -> dict:
        pots = manager.all()
        return {"status": "UP", "pots": len(pots), "connected": sum(1 for p in pots if p.device.connected),
                "api": "ENABLED" if token else "DISABLED"}

    @app.get("/v1/pots", dependencies=guarded)
    def list_pots() -> list[dict]:
        return [manager.snapshot(pot) for pot in manager.all()]

    @app.get("/v1/pots/{crop_id}", dependencies=guarded)
    def get_pot(crop_id: str) -> dict:
        pot = manager.get(crop_id)
        if pot is None:
            raise HTTPException(status.HTTP_404_NOT_FOUND, "El cultivo virtual no existe")
        return manager.snapshot(pot)

    @app.put("/v1/pots/{crop_id}", dependencies=guarded)
    def put_pot(crop_id: str, payload: PotRequest) -> dict:
        existing = manager.get(crop_id)
        if existing is not None and not existing.config.managed:
            raise HTTPException(status.HTTP_409_CONFLICT, "Ese cultivo está definido en SIMULATOR_DEVICES")
        location = None if payload.location is None else Location(**payload.location.model_dump())
        config = PotConfig(crop_id=crop_id, key=payload.key, crop_type=payload.crop_type, mode=payload.mode,
                           manual=payload.manual.as_dict() if payload.manual else {}, location=location,
                           interval_seconds=payload.interval_seconds, setting=payload.setting,
                           exposure=payload.exposure)
        try:
            return manager.snapshot(manager.upsert(config))
        except ValueError as error:
            raise HTTPException(status.HTTP_400_BAD_REQUEST, str(error)) from error

    @app.delete("/v1/pots/{crop_id}", dependencies=guarded, status_code=204)
    def delete_pot(crop_id: str) -> Response:
        pot = manager.get(crop_id)
        if pot is not None and not pot.config.managed:
            raise HTTPException(status.HTTP_409_CONFLICT, "Ese cultivo está definido en SIMULATOR_DEVICES")
        manager.remove(crop_id)
        return Response(status_code=204)

    @app.get("/v1/places", dependencies=guarded)
    def places(q: Annotated[str, Query(min_length=2, max_length=80)]) -> list[dict]:
        try:
            return [place.as_dict() for place in manager.weather_client.search(q)]
        except Exception as error:  # noqa: BLE001 - el buscador externo puede fallar
            raise HTTPException(status.HTTP_502_BAD_GATEWAY, "No se pudo consultar el buscador de lugares") \
                from error

    @app.get("/v1/weather", dependencies=guarded)
    def weather(latitude: Annotated[float, Query(ge=-90, le=90)],
                longitude: Annotated[float, Query(ge=-180, le=180)]) -> dict:
        try:
            return manager.weather_client.current(latitude, longitude).as_dict()
        except Exception as error:  # noqa: BLE001 - el servicio de clima puede fallar
            raise HTTPException(status.HTTP_502_BAD_GATEWAY, "No se pudo consultar el clima") from error

    return app
