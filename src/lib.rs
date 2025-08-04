// src/lib.rs

// --- Модули ---
// Примечание: модуль `handler` был переименован в `handlers` для лучшего соответствия
// общепринятым практикам именования (модуль, содержащий несколько обработчиков).
pub mod admin;
pub mod circuit_breaker;
pub mod config;
pub mod error;
pub mod handlers; // <-- Переименовано с `handler`
pub mod key_manager;
pub mod metrics;
pub mod middleware;
pub mod proxy;
pub mod state;
pub mod tokenizer;

// --- Зависимости и пере-экспорты ---
use crate::handlers::{health_check, proxy_handler};
use axum::{
    body::Body,
    http::{HeaderValue, Request as AxumRequest},
    response::IntoResponse,
    routing::{any, get},
    Router,
};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::Instant,
};
use tower_cookies::CookieManagerLayer;
use tracing::{error, info, info_span, Instrument};
use uuid::Uuid;

// Пере-экспорт ключевых типов для удобства использования
pub use config::AppConfig;
pub use error::{AppError, Result};
pub use state::AppState;

/// Создает основной роутер Axum для приложения.
pub fn create_router(state: Arc<AppState>) -> Router {
    // Объединяем маршруты прокси для уменьшения дублирования
    let proxy_routes = [
        "/v1/*path",
        "/v1beta/*path",
        "/chat/*path",
        "/embeddings",
        "/models",
    ];

    let mut router = Router::new()
        .route("/health", get(health_check))
        .route("/metrics", get(metrics::metrics_handler))
        .merge(admin::admin_routes(state.clone()));

    for path in proxy_routes {
        router = router.route(path, any(proxy_handler));
    }

    router.layer(CookieManagerLayer::new()).with_state(state)
}

/// Middleware для добавления Request ID и трассировки запросов.
async fn trace_requests(
    mut req: AxumRequest<Body>,
    next: axum::middleware::Next,
) -> impl IntoResponse {
    let request_id = Uuid::new_v4();
    let start_time = Instant::now();
    let method = req.method().clone();
    let path = req.uri().path().to_string();

    // Создаем span для трассировки запроса с полезными полями
    let span = info_span!(
        "request",
        request_id = %request_id,
        http.method = %method,
        url.path = %path,
    );

    // Добавляем request_id в расширения запроса для доступа в других обработчиках
    req.extensions_mut().insert(request_id);

    // Выполняем запрос внутри span
    async move {
        let mut response = next.run(req).await;
        let elapsed = start_time.elapsed();

        // Добавляем заголовок X-Request-ID в ответ
        response.headers_mut().insert(
            "X-Request-ID",
            HeaderValue::from_str(&request_id.to_string()).unwrap(),
        );

        // Логируем завершение обработки запроса с кодом ответа и длительностью
        info!(
            http.response.duration = ?elapsed,
            http.status_code = response.status().as_u16(),
            "Finished processing request"
        );

        response
    }
    .instrument(span)
    .await
}

/// Основная функция настройки приложения, отвечающая за конфигурацию,
/// инициализацию состояния и создание роутера.
pub async fn run(
    config_path_override: Option<PathBuf>,
) -> std::result::Result<(Router, AppConfig), AppError> {
    info!("Starting Gemini API Key Rotation Proxy...");

    // 1. Настройка конфигурации
    let (app_config, config_path) = setup_configuration(config_path_override)?;

    // 2. Инициализация состояния приложения
    let (app_state, mut config_update_rx) =
        build_application_state(&app_config, &config_path).await?;

    // 3. Запуск фонового обработчика для обновления конфигурации
    let state_for_worker = app_state.clone();
    tokio::spawn(async move {
        loop {
            match config_update_rx.recv().await {
                Ok(new_config) => {
                    info!("Received configuration update message. Reloading state...");
                    // Эта логика перенесена из `modify_config_and_reload`
                    if let Err(e) =
                        admin::reload_state_from_config(state_for_worker.clone(), new_config).await
                    {
                        error!("Failed to reload state in background worker: {:?}", e);
                    }
                }
                Err(e) => {
                    error!("Config update channel error: {}. Worker terminating.", e);
                    break;
                }
            }
        }
    });

    // 4. Настройка роутера и middleware
    let app = create_router(app_state)
        .layer(axum::middleware::from_fn(crate::middleware::request_size_limit_middleware))
        .layer(axum::middleware::from_fn(trace_requests));

    Ok((app, app_config))
}

/// Загружает, валидирует и логирует конфигурацию приложения.
fn setup_configuration(config_path_override: Option<PathBuf>) -> Result<(AppConfig, PathBuf)> {
    let config_path = config_path_override.unwrap_or_else(|| {
        std::env::var("CONFIG_PATH").map_or_else(|_| PathBuf::from("config.yaml"), PathBuf::from)
    });

    let config_path_display = config_path.display().to_string();
    if config_path.exists() {
        info!(config.path = %config_path_display, "Using configuration file");
    } else {
        info!(config.path = %config_path_display, "Optional configuration file not found. Using defaults and environment variables.");
    }

    let app_config = config::load_config(&config_path).map_err(|e| {
        error!(
            config.path = %config_path_display,
            error = ?e,
            "Failed to load or validate configuration. Exiting."
        );
        e
    })?;

    let total_keys: usize = app_config.groups.iter().map(|g| g.api_keys.len()).sum();
    let group_names: Vec<String> = app_config.groups.iter().map(|g| g.name.clone()).collect();
    info!(
         config.groups.count = app_config.groups.len(),
         config.groups.names = ?group_names,
         config.total_keys = total_keys,
         server.port = app_config.server.port,
         "Configuration loaded and validated successfully."
    );

    Ok((app_config, config_path))
}

/// Создает и инициализирует состояние приложения, включая подключение к Redis.
async fn build_application_state(
    app_config: &AppConfig,
    config_path: &Path,
) -> Result<(Arc<AppState>, tokio::sync::broadcast::Receiver<AppConfig>)> {
    // Примечание: Логика создания пула Redis теперь должна быть внутри `AppState::new`.
    // Это улучшает инкапсуляцию, так как AppState управляет своими собственными зависимостями.
    // Функция `run` больше не должна беспокоиться о деталях создания пула.

    let (app_state, rx) = AppState::new(app_config, config_path).await.map_err(|e| {
        error!(error = ?e, "Failed to initialize application state. Exiting.");
        e
    })?;

    info!("Application state initialized successfully.");
    if app_config.redis_url.is_some() {
        info!("Redis persistence is enabled.");
    } else {
        info!("Running without Redis persistence.");
    }

    Ok((Arc::new(app_state), rx))
}
