FROM python:3.13-slim-bookworm AS build

COPY --from=ghcr.io/astral-sh/uv:0.12.12 /uv /bin/uv

ENV UV_COMPILE_BYTECODE=1 \
    UV_LINK_MODE=copy \
    UV_PYTHON_DOWNLOADS=never

WORKDIR /app

COPY pyproject.toml uv.lock .python-version ./
RUN uv sync --frozen --no-dev --no-install-project

COPY simulator ./simulator

FROM python:3.13-slim-bookworm

LABEL org.opencontainers.image.title="SmartPot DataGenerator" \
      org.opencontainers.image.description="Simulador de macetas SmartPot por MQTT" \
      org.opencontainers.image.source="https://github.com/SmartPotTech/SmartPot-DataGenerator" \
      org.opencontainers.image.licenses="MIT"

ENV PATH="/app/.venv/bin:$PATH" \
    PYTHONDONTWRITEBYTECODE=1 \
    PYTHONUNBUFFERED=1

WORKDIR /app

COPY --from=build /app /app

USER 1000:1000

CMD ["python", "-m", "simulator"]
