// src/monitoring/key_health.rs

use crate::error::{AppError, Result};
use crate::key_manager::{KeyManagerTrait};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use tokio::time::interval;
use tracing::{debug, error, info, warn};

/// Система мониторинга здоровья ключей
#[derive(Clone)]
pub struct KeyHealthMonitor {
    key_manager: Arc<dyn KeyManagerTrait>,
    health_stats: Arc<RwLock<HashMap<String, KeyHealthStats>>>,
    check_interval: Duration,
    recovery_threshold: Duration,
}

#[derive(Debug, Clone)]
pub struct KeyHealthStats {
    pub key_preview: String,
    pub group_name: String,
    pub total_requests: u64,
    pub successful_requests: u64,
    pub failed_requests: u64,
    pub consecutive_failures: u32,
    pub last_success: Option<Instant>,
    pub last_failure: Option<Instant>,
    pub average_response_time: Duration,
    pub health_score: f64, // 0.0 - 1.0
    pub is_healthy: bool,
    pub recovery_attempts: u32,
}

impl KeyHealthMonitor {
    pub fn new(key_manager: Arc<dyn KeyManagerTrait>) -> Self {
        Self {
            key_manager,
            health_stats: Arc::new(RwLock::new(HashMap::new())),
            check_interval: Duration::from_secs(30),
            recovery_threshold: Duration::from_secs(300), // 5 минут
        }
    }

    /// Запускает фоновый мониторинг
    pub async fn start_monitoring(&self) {
        let mut interval = interval(self.check_interval);
        let health_stats = Arc::clone(&self.health_stats);
        let key_manager = Arc::clone(&self.key_manager);
        let recovery_threshold = self.recovery_threshold;

        tokio::spawn(async move {
            loop {
                interval.tick().await;
                
                if let Err(e) = Self::perform_health_check(
                    &key_manager,
                    &health_stats,
                    recovery_threshold,
                ).await {
                    error!("Health check failed: {}", e);
                }
            }
        });

        info!("Key health monitoring started");
    }

    async fn perform_health_check(
        key_manager: &Arc<dyn KeyManagerTrait>,
        health_stats: &Arc<RwLock<HashMap<String, KeyHealthStats>>>,
        recovery_threshold: Duration,
    ) -> Result<()> {
        debug!("Performing key health check");
        
        let key_states = key_manager.get_key_states().await?;
        let all_keys = key_manager.get_all_key_info().await;
        let mut stats = health_stats.write().await;
        let now = Instant::now();

        for (key, key_info) in all_keys {
            let key_preview = Self::preview_key(&key);
            
            // Получаем или создаем статистику
            let health_stat = stats.entry(key.clone()).or_insert_with(|| {
                KeyHealthStats {
                    key_preview: key_preview.clone(),
                    group_name: key_info.group_name.clone(),
                    total_requests: 0,
                    successful_requests: 0,
                    failed_requests: 0,
                    consecutive_failures: 0,
                    last_success: None,
                    last_failure: None,
                    average_response_time: Duration::from_millis(0),
                    health_score: 1.0,
                    is_healthy: true,
                    recovery_attempts: 0,
                }
            });

            // Обновляем статистику на основе состояния ключа
            if let Some(key_state) = key_states.get(&key) {
                health_stat.consecutive_failures = key_state.consecutive_failures;
                
                if key_state.is_blocked {
                    health_stat.is_healthy = false;
                    
                    // Проверяем возможность восстановления
                    if let Some(last_failure) = key_state.last_failure {
                        let failure_instant = Instant::now() - Duration::from_secs(
                            (chrono::Utc::now() - last_failure).num_seconds() as u64
                        );
                        
                        if now.duration_since(failure_instant) > recovery_threshold {
                            health_stat.recovery_attempts += 1;
                            info!(
                                key_preview = %key_preview,
                                group = %key_info.group_name,
                                recovery_attempts = health_stat.recovery_attempts,
                                "Attempting key recovery"
                            );
                        }
                    }
                }
            }

            // Вычисляем health score
            health_stat.health_score = Self::calculate_health_score(health_stat);
        }

        // Логируем общую статистику
        let total_keys = stats.len();
        let healthy_keys = stats.values().filter(|s| s.is_healthy).count();
        let unhealthy_keys = total_keys - healthy_keys;

        if unhealthy_keys > 0 {
            warn!(
                total_keys,
                healthy_keys,
                unhealthy_keys,
                "Key health check completed with unhealthy keys"
            );
        } else {
            debug!(
                total_keys,
                healthy_keys,
                "All keys are healthy"
            );
        }

        Ok(())
    }

    fn calculate_health_score(stats: &KeyHealthStats) -> f64 {
        if stats.total_requests == 0 {
            return 1.0; // Новый ключ считается здоровым
        }

        let success_rate = stats.successful_requests as f64 / stats.total_requests as f64;
        let failure_penalty = (stats.consecutive_failures as f64 * 0.1).min(0.5);
        
        (success_rate - failure_penalty).max(0.0).min(1.0)
    }

    /// Записывает успешный запрос
    pub async fn record_success(&self, key: &str, response_time: Duration) {
        let mut stats = self.health_stats.write().await;
        let stat = stats.entry(key.to_string()).or_insert_with(|| KeyHealthStats {
            key_preview: Self::preview_key(key),
            group_name: "unknown".to_string(),
            total_requests: 0,
            successful_requests: 0,
            failed_requests: 0,
            consecutive_failures: 0,
            last_success: None,
            last_failure: None,
            average_response_time: Duration::from_millis(0),
            health_score: 1.0,
            is_healthy: true,
            recovery_attempts: 0,
        });
        
        {
            stat.total_requests += 1;
            stat.successful_requests += 1;
            stat.consecutive_failures = 0;
            stat.last_success = Some(Instant::now());
            stat.is_healthy = true;
            
            // Обновляем среднее время ответа
            let total_time = stat.average_response_time.as_millis() as u64 * (stat.successful_requests - 1)
                + response_time.as_millis() as u64;
            stat.average_response_time = Duration::from_millis(total_time / stat.successful_requests);
            
            stat.health_score = Self::calculate_health_score(stat);
        }
    }

    /// Записывает неудачный запрос
    pub async fn record_failure(&self, key: &str, is_terminal: bool) {
        let mut stats = self.health_stats.write().await;
        let stat = stats.entry(key.to_string()).or_insert_with(|| KeyHealthStats {
            key_preview: Self::preview_key(key),
            group_name: "unknown".to_string(),
            total_requests: 0,
            successful_requests: 0,
            failed_requests: 0,
            consecutive_failures: 0,
            last_success: None,
            last_failure: None,
            average_response_time: Duration::from_millis(0),
            health_score: 1.0,
            is_healthy: true,
            recovery_attempts: 0,
        });
        
        {
            stat.total_requests += 1;
            stat.failed_requests += 1;
            stat.consecutive_failures += 1;
            stat.last_failure = Some(Instant::now());
            
            if is_terminal || stat.consecutive_failures >= 3 {
                stat.is_healthy = false;
            }
            
            stat.health_score = Self::calculate_health_score(stat);
        }
    }

    /// Получает статистику здоровья всех ключей
    pub async fn get_health_stats(&self) -> HashMap<String, KeyHealthStats> {
        self.health_stats.read().await.clone()
    }

    /// Получает топ нездоровых ключей
    pub async fn get_unhealthy_keys(&self, limit: usize) -> Vec<KeyHealthStats> {
        let stats = self.health_stats.read().await;
        let mut unhealthy: Vec<_> = stats.values()
            .filter(|s| !s.is_healthy)
            .cloned()
            .collect();
        
        unhealthy.sort_by(|a, b| a.health_score.partial_cmp(&b.health_score).unwrap());
        unhealthy.truncate(limit);
        unhealthy
    }

    fn preview_key(key: &str) -> String {
        if key.len() > 8 {
            format!("{}...{}", &key[..4], &key[key.len() - 4..])
        } else {
            key.to_string()
        }
    }
}