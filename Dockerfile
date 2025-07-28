# Этап 0: Сборщик зависимостей
# Этот этап пересобирается только при изменении Cargo.toml или Cargo.lock.
# Используем конкретную версию Alpine для стабильности и меньшего размера.
FROM rust:latest AS dependencies_builder
WORKDIR /app

# Создаем фиктивный проект, чтобы cargo мог скачать и собрать зависимости.
# Это стандартный трюк для эффективного кэширования.
RUN cargo init --bin

# Копируем только файлы манифеста. Слой кэшируется, если они не изменились.
COPY Cargo.toml Cargo.lock ./

# Собираем только зависимости, без кода приложения.
RUN cargo build --release --locked


# Этап 1: Сборщик приложения
# Этот этап использует кэш зависимостей и собирает финальный бинарник.
FROM rust:latest AS builder
WORKDIR /app

# Копируем кэш зависимостей из предыдущего этапа.
COPY --from=dependencies_builder /usr/local/cargo/registry /usr/local/cargo/registry
COPY --from=dependencies_builder /app/target /app/target

# Копируем весь исходный код приложения.
COPY . .

# Собираем приложение. Это будет быстро, так как зависимости уже скомпилированы.
RUN cargo build --release --locked


# Этап 2: Финальный образ
# Создаем минимальный и безопасный образ для запуска.
FROM alpine:latest
WORKDIR /app

# Устанавливаем curl для HEALTHCHECK.
RUN apk --no-cache add curl

# Копируем только скомпилированный бинарник из этапа 'builder'.
COPY --from=builder /app/target/release/gemini-proxy-key-rotation-rust .

# Копируем необходимые статические файлы и конфигурацию.
COPY static ./static
COPY config.example.yaml .

# Открываем порт, на котором будет работать приложение.
EXPOSE 8080

# Добавляем проверку состояния, чтобы Docker мог отслеживать работоспособность.
# Проверяем, что главный эндпоинт отдает статус 200.
HEALTHCHECK --interval=30s --timeout=3s --start-period=5s --retries=3 \
  CMD curl -f http://localhost:8080/ || exit 1

# Команда для запуска приложения.
CMD ["./gemini-proxy-key-rotation-rust"]
# Этап 3: Линтер
# Используется для запуска clippy в CI/CD или локально.
FROM builder AS linter
WORKDIR /app
RUN rustup component add clippy
RUN cargo clippy --all-targets --all-features -- -D warnings

# Этап 4: Тестер
# Используется для запуска тестов в CI/CD.
FROM builder AS tester
WORKDIR /app
RUN cargo test --all-features

# Этап 5: Генератор отчета о покрытии
# Используется для генерации отчета о покрытии кода тестами.
FROM builder AS coverage_runner
WORKDIR /app
# Устанавливаем tarpaulin. Фиксируем версию для воспроизводимости.
RUN cargo install cargo-tarpaulin --version 0.27.1
# Запускаем tarpaulin для генерации отчета в формате LCOV (стандарт для многих CI/CD систем)
RUN cargo tarpaulin --all-features --out Lcov