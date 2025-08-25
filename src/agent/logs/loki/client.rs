use std::collections::HashMap;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Result, anyhow};
use base64::prelude::*;
use futures_util::{Stream, StreamExt};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::pin::Pin;
use std::task::{Context, Poll};
use tokio_tungstenite::{connect_async, tungstenite::Message};
use url::Url;

/// A client for querying Grafana Loki log aggregation system
///
/// This client implements the Loki HTTP API for querying logs within time ranges
/// and tailing logs in real-time. It supports various authentication methods
/// including basic auth and bearer tokens.
#[derive(Debug, Clone)]
pub struct LokiClient {
    client: Client,
    base_url: String,
    auth_header: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct QueryRangeParams {
    pub query: String,
    pub start: String,
    pub end: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub direction: Option<Direction>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub step: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub interval: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TailParams {
    pub query: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delay_for: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Direction {
    Forward,
    Backward,
}

#[derive(Debug, Deserialize)]
pub struct LokiResponse {
    pub status: String,
    pub data: QueryData,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum QueryData {
    Matrix {
        #[serde(rename = "resultType")]
        result_type: String,
        result: Vec<MatrixResult>,
        stats: Option<Value>,
    },
    Streams {
        #[serde(rename = "resultType")]
        result_type: String,
        result: Vec<LogStream>,
        stats: Option<Value>,
    },
}

#[derive(Debug, Deserialize)]
pub struct MatrixResult {
    pub metric: HashMap<String, String>,
    pub values: Vec<[String; 2]>,
}

#[derive(Debug, Deserialize)]
pub struct LogStream {
    pub stream: HashMap<String, String>,
    pub values: Vec<[String; 2]>,
}

#[derive(Debug, Deserialize)]
pub struct TailResponse {
    pub streams: Vec<LogStream>,
    #[serde(rename = "droppedEntries")]
    pub dropped_entries: Option<Vec<DroppedEntry>>,
}

#[derive(Debug, Deserialize)]
pub struct DroppedEntry {
    pub labels: HashMap<String, String>,
    pub timestamp: String,
}

/// An async stream for tailing Loki logs
pub struct TailStream {
    read: futures_util::stream::SplitStream<
        tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
    >,
    _write: futures_util::stream::SplitSink<
        tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
        Message,
    >,
}

impl LokiClient {
    /// Create a new Loki client
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            client: Client::new(),
            base_url: base_url.into(),
            auth_header: None,
        }
    }

    /// Create a new Loki client with authentication
    pub fn with_auth(base_url: impl Into<String>, auth_header: impl Into<String>) -> Self {
        Self {
            client: Client::new(),
            base_url: base_url.into(),
            auth_header: Some(auth_header.into()),
        }
    }

    /// Create a new Loki client with basic auth (username:password)
    pub fn with_basic_auth(
        base_url: impl Into<String>,
        username: impl AsRef<str>,
        password: impl AsRef<str>,
    ) -> Self {
        let auth = BASE64_STANDARD.encode(format!("{}:{}", username.as_ref(), password.as_ref()));
        Self::with_auth(base_url, format!("Basic {}", auth))
    }

    /// Create a new Loki client with bearer token
    pub fn with_bearer_token(base_url: impl Into<String>, token: impl AsRef<str>) -> Self {
        Self::with_auth(base_url, format!("Bearer {}", token.as_ref()))
    }

    /// Query logs within a range of time
    pub async fn query_range(&self, params: QueryRangeParams) -> Result<LokiResponse> {
        let url = format!("{}/loki/api/v1/query_range", self.base_url);

        let mut request = self.client.get(&url).query(&params);

        if let Some(auth) = &self.auth_header {
            request = request.header("Authorization", auth);
        }

        let response = request.send().await?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(anyhow!("Request failed with status {}: {}", status, body));
        }

        let loki_response: LokiResponse = response.json().await?;
        Ok(loki_response)
    }

    /// Tail logs in real-time using WebSocket streaming
    ///
    /// Returns an async stream that yields `TailResponse` items.
    /// This implementation connects to Loki's `/loki/api/v1/tail` WebSocket endpoint
    /// for real-time log streaming.
    pub async fn tail(&self, params: TailParams) -> Result<TailStream> {
        // Convert HTTP URL to WebSocket URL
        let ws_url = self.build_ws_tail_url(&params)?;

        // Connect to WebSocket with optional authentication
        let (ws_stream, _) = if let Some(_auth) = &self.auth_header {
            // TODO: For WebSocket, we need to handle auth differently
            connect_async(ws_url.as_str()).await?
        } else {
            connect_async(ws_url.as_str()).await?
        };

        let (write, read) = ws_stream.split();

        Ok(TailStream {
            read,
            _write: write,
        })
    }

    /// Build WebSocket URL for tail endpoint
    fn build_ws_tail_url(&self, params: &TailParams) -> Result<Url> {
        // Convert HTTP(S) to WS(S)
        let ws_base = if self.base_url.starts_with("https://") {
            self.base_url.replace("https://", "wss://")
        } else if self.base_url.starts_with("http://") {
            self.base_url.replace("http://", "ws://")
        } else {
            format!("ws://{}", self.base_url)
        };

        let mut url = Url::parse(&format!("{}/loki/api/v1/tail", ws_base))?;

        // Add query parameters
        {
            let mut query_pairs = url.query_pairs_mut();
            query_pairs.append_pair("query", &params.query);

            if let Some(delay) = params.delay_for {
                query_pairs.append_pair("delay_for", &delay.to_string());
            }
            if let Some(limit) = params.limit {
                query_pairs.append_pair("limit", &limit.to_string());
            }
            if let Some(start) = &params.start {
                query_pairs.append_pair("start", start);
            }
        }

        Ok(url)
    }

    /// Helper method to create a timestamp string from SystemTime
    pub fn timestamp_to_string(time: SystemTime) -> String {
        time.duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
            .to_string()
    }

    /// Helper method to create a timestamp string for "now"
    pub fn now_timestamp() -> String {
        Self::timestamp_to_string(SystemTime::now())
    }

    /// Helper method to create a timestamp string for "duration ago"
    pub fn duration_ago_timestamp(duration: Duration) -> String {
        let now = SystemTime::now();
        let past = now - duration;
        Self::timestamp_to_string(past)
    }
}

impl QueryRangeParams {
    /// Create a new QueryRangeParams with required fields
    pub fn new(query: impl Into<String>, start: impl Into<String>, end: impl Into<String>) -> Self {
        Self {
            query: query.into(),
            start: start.into(),
            end: end.into(),
            limit: None,
            direction: None,
            step: None,
            interval: None,
        }
    }

    /// Set the limit
    pub fn with_limit(mut self, limit: u32) -> Self {
        self.limit = Some(limit);
        self
    }

    /// Set the direction
    pub fn with_direction(mut self, direction: Direction) -> Self {
        self.direction = Some(direction);
        self
    }

    /// Set the step
    pub fn with_step(mut self, step: impl Into<String>) -> Self {
        self.step = Some(step.into());
        self
    }

    /// Set the interval
    pub fn with_interval(mut self, interval: impl Into<String>) -> Self {
        self.interval = Some(interval.into());
        self
    }
}

impl TailParams {
    /// Create a new TailParams with required query
    pub fn new(query: impl Into<String>) -> Self {
        Self {
            query: query.into(),
            delay_for: None,
            limit: None,
            start: None,
        }
    }

    /// Set the delay between requests
    pub fn with_delay(mut self, delay_for: u32) -> Self {
        self.delay_for = Some(delay_for);
        self
    }

    /// Set the limit
    pub fn with_limit(mut self, limit: u32) -> Self {
        self.limit = Some(limit);
        self
    }

    /// Set the start timestamp
    pub fn with_start(mut self, start: impl Into<String>) -> Self {
        self.start = Some(start.into());
        self
    }
}

impl Stream for TailStream {
    type Item = Result<TailResponse>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        loop {
            match Pin::new(&mut self.read).poll_next(cx) {
                Poll::Ready(Some(Ok(msg))) => {
                    match msg {
                        Message::Text(text) => {
                            // Parse the JSON response
                            match serde_json::from_str::<TailResponse>(&text) {
                                Ok(tail_response) => return Poll::Ready(Some(Ok(tail_response))),
                                Err(e) => {
                                    return Poll::Ready(Some(Err(anyhow!(
                                        "Failed to parse tail response: {}",
                                        e
                                    ))));
                                }
                            }
                        }
                        Message::Close(_) => {
                            // WebSocket connection closed
                            return Poll::Ready(None);
                        }
                        Message::Ping(_) => {
                            // TODO: Handle ping by sending pong
                            // For now, just continue to next message
                            continue;
                        }
                        Message::Pong(_) => {
                            // Ignore pong messages
                            continue;
                        }
                        Message::Binary(_) => {
                            return Poll::Ready(Some(Err(anyhow!(
                                "Received unexpected binary message"
                            ))));
                        }
                        Message::Frame(_) => {
                            // Handle raw frames - usually internal to the WebSocket implementation
                            continue;
                        }
                    }
                }
                Poll::Ready(Some(Err(e))) => {
                    return Poll::Ready(Some(Err(anyhow!("WebSocket error: {}", e))));
                }
                Poll::Ready(None) => {
                    return Poll::Ready(None);
                }
                Poll::Pending => {
                    return Poll::Pending;
                }
            }
        }
    }
}
