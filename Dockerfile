# Этап 0: Сборщик зависимостей (deps)
# Этот этап кеширует зависимости Rust. Пересобирается только при изменении Cargo.toml/lock.
# Используем конкретную slim-версию для воспроизводимости и меньшего размера.
FROM rust:1.78-slim-bookworm AS deps

# Устанавливаем зависимости для статической линковки с musl и очищаем кеш apt в одном слое.
RUN apt-get update && apt-get install -y --no-install-recommends \
    musl-tools \
    build-essential \
    pkg-config \
    && rm -rf /var/lib/apt/lists/*

# Добавляем musl target для статической сборки.
RUN rustup target add x86_64-unknown-linux-musl

WORKDIR /app

# Копируем только файлы манифеста.
COPY Cargo.toml Cargo.lock ./

# Создаем "пустышку" для сборки только зависимостей.
# Это позволяет кешировать этот слой, пока Cargo.toml/lock не изменятся.
RUN mkdir src && echo "fn main() {}" > src/main.rs && \
    cargo build --release --target x86_64-unknown-linux-musl --locked

# ---

# Этап 1: Сборщик приложения (builder)
# Этот этап компилирует приложение, используя кешированные зависимости.
# Наследуемся от `deps`, чтобы не переустанавливать все инструменты.
FROM deps AS builder

WORKDIR /app

# Копируем уже скомпилированные зависимости из предыдущего этапа.
COPY --from=deps /app/target ./target
# Копируем исходный код. Этот слой будет пересобираться только при изменении кода в `src`.
COPY src ./src

# Собираем финальный бинарный файл.
# Это будет очень быстро, так как все зависимости уже скомпилированы.
RUN cargo build --release --target x86_64-unknown-linux-musl --locked

# ---

# Этап 2: Финальный образ
# Это минимальный и безопасный образ для запуска в production.
FROM alpine:3.19.1

# Устанавливаем ca-certificates для HTTPS-запросов и сразу очищаем кеш.
RUN apk --no-cache add ca-certificates

WORKDIR /app

# Создаем пользователя и группу без root-прав для запуска приложения.
RUN addgroup -S appgroup && adduser -S appuser -G appgroup

# Копируем скомпилированный бинарный файл из этапа 'builder'.
COPY --from=builder /app/target/x86_64-unknown-linux-musl/release/gemini-proxy-key-rotation-rust .
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