use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("JSON parse error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("{provider} returned error: {message}")]
    Provider { provider: String, message: String },

    #[error("authentication failed for {provider}: {reason}")]
    Auth { provider: String, reason: String },

    #[error("rate limit exceeded for {provider}")]
    RateLimit { provider: String },

    #[error("resource not found: {id}")]
    NotFound { id: String },

    #[error("no providers configured")]
    NoProviders,

    #[error("provider {provider} not found in client")]
    ProviderNotFound { provider: String },

    #[error("all providers failed: {details}")]
    AllProvidersFailed { details: String },
}
