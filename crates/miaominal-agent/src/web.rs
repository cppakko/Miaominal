use crate::error::{AgentError, AgentResult};
use miaominal_settings::{WebSearchConfig, WebSearchProviderKind};
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderValue};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::future::Future;
use std::pin::Pin;

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
    ) -> Pin<Box<dyn Future<Output = AgentResult<Vec<WebSearchResult>>> + Send + 'a>>;
}

#[derive(Debug, Clone, Default)]
pub struct DisabledWebSearchProvider;

impl WebSearchProvider for DisabledWebSearchProvider {
    fn search<'a>(
        &'a self,
        _query: &'a str,
    ) -> Pin<Box<dyn Future<Output = AgentResult<Vec<WebSearchResult>>> + Send + 'a>> {
        Box::pin(async {
            Err(AgentError::UnsupportedProvider(
                "web_search provider is not configured".into(),
            ))
        })
    }
}

#[derive(Debug, Clone)]
pub struct ConfiguredWebSearchProvider {
    client: reqwest::Client,
    config: WebSearchConfig,
    api_key: Option<String>,
}

impl ConfiguredWebSearchProvider {
    pub fn new(config: WebSearchConfig, api_key: Option<String>) -> Self {
        Self {
            client: reqwest::Client::new(),
            config,
            api_key: api_key.and_then(|key| {
                let key = key.trim().to_string();
                (!key.is_empty()).then_some(key)
            }),
        }
    }

    fn api_key(&self) -> AgentResult<Option<&str>> {
        if self.config.kind.requires_api_key() && self.api_key.is_none() {
            return Err(AgentError::UnsupportedProvider(format!(
                "{} web search API key is not configured",
                self.config.kind.label()
            )));
        }

        Ok(self.api_key.as_deref())
    }

    fn endpoint(&self) -> AgentResult<String> {
        let endpoint = self.config.endpoint.trim().trim_end_matches('/');
        if !endpoint.is_empty() {
            return Ok(endpoint.to_string());
        }

        match self.config.kind {
            WebSearchProviderKind::Tavily => Ok("https://api.tavily.com".into()),
            WebSearchProviderKind::Exa => Ok("https://api.exa.ai".into()),
            WebSearchProviderKind::Bocha => Ok("https://api.bochaai.com".into()),
            WebSearchProviderKind::Zhipu => Ok("https://open.bigmodel.cn".into()),
            WebSearchProviderKind::SearXng => Err(AgentError::UnsupportedProvider(
                "SearXNG endpoint is not configured".into(),
            )),
        }
    }

    async fn search_tavily(&self, query: &str) -> AgentResult<Vec<WebSearchResult>> {
        let api_key = self.api_key()?.unwrap_or_default();
        let response = self
            .client
            .post(format!("{}/search", self.endpoint()?))
            .json(&json!({
                "api_key": api_key,
                "query": query,
                "max_results": self.config.max_results,
            }))
            .send()
            .await
            .map_err(anyhow::Error::from)?;
        parse_json_response(response, &["results"]).await
    }

    async fn search_exa(&self, query: &str) -> AgentResult<Vec<WebSearchResult>> {
        let api_key = self.api_key()?.unwrap_or_default();
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-api-key",
            HeaderValue::from_str(api_key).map_err(|error| {
                AgentError::InvalidArguments(format!("invalid Exa API key header: {error}"))
            })?,
        );
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        let response = self
            .client
            .post(format!("{}/search", self.endpoint()?))
            .headers(headers)
            .json(&json!({
                "query": query,
                "numResults": self.config.max_results,
                "contents": { "text": true },
            }))
            .send()
            .await
            .map_err(anyhow::Error::from)?;
        parse_json_response(response, &["results"]).await
    }

    async fn search_bocha(&self, query: &str) -> AgentResult<Vec<WebSearchResult>> {
        let api_key = self.api_key()?.unwrap_or_default();
        let response = self
            .client
            .post(format!("{}/v1/web-search", self.endpoint()?))
            .bearer_auth(api_key)
            .json(&json!({
                "query": query,
                "count": self.config.max_results,
            }))
            .send()
            .await
            .map_err(anyhow::Error::from)?;
        parse_json_response(
            response,
            &["data.webPages.value", "webPages.value", "results"],
        )
        .await
    }

    async fn search_zhipu(&self, query: &str) -> AgentResult<Vec<WebSearchResult>> {
        let api_key = self.api_key()?.unwrap_or_default();
        let response = self
            .client
            .post(format!("{}/api/paas/v4/tools", self.endpoint()?))
            .bearer_auth(api_key)
            .json(&json!({
                "tool": "web-search-pro",
                "messages": [
                    { "role": "user", "content": query }
                ],
                "stream": false,
            }))
            .send()
            .await
            .map_err(anyhow::Error::from)?;
        parse_json_response(
            response,
            &[
                "search_result",
                "results",
                "choices.0.message.tool_calls.0.search_result",
            ],
        )
        .await
    }

    async fn search_sear_xng(&self, query: &str) -> AgentResult<Vec<WebSearchResult>> {
        let mut request = self
            .client
            .get(format!("{}/search", self.endpoint()?))
            .query(&[("q", query), ("format", "json"), ("safesearch", "0")]);

        if let Some(api_key) = self.api_key()? {
            request = request.header(AUTHORIZATION, format!("Bearer {api_key}"));
        }

        let response = request.send().await.map_err(anyhow::Error::from)?;
        parse_json_response(response, &["results"]).await
    }
}

impl WebSearchProvider for ConfiguredWebSearchProvider {
    fn search<'a>(
        &'a self,
        query: &'a str,
    ) -> Pin<Box<dyn Future<Output = AgentResult<Vec<WebSearchResult>>> + Send + 'a>> {
        Box::pin(async move {
            if !self.config.enabled {
                return Err(AgentError::UnsupportedProvider(
                    "web_search provider is disabled".into(),
                ));
            }

            match self.config.kind {
                WebSearchProviderKind::Tavily => self.search_tavily(query).await,
                WebSearchProviderKind::Exa => self.search_exa(query).await,
                WebSearchProviderKind::Bocha => self.search_bocha(query).await,
                WebSearchProviderKind::Zhipu => self.search_zhipu(query).await,
                WebSearchProviderKind::SearXng => self.search_sear_xng(query).await,
            }
            .map(|mut results| {
                results.truncate(self.config.max_results as usize);
                results
            })
        })
    }
}

async fn parse_json_response(
    response: reqwest::Response,
    result_paths: &[&str],
) -> AgentResult<Vec<WebSearchResult>> {
    let status = response.status();
    let body = response.text().await.map_err(anyhow::Error::from)?;
    if !status.is_success() {
        return Err(AgentError::Backend(anyhow::anyhow!(
            "web search request failed with HTTP {status}: {body}"
        )));
    }

    let value: Value =
        serde_json::from_str(&body).map_err(|error| AgentError::Backend(anyhow::anyhow!(error)))?;
    let items = result_paths
        .iter()
        .find_map(|path| value_at_path(&value, path))
        .and_then(Value::as_array)
        .cloned()
        .or_else(|| value.as_array().cloned())
        .unwrap_or_default();

    Ok(items.iter().filter_map(parse_result_item).collect())
}

fn value_at_path<'a>(value: &'a Value, path: &str) -> Option<&'a Value> {
    let mut current = value;
    for segment in path.split('.') {
        current = if let Ok(index) = segment.parse::<usize>() {
            current.as_array()?.get(index)?
        } else {
            current.get(segment)?
        };
    }
    Some(current)
}

fn parse_result_item(item: &Value) -> Option<WebSearchResult> {
    let title = first_string(item, &["title", "name"]).unwrap_or_default();
    let url = first_string(item, &["url", "link", "href"]).unwrap_or_default();
    if url.trim().is_empty() {
        return None;
    }
    let snippet = first_string(
        item,
        &[
            "snippet",
            "content",
            "text",
            "summary",
            "description",
            "body",
        ],
    )
    .unwrap_or_default();

    Some(WebSearchResult {
        title,
        url,
        snippet,
    })
}

fn first_string(item: &Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .filter_map(|key| item.get(*key))
        .find_map(|value| value.as_str().map(str::trim).map(str::to_string))
        .filter(|value| !value.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn value_at_path_reads_nested_arrays() {
        let value = json!({
            "data": {
                "webPages": {
                    "value": [
                        { "name": "Example", "url": "https://example.com" }
                    ]
                }
            }
        });

        let result = value_at_path(&value, "data.webPages.value.0.name");

        assert_eq!(result.and_then(Value::as_str), Some("Example"));
    }

    #[test]
    fn parse_result_item_accepts_common_field_names() {
        let item = json!({
            "name": "Example",
            "link": "https://example.com",
            "description": "A result"
        });

        let result = parse_result_item(&item).expect("result should parse");

        assert_eq!(result.title, "Example");
        assert_eq!(result.url, "https://example.com");
        assert_eq!(result.snippet, "A result");
    }

    #[tokio::test]
    async fn disabled_provider_returns_unsupported_error() {
        let provider = DisabledWebSearchProvider;

        let error = provider.search("miaominal").await.unwrap_err();

        assert!(matches!(error, AgentError::UnsupportedProvider(_)));
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
