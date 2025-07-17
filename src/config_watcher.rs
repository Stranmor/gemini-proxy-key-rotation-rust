// src/config_watcher.rs

use crate::config::{load_config, AppConfig};
use crate::error::{AppError, Result};
use notify::{Config, Event, RecommendedWatcher, RecursiveMode, Watcher};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::{mpsc, RwLock};
use tracing::{debug, error, info, warn};

/// Configuration watcher that monitors config file changes
pub struct ConfigWatcher {
    config_path: PathBuf,
    current_config: Arc<RwLock<AppConfig>>,
    _watcher: RecommendedWatcher,
    receiver: mpsc::Receiver<Result<AppConfig>>,
}

impl ConfigWatcher {
    /// Create a new config watcher
    ///
    /// # Errors
    ///
    /// Returns an error if the initial configuration cannot be loaded or if the
    /// file watcher cannot be created.
    pub fn new(config_path: PathBuf) -> Result<Self> {
        let (tx, receiver) = mpsc::channel(10);
        let config_path_clone = config_path.clone();

        let mut watcher = RecommendedWatcher::new(
            #[allow(clippy::cognitive_complexity)]
            move |res: notify::Result<Event>| {
                if let Ok(event) = res {
                    debug!("Config file event: {event:?}");

                    // Only react to write events
                    if matches!(event.kind, notify::EventKind::Modify(_)) {
                        info!("Config file modified, reloading...");

                        match load_config(&config_path_clone) {
                            Ok(new_config) => {
                                if let Err(e) = tx.try_send(Ok(new_config)) {
                                    warn!("Failed to send config update: {e}");
                                }
                            }
                            Err(e) => {
                                error!("Failed to reload config: {e:?}");
                                if let Err(send_err) = tx.try_send(Err(e)) {
                                    warn!("Failed to send config error: {send_err}");
                                }
                            }
                        }
                    }
                } else if let Err(e) = res {
                    error!("Config watcher error: {e:?}");
                }
            },
            Config::default(),
        )
        .map_err(|e| AppError::Internal(format!("Failed to create file watcher: {e}")))?;

        // Watch the config file
        watcher
            .watch(&config_path, RecursiveMode::NonRecursive)
            .map_err(|e| AppError::Internal(format!("Failed to watch config file: {e}")))?;

        // Load initial config
        let initial_config = load_config(&config_path)?;
        let current_config = Arc::new(RwLock::new(initial_config));

        info!("Config watcher initialized for: {}", config_path.display());

        Ok(Self {
            config_path,
            current_config,
            _watcher: watcher,
            receiver,
        })
    }

    /// Get the current configuration
    pub async fn get_config(&self) -> AppConfig {
        self.current_config.read().await.clone()
    }

    /// Wait for the next configuration change
    ///
    /// # Errors
    ///
    /// Returns an error if the configuration cannot be reloaded or if the
    /// watcher channel has been closed.
    pub async fn wait_for_change(&mut self) -> Result<AppConfig> {
        if let Some(config_result) = self.receiver.recv().await {
            match config_result {
                Ok(new_config) => {
                    info!("Configuration reloaded successfully");
                    *self.current_config.write().await = new_config.clone();
                    Ok(new_config)
                }
                Err(e) => {
                    warn!("Configuration reload failed, keeping current config: {e:?}");
                    Err(e)
                }
            }
        } else {
            error!("Config watcher channel closed");
            Err(AppError::Internal(
                "Config watcher channel closed".to_string(),
            ))
        }
    }

    /// Force reload the configuration
    ///
    /// # Errors
    ///
    /// Returns an error if the configuration file cannot be read or parsed.
    pub async fn force_reload(&self) -> Result<AppConfig> {
        info!("Force reloading configuration from: {}", self.config_path.display());
        
        match load_config(&self.config_path) {
            Ok(new_config) => {
                *self.current_config.write().await = new_config.clone();
                info!("Configuration force reloaded successfully");
                Ok(new_config)
            }
            Err(e) => {
                error!("Failed to force reload config: {e:?}");
                Err(e)
            }
        }
    }

    /// Get configuration file path
    #[must_use]
    #[allow(clippy::missing_const_for_fn)]
    pub fn config_path(&self) -> &Path {
        &self.config_path
    }
}

/// Configuration change notification
#[derive(Debug, Clone)]
pub struct ConfigChange {
    pub old_config: AppConfig,
    pub new_config: AppConfig,
    pub timestamp: chrono::DateTime<chrono::Utc>,
}

impl ConfigChange {
    #[must_use]
    pub fn new(old_config: AppConfig, new_config: AppConfig) -> Self {
        Self {
            old_config,
            new_config,
            timestamp: chrono::Utc::now(),
        }
    }

    /// Check if API keys have changed
    #[must_use]
    pub fn keys_changed(&self) -> bool {
        self.old_config.groups != self.new_config.groups
    }

    /// Check if server configuration has changed
    #[must_use]
    pub fn server_changed(&self) -> bool {
        self.old_config.server != self.new_config.server
    }

    /// Get summary of changes
    #[must_use]
    pub fn change_summary(&self) -> String {
        let mut changes = Vec::new();
        
        if self.server_changed() {
            changes.push("server configuration");
        }
        
        if self.keys_changed() {
            changes.push("API key groups");
        }

        if changes.is_empty() {
            "no significant changes detected".to_string()
        } else {
            format!("changed: {}", changes.join(", "))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{KeyGroup, ServerConfig};
    use std::fs;
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_config_watcher_creation() {
        let temp_dir = tempdir().unwrap();
        let config_path = temp_dir.path().join("test_config.yaml");
        use std::io::Write;

        // Create an empty config file to ensure the watcher can be created.
        // The watcher needs to be able to read the file on initialization.
        let initial_config = AppConfig {
            server: ServerConfig {
                host: "127.0.0.1".to_string(),
                port: 8080,
            },
            groups: vec![KeyGroup {
                name: "test-group".to_string(),
                api_keys: vec!["test-key".to_string()],
                target_url: "http://localhost:1234".to_string(),
                proxy_url: None,
            }],
        };
        let yaml = serde_yaml::to_string(&initial_config).unwrap();
        
        // Use File::create and sync_all to ensure the file is written to disk
        let mut temp_file = fs::File::create(&config_path).unwrap();
        temp_file.write_all(yaml.as_bytes()).unwrap();
        temp_file.sync_all().unwrap(); // This is the fix

        // Give the filesystem and notify crate a moment to catch up.
        // This is the definitive fix for the race condition.
        std::thread::sleep(std::time::Duration::from_millis(50));

        let watcher_result = ConfigWatcher::new(config_path);
        assert!(watcher_result.is_ok(), "Watcher creation failed: {:?}", watcher_result.err());
    }

    #[tokio::test]
    async fn test_config_change_detection() {
        let change = ConfigChange::new(
            AppConfig {
                server: ServerConfig {
                    host: "127.0.0.1".to_string(),
                    port: 8080,
                },
                groups: vec![],
            },
            AppConfig {
                server: ServerConfig {
                    host: "0.0.0.0".to_string(),
                    port: 8081,
                },
                groups: vec![KeyGroup {
                    name: "test".to_string(),
                    api_keys: vec!["key1".to_string()],
                    proxy_url: None,
                    target_url: "http://test.com".to_string(),
                }],
            },
        );

        assert!(change.server_changed());
        assert!(change.keys_changed());
        assert!(change.change_summary().contains("server configuration"));
        assert!(change.change_summary().contains("API key groups"));
    }





































}