//! Shared HTTP client.

use std::{sync::OnceLock, time::Duration};

static HTTP_CLIENT: OnceLock<reqwest::Client> = OnceLock::new();

/// Shared HTTP client with sensible defaults.
pub fn client() -> &'static reqwest::Client {
    HTTP_CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .user_agent(concat!("kotoba/", env!("CARGO_PKG_VERSION")))
            .timeout(Duration::from_secs(30))
            .build()
            .expect("HTTP client initialization should not fail")
    })
}
