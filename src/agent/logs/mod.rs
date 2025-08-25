use std::{
    ops::RangeInclusive,
    pin::Pin,
    task::{Context, Poll},
};

use anyhow::Result;
use futures_util::Stream;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{
    agent::logs::loki::{
        Direction, LokiClient, LokiResponse, QueryData, QueryRangeParams, TailParams, TailResponse,
        TailStream,
    },
    resources::core::{LogStreamItem, LogStreamTarget},
};

pub mod loki;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(tag = "type")]
pub enum LogsStoreConfig {
    #[serde(rename = "loki")]
    Loki(LokiStoreConfig),
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct LokiStoreConfig {
    pub url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct LogsAgentConfig {
    pub otel_ingest_endpoint: String,
    pub store: LogsStoreConfig,
}

pub struct LogsAgent {
    config: LogsAgentConfig,
}

#[derive(Debug)]
pub enum LogStreamOrigin {
    Machine {
        tenant: String,
        name: String,
        namespace: Option<String>,
    },
    Group {
        tenant: String,
        name: String,
        namespace: Option<String>,
    },
}

impl LogStreamOrigin {
    pub fn loki_log_query(&self) -> String {
        let tenant = match self {
            LogStreamOrigin::Machine { tenant, .. } => tenant,
            LogStreamOrigin::Group { tenant, .. } => tenant,
        };

        let mut loki_log_query = vec![format!("service_tenant = \"{}\"", tenant)];

        loki_log_query.push(match self {
            LogStreamOrigin::Machine { name, .. } => {
                format!("service_name = \"{}\"", name)
            }
            LogStreamOrigin::Group { name, .. } => {
                format!("service_group = \"{}\"", name)
            }
        });

        if let Some(namespace) = match self {
            LogStreamOrigin::Machine { namespace, .. } => namespace,
            LogStreamOrigin::Group { namespace, .. } => namespace,
        } {
            loki_log_query.push(format!("service_namespace = \"{}\"", namespace));
        }

        format!("{{ {} }}", loki_log_query.join(", "))
    }
}

impl LogsAgent {
    pub fn new(config: LogsAgentConfig) -> Self {
        Self { config }
    }

    fn get_loki_client(&self) -> Result<LokiClient> {
        let LogsStoreConfig::Loki(loki_config) = &self.config.store;
        Ok(LokiClient::new(loki_config.url.clone()))
    }

    pub fn get_otel_ingest_endpoint(&self) -> String {
        self.config.otel_ingest_endpoint.clone()
    }

    pub async fn query(
        &self,
        origin: LogStreamOrigin,
        range: RangeInclusive<u128>,
    ) -> Result<Vec<LogStreamItem>> {
        let loki_client = self.get_loki_client()?;
        let loki_log_query = origin.loki_log_query();

        let mut start_ts = *range.start();
        let end_ts = *range.end();

        let mut output = vec![];

        loop {
            let Ok(LokiResponse {
                data: QueryData::Streams { result, .. },
                ..
            }) = loki_client
                .query_range(
                    QueryRangeParams::new(
                        loki_log_query.clone(),
                        start_ts.to_string(),
                        end_ts.to_string(),
                    )
                    .with_limit(1000)
                    .with_direction(Direction::Forward),
                )
                .await
            else {
                return Ok(output);
            };

            // Collect all entries from all streams and sort by timestamp
            let mut all_entries: Vec<(u128, String, LogStreamTarget)> = Vec::new();

            for stream in result {
                let target_stream = stream
                    .stream
                    .get("log_stream")
                    .and_then(|s| s.parse::<LogStreamTarget>().ok())
                    .unwrap_or(LogStreamTarget::Stdout);

                for entry in stream.values {
                    let timestamp = entry[0].parse::<u128>().unwrap_or(0);
                    let message = entry[1].clone();
                    all_entries.push((timestamp, message, target_stream.clone()));
                }
            }

            // Sort entries by timestamp to interlace streams
            all_entries.sort_by_key(|&(timestamp, _, _)| timestamp);
            let entry_count = all_entries.len();

            for (timestamp, message, target_stream) in all_entries {
                if timestamp > start_ts {
                    start_ts = timestamp + 1;
                }

                let log_stream_item = LogStreamItem {
                    timestamp,
                    message,
                    target_stream,
                };

                output.push(log_stream_item);
            }

            if entry_count == 0 {
                break;
            }
        }

        Ok(output)
    }

    pub async fn stream(&self, origin: LogStreamOrigin) -> Result<LogStream> {
        let loki_client = self.get_loki_client()?;
        let loki_log_query = origin.loki_log_query();

        Ok(LogStream {
            inner: loki_client.tail(TailParams::new(loki_log_query)).await?,
        })
    }
}

pub struct LogStream {
    inner: TailStream,
}

impl Stream for LogStream {
    type Item = Vec<LogStreamItem>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let result = Pin::new(&mut self.inner).poll_next(cx);

        match result {
            Poll::Ready(Some(Ok(TailResponse { streams, .. }))) => {
                let mut output = vec![];

                for stream in streams {
                    let target_stream = stream
                        .stream
                        .get("log_stream")
                        .and_then(|s| s.parse::<LogStreamTarget>().ok())
                        .unwrap_or(LogStreamTarget::Stdout);

                    for entry in stream.values {
                        let timestamp = entry[0].parse::<u128>().unwrap_or(0);
                        let message = entry[1].clone();
                        output.push(LogStreamItem {
                            timestamp,
                            message,
                            target_stream: target_stream.clone(),
                        });
                    }
                }

                Poll::Ready(Some(output))
            }
            Poll::Ready(Some(Err(_))) => {
                return Poll::Ready(None);
            }
            Poll::Ready(None) => Poll::Ready(None),
            Poll::Pending => Poll::Pending,
        }
    }
}
