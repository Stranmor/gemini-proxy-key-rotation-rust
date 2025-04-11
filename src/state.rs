use crate::config::ProxyConfig;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct ProxyState {
    pub config: Arc<ProxyConfig>,
    key_index: Arc<AtomicUsize>,
    http_client: reqwest::Client,
}

impl ProxyState {
    pub fn new(config: ProxyConfig) -> Self {
        ProxyState {
            config: Arc::new(config),
            key_index: Arc::new(AtomicUsize::new(0)),
            http_client: reqwest::Client::builder()
                .use_rustls_tls()
                .build()
                .expect("Failed to build reqwest client"),
        }
    }

    pub fn get_next_api_key(&self) -> Option<String> {
        let keys = &self.config.api_keys;
        if keys.is_empty() {
            return None;
        }
        let index = self.key_index.fetch_add(1, Ordering::Relaxed) % keys.len();
        keys.get(index).cloned()
    }

    pub fn client(&self) -> &reqwest::Client {
        &self.http_client
    }
}