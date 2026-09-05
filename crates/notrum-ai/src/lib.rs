// Copyright 2026 Evgeniy Udodov
// SPDX-License-Identifier: GPL-3.0-only

#![forbid(unsafe_code)]

//! Shared AI configuration. This crate never reads notes or generates content.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use zeroize::Zeroizing;

mod models;
mod transport;
pub use transport::{CatalogTransport, HttpsCatalogTransport};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AiProvider {
    OpenAi,
    Anthropic,
}

impl AiProvider {
    pub fn name(self) -> &'static str {
        match self {
            Self::OpenAi => "OpenAI",
            Self::Anthropic => "Anthropic",
        }
    }
}

/// Not serializable, and deliberately has no Debug implementation.
pub struct ApiKey(Zeroizing<String>);

impl ApiKey {
    pub fn parse(value: Zeroizing<String>) -> Result<(AiProvider, Self), AiError> {
        let value = Zeroizing::new(value.trim().to_owned());
        let provider = detect_provider(&value).ok_or(AiError::KeyFormat)?;
        Ok((provider, Self(value)))
    }
    pub fn expose(&self) -> &str {
        &self.0
    }
}

pub fn detect_provider(value: &str) -> Option<AiProvider> {
    let value = value.trim();
    if !(24..=4096).contains(&value.len())
        || !value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return None;
    }
    if value.starts_with("sk-ant-api") {
        Some(AiProvider::Anthropic)
    } else if value.starts_with("sk-ant-")
        || value.starts_with("sk-or-")
        || value.starts_with("sk-admin-")
    {
        None
    } else if value.starts_with("sk-proj-")
        || value.starts_with("sk-svcacct-")
        || (value.starts_with("sk-") && !value[3..].contains('-'))
    {
        Some(AiProvider::OpenAi)
    } else {
        None
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AiTaskSize {
    Small,
    Medium,
    Large,
}
impl AiTaskSize {
    pub const ALL: [Self; 3] = [Self::Small, Self::Medium, Self::Large];
    pub fn name(self) -> &'static str {
        match self {
            Self::Small => "Small",
            Self::Medium => "Medium",
            Self::Large => "Large",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AiEffort {
    None,
    Minimal,
    Low,
    Medium,
    High,
    Xhigh,
    Max,
}
impl AiEffort {
    pub const ALL: [Self; 7] = [
        Self::None,
        Self::Minimal,
        Self::Low,
        Self::Medium,
        Self::High,
        Self::Xhigh,
        Self::Max,
    ];
    pub fn name(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Minimal => "minimal",
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::Xhigh => "xhigh",
            Self::Max => "max",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct AiModel {
    pub id: String,
    pub name: String,
    /// Empty means that the model manages reasoning itself.
    pub efforts: Vec<AiEffort>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AiProfile {
    pub model: String,
    pub effort: Option<AiEffort>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AiConnection {
    pub provider: AiProvider,
    pub credential: String,
    pub models: Vec<AiModel>,
    pub checked_at: u64,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AiSettings {
    pub connection: Option<AiConnection>,
    pub profiles: BTreeMap<AiTaskSize, AiProfile>,
    /// Retained until the system store confirms deletion; never contains keys.
    pub pending_deletions: Vec<String>,
    #[serde(flatten)]
    pub additional: BTreeMap<String, serde_json::Value>,
}

impl AiSettings {
    pub fn is_empty(&self) -> bool {
        self.connection.is_none()
            && self.profiles.is_empty()
            && self.pending_deletions.is_empty()
            && self.additional.is_empty()
    }
    /// Shared engine-facing lookup, never silently substitutes a model or effort.
    pub fn profile(&self, size: AiTaskSize) -> Result<&AiProfile, AiError> {
        let profile = self.profiles.get(&size).ok_or(AiError::Incomplete)?;
        self.validate_profile(profile)?;
        Ok(profile)
    }
    pub fn validate_profile(&self, profile: &AiProfile) -> Result<(), AiError> {
        let model = self
            .connection
            .as_ref()
            .ok_or(AiError::Incomplete)?
            .models
            .iter()
            .find(|model| model.id == profile.model)
            .ok_or(AiError::ModelUnavailable)?;
        if if model.efforts.is_empty() {
            profile.effort.is_none()
        } else {
            profile
                .effort
                .is_some_and(|effort| model.efforts.contains(&effort))
        } {
            Ok(())
        } else {
            Err(AiError::EffortRequired)
        }
    }
    pub fn configured_count(&self) -> usize {
        AiTaskSize::ALL
            .iter()
            .filter(|size| self.profile(**size).is_ok())
            .count()
    }
    pub fn ready(&self) -> bool {
        self.configured_count() == 3
    }
    pub fn connect(&mut self, connection: AiConnection) {
        if self
            .connection
            .as_ref()
            .is_some_and(|old| old.provider != connection.provider)
        {
            self.profiles.clear();
        }
        self.connection = Some(connection);
        // Retain unavailable selections visibly, but profile() refuses to use them.
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AiError {
    KeyFormat,
    Unauthorized,
    Forbidden,
    RateLimited,
    Network,
    Response,
    NoModels,
    ModelUnavailable,
    EffortRequired,
    Incomplete,
}

#[cfg(test)]
mod tests {
    use super::*;
    fn connected() -> AiSettings {
        let mut settings = AiSettings::default();
        settings.connect(AiConnection {
            provider: AiProvider::OpenAi,
            credential: "ai/key/test".into(),
            models: vec![AiModel {
                id: "gpt-5.6-luna".into(),
                name: "GPT-5.6 Luna".into(),
                efforts: vec![AiEffort::High],
            }],
            checked_at: 1,
        });
        settings
    }
    #[test]
    fn every_profile_requires_an_explicit_choice() {
        let mut settings = connected();
        assert!(settings.profiles.is_empty());
        assert_eq!(settings.configured_count(), 0);
        let mut profile = AiProfile {
            model: "gpt-5.6-luna".into(),
            effort: None,
        };
        assert_eq!(
            settings.validate_profile(&profile),
            Err(AiError::EffortRequired)
        );
        profile.effort = Some(AiEffort::High);
        for (index, size) in AiTaskSize::ALL.into_iter().enumerate() {
            settings.profiles.insert(size, profile.clone());
            assert_eq!(settings.configured_count(), index + 1);
            assert_eq!(settings.ready(), index == 2);
        }
    }
    #[test]
    fn provider_detection_never_probes_multiple_services() {
        assert_eq!(
            detect_provider("sk-ant-api03-abcdefghijklmnop"),
            Some(AiProvider::Anthropic)
        );
        assert_eq!(
            detect_provider("sk-proj-abcdefghijklmnopqrstuv"),
            Some(AiProvider::OpenAi)
        );
        for key in [
            "sk-ant-oat01-abcdefghijklmnopqrstuv",
            "sk-or-v1-abcdefghijklmnopqrstuv",
            "unknown",
            "sk-proj-abcdefghijklmn\nxyz",
        ] {
            assert_eq!(detect_provider(key), None);
        }
    }
    #[test]
    fn connection_changes_do_not_fill_or_substitute_profiles() {
        let mut settings = connected();
        settings.profiles.insert(
            AiTaskSize::Small,
            AiProfile {
                model: "gpt-5.6-luna".into(),
                effort: Some(AiEffort::High),
            },
        );
        let mut connection = settings.connection.clone().unwrap();
        connection.models.clear();
        settings.connect(connection.clone());
        assert_eq!(
            settings.profile(AiTaskSize::Small),
            Err(AiError::ModelUnavailable)
        );
        assert_eq!(settings.profiles.len(), 1);
        connection.provider = AiProvider::Anthropic;
        settings.connect(connection);
        assert!(settings.profiles.is_empty());
    }
}
