use std::{
    io::Read,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use anyhow::{Context, Result, bail};
use chrono::Utc;
use reqwest::{StatusCode, Url, blocking::Client, redirect::Policy};
use serde::{Deserialize, Serialize};
use serde_json::Value;

const MAX_QUERY_BYTES: usize = 500;
const MAX_RESULTS: usize = 10;
const MAX_RESPONSE_BYTES: u64 = 1024 * 1024;

#[derive(Clone)]
pub struct SearchHost {
    provider: Arc<SearchProvider>,
    action_enabled: Arc<AtomicBool>,
}

#[derive(Clone)]
enum SearchProvider {
    Brave { api_key: String },
    Searxng { endpoint: Url },
    Unconfigured { message: String },
}

pub struct SearchActionGuard {
    enabled: Arc<AtomicBool>,
}

impl Drop for SearchActionGuard {
    fn drop(&mut self) {
        self.enabled.store(false, Ordering::Release);
    }
}

#[derive(Debug, Deserialize)]
pub struct SearchRequest {
    pub query: String,
    pub count: Option<usize>,
    #[serde(default)]
    pub freshness: Freshness,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Freshness {
    #[default]
    Any,
    Day,
    Week,
    Month,
    Year,
}

#[derive(Debug, Serialize)]
pub struct SearchResponse {
    pub query: String,
    pub provider: &'static str,
    pub provider_status: &'static str,
    pub retryable: bool,
    pub provider_errors: Vec<String>,
    pub searched_at: String,
    pub results: Vec<SearchResult>,
}

#[derive(Debug, Serialize)]
pub struct SearchResult {
    pub citation_id: usize,
    pub title: String,
    pub url: String,
    pub snippet: String,
    pub published_at: Option<String>,
    pub engine: Option<String>,
}

impl SearchHost {
    pub fn from_env() -> Result<Self> {
        let provider_name = std::env::var("HABIBI_SEARCH_PROVIDER")
            .unwrap_or_else(|_| "brave".to_owned())
            .to_lowercase();
        let provider = match provider_name.as_str() {
            "brave" => match std::env::var("HABIBI_BRAVE_SEARCH_API_KEY") {
                Ok(api_key) if !api_key.is_empty() => SearchProvider::Brave { api_key },
                _ => SearchProvider::Unconfigured {
                    message: "Brave web search requires HABIBI_BRAVE_SEARCH_API_KEY".into(),
                },
            },
            "searxng" => match std::env::var("HABIBI_SEARXNG_URL") {
                Ok(url) => match searxng_endpoint(&url) {
                    Ok(endpoint) => SearchProvider::Searxng { endpoint },
                    Err(error) => SearchProvider::Unconfigured {
                        message: error.to_string(),
                    },
                },
                Err(_) => SearchProvider::Unconfigured {
                    message: "SearXNG web search requires HABIBI_SEARXNG_URL".into(),
                },
            },
            _ => SearchProvider::Unconfigured {
                message: "HABIBI_SEARCH_PROVIDER must be 'brave' or 'searxng'".into(),
            },
        };
        Ok(Self {
            provider: Arc::new(provider),
            action_enabled: Arc::new(AtomicBool::new(false)),
        })
    }

    pub fn configured(&self) -> bool {
        !matches!(self.provider.as_ref(), SearchProvider::Unconfigured { .. })
    }

    pub fn begin_action(&self) -> Result<SearchActionGuard> {
        if self.action_enabled.swap(true, Ordering::AcqRel) {
            bail!("web search action context is already active");
        }
        Ok(SearchActionGuard {
            enabled: self.action_enabled.clone(),
        })
    }

    pub fn search(&self, request: SearchRequest) -> Result<SearchResponse> {
        if !self.action_enabled.load(Ordering::Acquire) {
            bail!("web search is available only during a registered tool action");
        }
        let query = request.query.trim();
        if query.is_empty() || query.len() > MAX_QUERY_BYTES {
            bail!("web search query must contain between 1 and 500 bytes");
        }
        let count = request.count.unwrap_or(5);
        if !(1..=MAX_RESULTS).contains(&count) {
            bail!("web search count must be between 1 and 10");
        }
        let client = search_client()?;
        let (provider, values, provider_errors) = match self.provider.as_ref() {
            SearchProvider::Brave { api_key } => (
                "brave",
                self.search_brave(&client, query, count, &request.freshness, api_key)?,
                Vec::new(),
            ),
            SearchProvider::Searxng { endpoint } => {
                let (values, errors) =
                    self.search_searxng(&client, query, count, &request.freshness, endpoint)?;
                ("searxng", values, errors)
            }
            SearchProvider::Unconfigured { message } => bail!("{message}"),
        };
        let mut results = Vec::new();
        for value in values.into_iter().take(count) {
            let Some(result) = normalize_result(provider, value, results.len() + 1) else {
                continue;
            };
            results.push(result);
        }
        let provider_status = if provider_errors.is_empty() {
            "ok"
        } else if results.is_empty() {
            "unavailable"
        } else {
            "degraded"
        };
        Ok(SearchResponse {
            query: query.to_owned(),
            provider,
            provider_status,
            retryable: provider_status != "unavailable",
            provider_errors,
            searched_at: Utc::now().to_rfc3339(),
            results,
        })
    }

    fn search_brave(
        &self,
        client: &Client,
        query: &str,
        count: usize,
        freshness: &Freshness,
        api_key: &str,
    ) -> Result<Vec<Value>> {
        let mut parameters = vec![
            ("q", query.to_owned()),
            ("count", count.to_string()),
            ("safesearch", "moderate".to_owned()),
        ];
        if let Some(freshness) = brave_freshness(freshness) {
            parameters.push(("freshness", freshness.to_owned()));
        }
        let response = client
            .get("https://api.search.brave.com/res/v1/web/search")
            .header("X-Subscription-Token", api_key)
            .query(&parameters)
            .send()
            .map_err(|_| anyhow::anyhow!("Brave Search request failed"))?;
        let body = bounded_json(response, "Brave Search")?;
        Ok(body
            .pointer("/web/results")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default())
    }

    fn search_searxng(
        &self,
        client: &Client,
        query: &str,
        count: usize,
        freshness: &Freshness,
        endpoint: &Url,
    ) -> Result<(Vec<Value>, Vec<String>)> {
        let mut parameters = vec![
            ("q", query.to_owned()),
            ("format", "json".to_owned()),
            ("safesearch", "1".to_owned()),
        ];
        if let Some(freshness) = searxng_freshness(freshness) {
            parameters.push(("time_range", freshness.to_owned()));
        }
        let response = client
            .get(endpoint.clone())
            .query(&parameters)
            .send()
            .map_err(|_| anyhow::anyhow!("SearXNG request failed"))?;
        let body = bounded_json(response, "SearXNG")?;
        let results = body
            .get("results")
            .and_then(Value::as_array)
            .map(|results| results.iter().take(count).cloned().collect())
            .unwrap_or_default();
        let errors = body
            .get("unresponsive_engines")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .take(10)
            .filter_map(|entry| {
                let pair = entry.as_array()?;
                let engine = pair.first()?.as_str()?;
                let reason = pair.get(1)?.as_str()?;
                Some(format!(
                    "{}: {}",
                    provider_message(engine, 100),
                    provider_message(reason, 200)
                ))
            })
            .collect();
        Ok((results, errors))
    }
}

fn search_client() -> Result<Client> {
    Ok(Client::builder()
        .timeout(Duration::from_secs(10))
        .redirect(Policy::none())
        .user_agent(concat!("Habibi/", env!("CARGO_PKG_VERSION")))
        .build()?)
}

fn bounded_json(mut response: reqwest::blocking::Response, provider: &str) -> Result<Value> {
    if response.status().is_redirection() {
        bail!("{provider} returned a redirect, which is not allowed");
    }
    if response.status() != StatusCode::OK {
        bail!("{provider} returned HTTP {}", response.status().as_u16());
    }
    if response
        .content_length()
        .is_some_and(|length| length > MAX_RESPONSE_BYTES)
    {
        bail!("{provider} response exceeded the 1 MiB limit");
    }
    let mut bytes = Vec::new();
    Read::take(&mut response, MAX_RESPONSE_BYTES + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_RESPONSE_BYTES {
        bail!("{provider} response exceeded the 1 MiB limit");
    }
    serde_json::from_slice(&bytes).with_context(|| format!("{provider} returned invalid JSON"))
}

fn normalize_result(provider: &str, value: Value, citation_id: usize) -> Option<SearchResult> {
    let title = bounded(value.get("title")?.as_str()?, 300);
    let raw_url = value.get("url")?.as_str()?;
    let url = Url::parse(raw_url).ok()?;
    if !matches!(url.scheme(), "http" | "https")
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return None;
    }
    let snippet = match provider {
        "brave" => value.get("description").and_then(Value::as_str),
        _ => value.get("content").and_then(Value::as_str),
    }
    .map(|snippet| bounded(snippet, 1_000))
    .unwrap_or_default();
    let published_at = match provider {
        "brave" => value
            .get("page_age")
            .or_else(|| value.get("age"))
            .and_then(Value::as_str),
        _ => value
            .get("publishedDate")
            .or_else(|| value.get("published_at"))
            .and_then(Value::as_str),
    }
    .map(|value| bounded(value, 100));
    let engine = value
        .get("engine")
        .and_then(Value::as_str)
        .map(|value| bounded(value, 100));
    Some(SearchResult {
        citation_id,
        title,
        url: url.to_string(),
        snippet,
        published_at,
        engine,
    })
}

fn searxng_endpoint(configured: &str) -> Result<Url> {
    let mut url = Url::parse(configured).context("HABIBI_SEARXNG_URL is not a valid URL")?;
    if url.username() != ""
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        bail!("HABIBI_SEARXNG_URL must not contain credentials, query, or fragment");
    }
    let loopback = url
        .host_str()
        .and_then(|host| host.parse::<std::net::IpAddr>().ok())
        .is_some_and(|address| address.is_loopback())
        || url.host_str() == Some("localhost");
    if url.scheme() != "https" && !(url.scheme() == "http" && loopback) {
        bail!("SearXNG must use HTTPS or an explicitly configured loopback HTTP origin");
    }
    url.set_path(&format!("{}/search", url.path().trim_end_matches('/')));
    Ok(url)
}

fn brave_freshness(freshness: &Freshness) -> Option<&'static str> {
    match freshness {
        Freshness::Any => None,
        Freshness::Day => Some("pd"),
        Freshness::Week => Some("pw"),
        Freshness::Month => Some("pm"),
        Freshness::Year => Some("py"),
    }
}

fn searxng_freshness(freshness: &Freshness) -> Option<&'static str> {
    match freshness {
        Freshness::Any => None,
        Freshness::Day => Some("day"),
        Freshness::Week => Some("week"),
        Freshness::Month => Some("month"),
        Freshness::Year => Some("year"),
    }
}

fn bounded(value: &str, characters: usize) -> String {
    value.chars().take(characters).collect()
}

fn provider_message(value: &str, characters: usize) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .take(characters)
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_host(endpoint: Url) -> SearchHost {
        SearchHost {
            provider: Arc::new(SearchProvider::Searxng { endpoint }),
            action_enabled: Arc::new(AtomicBool::new(false)),
        }
    }

    #[test]
    fn search_is_action_only_and_normalizes_a_local_searxng_response() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0u8; 2048];
            let read = stream.read(&mut request).unwrap();
            assert!(String::from_utf8_lossy(&request[..read]).contains("q=public+query"));
            let body = r#"{"results":[{"title":"Example","url":"https://example.com/","content":"Snippet","engine":"test"}]}"#;
            use std::io::Write;
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            )
            .unwrap();
        });
        let host = test_host(Url::parse(&format!("http://{address}/search")).unwrap());
        let request = || SearchRequest {
            query: "public query".into(),
            count: Some(1),
            freshness: Freshness::Any,
        };
        assert!(
            host.search(request())
                .unwrap_err()
                .to_string()
                .contains("tool action")
        );
        let _guard = host.begin_action().unwrap();
        let response = host.search(request()).unwrap();
        assert_eq!(response.provider, "searxng");
        assert_eq!(response.provider_status, "ok");
        assert!(response.retryable);
        assert!(response.provider_errors.is_empty());
        assert_eq!(response.results[0].url, "https://example.com/");
        server.join().unwrap();
    }

    #[test]
    fn forwards_searxng_engine_suspensions_as_non_retryable() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0u8; 2048];
            let _ = stream.read(&mut request).unwrap();
            let body = r#"{"results":[],"unresponsive_engines":[["brave","Suspended: account suspended"],["google","Suspended: CAPTCHA"]]}"#;
            use std::io::Write;
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            )
            .unwrap();
        });
        let host = test_host(Url::parse(&format!("http://{address}/search")).unwrap());
        let _guard = host.begin_action().unwrap();
        let response = host
            .search(SearchRequest {
                query: "public query".into(),
                count: Some(5),
                freshness: Freshness::Any,
            })
            .unwrap();
        assert_eq!(response.provider_status, "unavailable");
        assert!(!response.retryable);
        assert_eq!(
            response.provider_errors,
            [
                "brave: Suspended: account suspended",
                "google: Suspended: CAPTCHA"
            ]
        );
        server.join().unwrap();
    }

    #[test]
    fn validates_searxng_origins() {
        assert!(searxng_endpoint("http://127.0.0.1:8080").is_ok());
        assert!(searxng_endpoint("https://search.example/base").is_ok());
        assert!(searxng_endpoint("http://search.example").is_err());
        assert!(searxng_endpoint("https://user:secret@search.example").is_err());
    }

    #[test]
    fn normalizes_only_citable_http_results() {
        let result = normalize_result(
            "brave",
            serde_json::json!({
                "title": "Example",
                "url": "https://example.com/page",
                "description": "Snippet",
                "page_age": "2026-01-01"
            }),
            1,
        )
        .unwrap();
        assert_eq!(result.citation_id, 1);
        assert_eq!(result.title, "Example");
        assert!(
            normalize_result(
                "brave",
                serde_json::json!({ "title": "Bad", "url": "javascript:alert(1)" }),
                1,
            )
            .is_none()
        );
    }
}
