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
      org.opencontainers.image.description="Macetas virtuales de SmartPot por MQTT: automáticas, manuales o con el clima real" \
      org.opencontainers.image.source="https://github.com/SmartPotTech/SmartPot-DataGenerator" \
      org.opencontainers.image.licenses="MIT"

ENV PATH="/app/.venv/bin:$PATH" \
    PYTHONDONTWRITEBYTECODE=1 \
    PYTHONUNBUFFERED=1 \
    SIMULATOR_PORT=8081

WORKDIR /app

COPY --from=build /app /app

USER 1000:1000

# API interna de control de las macetas virtuales: solo la consume SmartPot-API.
EXPOSE 8081

HEALTHCHECK --interval=30s --timeout=5s --start-period=20s --retries=3 \
    CMD python -c "import os, urllib.request; urllib.request.urlopen(f'http://127.0.0.1:{os.environ[\"SIMULATOR_PORT\"]}/health', timeout=4)"

CMD ["python", "-m", "simulator"]
