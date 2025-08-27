// src/monitoring/mod.rs

pub mod key_health;

use crate::error::Result;
use crate::key_manager::KeyManagerTrait;
use key_health::KeyHealthMonitor;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::time::interval;
use tracing::{error, info, warn};

/// Central monitoring system
pub struct MonitoringSystem {
    key_health: KeyHealthMonitor,
    start_time: Instant,
    alert_thresholds: AlertThresholds,
}

#[derive(Debug, Clone)]
pub struct AlertThresholds {
    pub unhealthy_keys_threshold: usize,
    pub error_rate_threshold: f64,
    pub response_time_threshold: Duration,
}

impl Default for AlertThresholds {
    fn default() -> Self {
        Self {
            unhealthy_keys_threshold: 3,
            error_rate_threshold: 0.1, // 10%
            response_time_threshold: Duration::from_secs(5),
        }
    }
}

impl MonitoringSystem {
    pub fn new(key_manager: Arc<dyn KeyManagerTrait>) -> Self {
        Self {
            key_health: KeyHealthMonitor::new(key_manager),
            start_time: Instant::now(),
            alert_thresholds: AlertThresholds::default(),
        }
    }

    /// Starts all monitoring systems
    pub async fn start(&self) -> Result<()> {
        info!("Starting monitoring systems");

        // Start key health monitoring
        self.key_health.start_monitoring().await;

        // Start alert system
        self.start_alerting_system().await;

        info!("All monitoring systems started successfully");
        Ok(())
    }

    async fn start_alerting_system(&self) {
        let _key_health = &self.key_health;
        let thresholds = self.alert_thresholds.clone();

        let key_health = self.key_health.clone();
        tokio::spawn(async move {
            let mut interval = interval(Duration::from_secs(60)); // Check every minute

            loop {
                interval.tick().await;

                if let Err(e) = Self::check_alerts(&key_health, &thresholds).await {
                    error!("Alert check failed: {}", e);
                }
            }
        });
    }

    async fn check_alerts(
        key_health: &KeyHealthMonitor,
        thresholds: &AlertThresholds,
    ) -> Result<()> {
        let unhealthy_keys = key_health.get_unhealthy_keys(10).await;

        if unhealthy_keys.len() >= thresholds.unhealthy_keys_threshold {
            warn!(
                unhealthy_count = unhealthy_keys.len(),
                threshold = thresholds.unhealthy_keys_threshold,
                "ALERT: High number of unhealthy API keys detected"
            );

            // Log details of unhealthy keys
            for key_stat in &unhealthy_keys {
                warn!(
                    key_preview = %key_stat.key_preview,
                    group = %key_stat.group_name,
                    health_score = key_stat.health_score,
                    consecutive_failures = key_stat.consecutive_failures,
                    "Unhealthy key details"
                );
            }
        }

        // Check general statistics
        let all_stats = key_health.get_health_stats().await;
        let total_requests: u64 = all_stats.values().map(|s| s.total_requests).sum();
        let total_failures: u64 = all_stats.values().map(|s| s.failed_requests).sum();

        if total_requests > 0 {
            let error_rate = total_failures as f64 / total_requests as f64;

            if error_rate > thresholds.error_rate_threshold {
                warn!(
                    error_rate = %format!("{:.2}%", error_rate * 100.0),
                    threshold = %format!("{:.2}%", thresholds.error_rate_threshold * 100.0),
                    total_requests,
                    total_failures,
                    "ALERT: High error rate detected"
                );
            }
        }

        Ok(())
    }

    /// Gets general system statistics
    pub async fn get_system_stats(&self) -> SystemStats {
        let uptime = self.start_time.elapsed();
        let key_stats = self.key_health.get_health_stats().await;

        let total_keys = key_stats.len();
        let healthy_keys = key_stats.values().filter(|s| s.is_healthy).count();
        let total_requests: u64 = key_stats.values().map(|s| s.total_requests).sum();
        let total_failures: u64 = key_stats.values().map(|s| s.failed_requests).sum();

        let error_rate = if total_requests > 0 {
            total_failures as f64 / total_requests as f64
        } else {
            0.0
        };

        SystemStats {
            uptime,
            total_keys,
            healthy_keys,
            unhealthy_keys: total_keys - healthy_keys,
            total_requests,
            total_failures,
            error_rate,
        }
    }

    /// Gets key health monitoring
    pub fn key_health(&self) -> &KeyHealthMonitor {
        &self.key_health
    }
}

#[derive(Debug, Clone)]
pub struct SystemStats {
    pub uptime: Duration,
    pub total_keys: usize,
    pub healthy_keys: usize,
    pub unhealthy_keys: usize,
    pub total_requests: u64,
    pub total_failures: u64,
    pub error_rate: f64,
}
