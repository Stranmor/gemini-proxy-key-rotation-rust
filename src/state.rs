// src/state.rs

// Removed KeyState and FlattenedKeyInfo definitions (moved to key_manager.rs)
use crate::config::AppConfig;
use crate::error::{AppError, Result}; // Re-added AppError
// Import KeyManager
use crate::key_manager::KeyManager;
use reqwest::Client;
use std::time::Duration;
// Removed unused imports: HashMap, AtomicUsize, Ordering, chrono*, chrono_tz*, RwLock, error, debug, warn
use tracing::info; // Keep info for logging

// KeyState and FlattenedKeyInfo structs removed.

/// Represents the shared application state that is accessible by all Axum handlers.
///
/// This struct holds instances of shared resources like the HTTP client and the key manager.
/// It is typically wrapped in an `Arc` for thread-safe sharing across asynchronous tasks.
#[derive(Debug)]
pub struct AppState {
    // Fields related to keys are removed.
    /// Shared HTTP client (used as a base, proxy applied per-request).
    http_client: Client,
    /// Manages the API keys and their states. Made public for handler access.
    pub key_manager: KeyManager,
    // config: Arc<AppConfig>, // Keep config if needed for other state parts, otherwise remove
}

impl AppState {
    /// Creates a new `AppState`.
    ///
    /// Initializes the HTTP client and the KeyManager.
    pub fn new(config: &AppConfig) -> Result<Self> {
        info!("Creating shared AppState: Initializing HTTP client and KeyManager...");

        // Calculate total keys for pool size estimation (needed for client builder)
        let total_key_count: usize = config
            .groups
            .iter()
            .flat_map(|g| &g.api_keys) // Flatten all keys from all groups
            .filter(|k| !k.trim().is_empty()) // Count only non-empty keys
            .count();

        // --- HTTP Client Initialization ---
        let http_client = Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(300)) // Increased timeout to 5 minutes
            .pool_idle_timeout(Duration::from_secs(90))
            // Set pool size based on total non-empty keys, with a minimum fallback
            .pool_max_idle_per_host(total_key_count.max(10))
            .build().map_err(AppError::from)?;
        info!("HTTP client created successfully.");

        // --- Key Manager Initialization ---
        // KeyManager::new handles flattening keys and initializing states internally.
        let key_manager = KeyManager::new(config);

        // Key flattening and state initialization logic removed (now inside KeyManager::new).

        Ok(Self {
            http_client,
            key_manager, // Store the initialized KeyManager
                         // config: Arc::new(config), // Store config if needed later
        })
    }

    // Methods get_next_available_key_info, mark_key_as_limited, and preview removed.

    /// Returns a reference to the shared base HTTP client. Proxy settings are applied per-request.
    #[inline]
    pub const fn client(&self) -> &Client {
        &self.http_client
    }
}
