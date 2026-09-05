// Copyright 2026 Evgeniy Udodov
// SPDX-License-Identifier: GPL-3.0-only

#![forbid(unsafe_code)]

use crate::{AiError, AiModel, AiProvider, ApiKey, models};
use serde::Deserialize;
use std::collections::BTreeSet;
use std::time::{Duration, Instant};

const MAX_PAGE_BYTES: u64 = 512 * 1024;
const MAX_MODELS: usize = 2000;

pub trait CatalogTransport: Send + Sync {
    fn list(&self, provider: AiProvider, key: &ApiKey) -> Result<Vec<AiModel>, AiError>;
}

pub struct HttpsCatalogTransport;

impl CatalogTransport for HttpsCatalogTransport {
    fn list(&self, provider: AiProvider, key: &ApiKey) -> Result<Vec<AiModel>, AiError> {
        if crate::detect_provider(key.expose()) != Some(provider) {
            return Err(AiError::KeyFormat);
        }
        let config = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(15)))
            .https_only(true)
            .max_redirects(0)
            .proxy(None)
            .build();
        let agent = ureq::Agent::new_with_config(config);
        let start = Instant::now();
        collect(provider, |cursor| {
            if start.elapsed() > Duration::from_secs(30) {
                return Err(AiError::Network);
            }
            let mut request = match provider {
                AiProvider::OpenAi => agent
                    .get("https://api.openai.com/v1/models")
                    .header("Authorization", format!("Bearer {}", key.expose())),
                AiProvider::Anthropic => {
                    let request = agent
                        .get("https://api.anthropic.com/v1/models")
                        .header("x-api-key", key.expose())
                        .header("anthropic-version", "2023-06-01")
                        .query("limit", "1000");
                    if let Some(cursor) = cursor {
                        request.query("after_id", cursor)
                    } else {
                        request
                    }
                }
            };
            request = request.header("Accept", "application/json");
            let mut response = request.call().map_err(|error| match error {
                ureq::Error::StatusCode(code) => status_error(code),
                _ => AiError::Network,
            })?;
            if response.status().as_u16() != 200 {
                return Err(status_error(response.status().as_u16()));
            }
            response
                .body_mut()
                .with_config()
                .limit(MAX_PAGE_BYTES)
                .read_to_vec()
                .map_err(|_| AiError::Response)
        })
    }
}

fn status_error(code: u16) -> AiError {
    match code {
        401 => AiError::Unauthorized,
        403 => AiError::Forbidden,
        429 => AiError::RateLimited,
        _ => AiError::Network,
    }
}

#[derive(Deserialize)]
struct Page {
    data: Vec<serde_json::Value>,
    #[serde(default)]
    has_more: bool,
    last_id: Option<String>,
}

fn collect(
    provider: AiProvider,
    mut fetch: impl FnMut(Option<&str>) -> Result<Vec<u8>, AiError>,
) -> Result<Vec<AiModel>, AiError> {
    let mut cursor: Option<String> = None;
    let mut cursors = BTreeSet::new();
    let mut ids = BTreeSet::new();
    let mut result = Vec::new();
    let mut count = 0usize;
    for _ in 0..10 {
        let bytes = fetch(cursor.as_deref())?;
        if bytes.len() as u64 > MAX_PAGE_BYTES {
            return Err(AiError::Response);
        }
        let page: Page = serde_json::from_slice(&bytes).map_err(|_| AiError::Response)?;
        count += page.data.len();
        if count > MAX_MODELS {
            return Err(AiError::Response);
        }
        for raw in page.data {
            if let Some(model) = models::model(provider, &raw)
                && ids.insert(model.id.clone())
            {
                result.push(model);
            }
        }
        if !page.has_more {
            result.sort_by(|a, b| a.name.cmp(&b.name));
            return if result.is_empty() {
                Err(AiError::NoModels)
            } else {
                Ok(result)
            };
        }
        if provider != AiProvider::Anthropic {
            return Err(AiError::Response);
        }
        let next = page
            .last_id
            .filter(|id| models::valid_id(id))
            .ok_or(AiError::Response)?;
        if !cursors.insert(next.clone()) {
            return Err(AiError::Response);
        }
        cursor = Some(next);
    }
    Err(AiError::Response)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bounded_pagination_and_errors_never_include_response_text() {
        let mut calls = 0;
        let models = collect(AiProvider::Anthropic, |cursor| {
            calls += 1;
            if cursor.is_none() { Ok(br#"{"data":[{"id":"claude-opus-4-6"}],"has_more":true,"last_id":"claude-opus-4-6"}"#.to_vec()) }
            else { Ok(br#"{"data":[{"id":"claude-sonnet-4-6"}]}"#.to_vec()) }
        }).unwrap();
        assert_eq!(calls, 2);
        assert_eq!(models.len(), 2);
        assert_eq!(
            collect(AiProvider::Anthropic, |_| Ok(
                br#"{"data":[],"has_more":true,"last_id":"same"}"#.to_vec()
            )),
            Err(AiError::Response)
        );
        assert_eq!(
            collect(AiProvider::OpenAi, |_| Ok(b"secret-token".to_vec())),
            Err(AiError::Response)
        );
        assert_eq!(status_error(401), AiError::Unauthorized);
        assert_eq!(status_error(403), AiError::Forbidden);
        assert_eq!(status_error(429), AiError::RateLimited);
    }
}
