# Этап 0: Сборщик зависимостей (deps)
# Этот этап кеширует зависимости Rust. Пересобирается только при изменении Cargo.toml/lock.
FROM rust:latest AS deps

# Устанавливаем зависимости для сборки.
RUN apt-get update && apt-get install -y --no-install-recommends \
    build-essential \
    pkg-config \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Копируем только файлы манифеста.
COPY Cargo.toml Cargo.lock ./

# Создаем "пустышку" для сборки только зависимостей.
RUN mkdir src && echo "fn main() {}" > src/main.rs && \
    cargo build --release --locked

# ---

# Этап 1: Сборщик приложения (builder)
# Этот этап компилирует приложение, используя кешированные зависимости.
FROM rust:latest AS builder

WORKDIR /app

# Копируем уже скомпилированные зависимости из предыдущего этапа.
COPY --from=deps /app/target ./target
COPY --from=deps /usr/local/cargo /usr/local/cargo

# Копируем исходный код.
COPY . .

# Собираем финальный бинарный файл.
RUN cargo build --release --locked

# ---

# Этап 2: Финальный образ
# Это минимальный и безопасный образ для запуска в production.
FROM debian:stable-slim

# Устанавливаем ca-certificates для HTTPS-запросов и сразу очищаем кеш.
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Создаем пользователя и группу без root-прав для запуска приложения.
RUN addgroup --system appgroup && adduser --system --ingroup appgroup appuser

# Копируем скомпилированный бинарный файл из этапа 'builder'.
COPY --from=builder /app/target/release/gemini-proxy-key-rotation-rust .
# Копируем необходимые статические файлы и конфигурацию.
COPY config.example.yaml .
COPY static ./static

# Устанавливаем владельца для всех файлов приложения.
RUN chown -R appuser:appgroup /app

# Переключаемся на пользователя без root-прав.
USER appuser

# Открываем порт, на котором будет работать приложение.
EXPOSE 8080

# Команда для запуска приложения.
CMD ["./gemini-proxy-key-rotation-rust"]