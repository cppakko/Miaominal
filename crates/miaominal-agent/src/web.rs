use crate::error::{AgentError, AgentResult};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WebSearchResult {
    pub title: String,
    pub url: String,
    pub snippet: String,
}

pub trait WebSearchProvider: Send + Sync {
    fn search<'a>(
        &'a self,
        query: &'a str,
    ) -> impl Future<Output = AgentResult<Vec<WebSearchResult>>> + Send + 'a;
}

#[derive(Debug, Clone, Default)]
pub struct DisabledWebSearchProvider;

impl WebSearchProvider for DisabledWebSearchProvider {
    async fn search(&self, _query: &str) -> AgentResult<Vec<WebSearchResult>> {
        Err(AgentError::UnsupportedProvider(
            "web_search provider is not configured".into(),
        ))
    }
}

#[derive(Debug, Clone)]
pub struct WebFetchConfig {
    pub max_bytes: usize,
}

impl Default for WebFetchConfig {
    fn default() -> Self {
        Self {
            max_bytes: 128 * 1024,
        }
    }
}
