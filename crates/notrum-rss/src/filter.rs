// Copyright 2026 Evgeniy Udodov
// SPDX-License-Identifier: GPL-3.0-only

#![forbid(unsafe_code)]

use super::*;

pub const MAX_PREFERENCE_BYTES: usize = 16 * 1024;
pub const AI_VISIT_LIMIT: u16 = 99;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RssPreferences {
    pub likes: String,
    pub dislikes: String,
    pub alias: String,
    pub version: u64,
    #[serde(flatten)]
    pub additional: BTreeMap<String, serde_json::Value>,
}

impl Default for RssPreferences {
    fn default() -> Self {
        Self {
            likes: String::new(),
            dislikes: String::new(),
            alias: "default".into(),
            version: 0,
            additional: BTreeMap::new(),
        }
    }
}

impl RssPreferences {
    pub fn enabled(&self) -> bool {
        !self.likes.trim().is_empty() || !self.dislikes.trim().is_empty()
    }
    pub fn validate(&self) -> Result<(), EngineError> {
        if self.likes.len() > MAX_PREFERENCE_BYTES
            || self.dislikes.len() > MAX_PREFERENCE_BYTES
            || self.alias.len() > MAX_PREFERENCE_BYTES
        {
            return Err(EngineError::InvalidSetting("rss/preferences/size".into()));
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RssReaction {
    Like,
    Dislike,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RssDecision {
    Keep,
    Hide,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RssEntryState {
    pub reaction: Option<RssReaction>,
    pub reaction_version: u64,
    pub learned_version: u64,
    pub decision: Option<RssDecision>,
    pub preferences_version: u64,
    pub content_version: String,
    #[serde(flatten)]
    pub additional: BTreeMap<String, serde_json::Value>,
}

impl RssEntryState {
    pub fn hidden(&self) -> bool {
        match self.reaction {
            Some(RssReaction::Like) => false,
            Some(RssReaction::Dislike) => true,
            None => self.decision == Some(RssDecision::Hide),
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RssSchedule {
    pub iteration: u32,
    pub next_check: u64,
    pub paused: bool,
    pub ai_used: u16,
    pub forced: bool,
    pub filter_pending: bool,
    pub visit: u64,
    #[serde(flatten)]
    pub additional: BTreeMap<String, serde_json::Value>,
}

impl RssSchedule {
    pub fn visit(&mut self, now: u64) {
        self.iteration = 0;
        self.next_check = now;
        self.paused = false;
        self.ai_used = 0;
        self.forced = true;
        self.filter_pending = false;
        self.visit = self.visit.saturating_add(1);
    }
    pub fn allowed(&mut self, unread: usize) -> bool {
        if !self.forced && unread >= 99 {
            self.paused = true;
        }
        self.forced || !self.paused
    }
    pub fn finish(&mut self, now: u64, unread: usize) {
        let base = if unread > 0 { 2_f64 } else { 1.5_f64 };
        let delay =
            ((60.0 + base.powi(self.iteration.min(64) as i32)).min(86400.0) * 1000.0).ceil() as u64;
        self.next_check = now.saturating_add(delay);
        self.iteration = self.iteration.saturating_add(1);
        self.filter_pending = true;
    }
    pub fn finish_cycle(&mut self, unread: usize) {
        self.forced = false;
        self.filter_pending = false;
        self.allowed(unread);
    }
}

impl RssReadState {
    pub fn hidden(&self, id: &str) -> bool {
        self.entries.get(id).is_some_and(RssEntryState::hidden)
    }
    pub fn all_unread(&self, feed: &RssFeedCache) -> usize {
        feed.entries
            .iter()
            .filter(|entry| !self.read_entry_ids.contains(&entry.id))
            .count()
    }
}

pub fn content_version(entry: &RssEntry) -> String {
    digest_string(&serde_json::to_string(entry).expect("RSS entries serialize"))
}

#[derive(Clone, Debug)]
pub struct RssCheck {
    pub entry: RssEntry,
    pub preferences_version: u64,
    pub reaction_version: u64,
    pub content_version: String,
}

impl RssEngine {
    pub fn preferences(&self, id: &ItemId) -> Result<RssPreferences, EngineError> {
        self.subscriptions
            .iter()
            .find(|s| &s.id == id && !s.deleted)
            .map(|s| s.preferences.clone())
            .ok_or(EngineError::Conflict)
    }

    pub fn save_preferences(
        &mut self,
        id: &ItemId,
        expected: u64,
        mut value: RssPreferences,
    ) -> Result<(), EngineError> {
        value.validate()?;
        let _lock = self.operation_lock()?;
        let current = self.preferences(id)?;
        if current.version != expected {
            return Err(EngineError::Conflict);
        }
        value.version = current
            .version
            .checked_add(1)
            .ok_or(EngineError::Conflict)?;
        value.additional = current.additional;
        let enabled = value.enabled();
        self.update_subscription(id, |s| s.preferences = value)?;
        self.update_state(id, |state| {
            state.schedule.filter_pending = enabled;
            if !enabled {
                for entry in state.entries.values_mut() {
                    entry.decision = None;
                }
            }
            Ok(())
        })?;
        Ok(())
    }

    pub fn update_state<T>(
        &self,
        id: &ItemId,
        update: impl FnOnce(&mut RssReadState) -> Result<T, EngineError>,
    ) -> Result<T, EngineError> {
        let _lock = self.operation_lock()?;
        self.preferences(id)?;
        let mut state = self.load_read_state(id)?;
        let expected = state.revision;
        let result = update(&mut state)?;
        if self.load_read_state(id)?.revision != expected {
            return Err(EngineError::Conflict);
        }
        state.revision = expected.checked_add(1).ok_or(EngineError::Conflict)?;
        write_json_atomic(&read_state_path(&self.workspace, id), &state)?;
        Ok(result)
    }

    pub fn react(
        &self,
        id: &ItemId,
        entry_id: &str,
        reaction: RssReaction,
    ) -> Result<bool, EngineError> {
        if !self
            .load_cache(id)?
            .entries
            .iter()
            .any(|e| e.id == entry_id)
        {
            return Err(EngineError::Conflict);
        }
        self.update_state(id, |state| {
            let entry = state.entries.entry(entry_id.into()).or_default();
            if entry.reaction == Some(reaction) {
                return Ok(false);
            }
            entry.reaction = Some(reaction);
            entry.reaction_version = entry
                .reaction_version
                .checked_add(1)
                .ok_or(EngineError::Conflict)?;
            Ok(true)
        })
    }

    pub fn checks(&self, id: &ItemId, learning: bool) -> Result<Vec<RssCheck>, EngineError> {
        let preferences = self.preferences(id)?;
        let (feed, mut state) = self.feed(id)?;
        if !learning
            && (!preferences.enabled()
                || !state.schedule.allowed(state.all_unread(&feed))
                || state.schedule.ai_used >= AI_VISIT_LIMIT)
        {
            return Ok(Vec::new());
        }
        let limit = if learning {
            1
        } else {
            usize::from(AI_VISIT_LIMIT - state.schedule.ai_used).min(10)
        };
        Ok(feed
            .entries
            .into_iter()
            .filter_map(|entry| {
                let saved = state.entries.get(&entry.id).cloned().unwrap_or_default();
                let version = content_version(&entry);
                let needed = if learning {
                    saved.reaction.is_some() && saved.learned_version != saved.reaction_version
                } else {
                    !state.read_entry_ids.contains(&entry.id)
                        && saved.reaction.is_none()
                        && (saved.decision.is_none()
                            || saved.preferences_version != preferences.version
                            || saved.content_version != version)
                };
                needed.then_some(RssCheck {
                    entry,
                    preferences_version: preferences.version,
                    reaction_version: saved.reaction_version,
                    content_version: version,
                })
            })
            .take(limit)
            .collect())
    }

    pub fn apply_decisions(
        &self,
        id: &ItemId,
        checks: &[(RssCheck, RssDecision)],
    ) -> Result<(), EngineError> {
        let _lock = self.operation_lock()?;
        let preferences = self.preferences(id)?;
        let cache = self.load_cache(id)?;
        self.update_state(id, |state| {
            for (check, decision) in checks {
                let entry = state.entries.entry(check.entry.id.clone()).or_default();
                if preferences.enabled()
                    && preferences.version == check.preferences_version
                    && !state.read_entry_ids.contains(&check.entry.id)
                    && entry.reaction_version == check.reaction_version
                    && entry.reaction.is_none()
                    && cache.entries.iter().any(|e| {
                        e.id == check.entry.id && content_version(e) == check.content_version
                    })
                {
                    entry.decision = Some(*decision);
                    entry.preferences_version = check.preferences_version;
                    entry.content_version = check.content_version.clone();
                }
            }
            Ok(())
        })
    }

    /// False means the response is stale; the pending reaction remains queued.
    pub fn apply_learning(
        &mut self,
        id: &ItemId,
        check: &RssCheck,
        likes: &[String],
        dislikes: &[String],
    ) -> Result<bool, EngineError> {
        let _lock = self.operation_lock()?;
        let mut preferences = self.preferences(id)?;
        let (feed, state) = self.feed(id)?;
        if preferences.version != check.preferences_version
            || state
                .entries
                .get(&check.entry.id)
                .is_none_or(|e| e.reaction_version != check.reaction_version)
            || !feed
                .entries
                .iter()
                .any(|e| e.id == check.entry.id && content_version(e) == check.content_version)
        {
            return Ok(false);
        }
        append_unique(&mut preferences.likes, likes)?;
        append_unique(&mut preferences.dislikes, dislikes)?;
        self.save_preferences(id, check.preferences_version, preferences)?;
        self.update_state(id, |state| {
            state
                .entries
                .entry(check.entry.id.clone())
                .or_default()
                .learned_version = check.reaction_version;
            Ok(())
        })?;
        Ok(true)
    }
}

fn append_unique(text: &mut String, additions: &[String]) -> Result<(), EngineError> {
    let mut known = text
        .lines()
        .map(|s| s.trim().to_lowercase())
        .collect::<BTreeSet<_>>();
    for line in additions
        .iter()
        .flat_map(|s| s.lines())
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        if known.insert(line.to_lowercase()) {
            if !text.is_empty() && !text.ends_with('\n') {
                text.push('\n');
            }
            text.push_str(line);
        }
    }
    if text.len() > MAX_PREFERENCE_BYTES {
        return Err(EngineError::InvalidSetting("rss/preferences/size".into()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (tempfile::TempDir, RssEngine, ItemId) {
        let root = tempfile::tempdir().unwrap();
        let mut engine = RssEngine::open(root.path()).unwrap();
        let id = engine
            .create_subscription(
                "https://example.test/feed",
                vec![],
                false,
                "2026-01-01T00:00:00Z",
            )
            .unwrap();
        engine
            .apply_refresh(RssRefreshResult::Fetched {
                item_id: id.clone(),
                cache: RssFeedCache {
                    fetched_at: Some("2026-01-01T00:00:00Z".into()),
                    entries: (0..12)
                        .map(|i| RssEntry {
                            id: format!("entry/{i}"),
                            title: format!("Title {i}"),
                            summary: "RSS data".into(),
                            author: None,
                            published: None,
                            updated: None,
                            link: None,
                        })
                        .collect(),
                    ..RssFeedCache::default()
                },
            })
            .unwrap();
        engine
            .save_preferences(
                &id,
                0,
                RssPreferences {
                    likes: "Rust".into(),
                    ..RssPreferences::default()
                },
            )
            .unwrap();
        (root, engine, id)
    }
    #[test]
    fn schedule_both_formulas_fractional_delay_cap_restart_and_reset() {
        for unread in [0, 1] {
            let mut schedule = RssSchedule::default();
            for i in 0..80 {
                let base = if unread == 0 { 1.5_f64 } else { 2_f64 };
                schedule.finish(1_000_000, unread);
                assert_eq!(
                    schedule.next_check,
                    1_000_000 + ((60.0 + base.powi(i)).min(86400.0) * 1000.0).ceil() as u64
                );
                schedule = serde_json::from_slice(&serde_json::to_vec(&schedule).unwrap()).unwrap();
            }
            schedule.paused = true;
            schedule.ai_used = 99;
            schedule.visit(99);
            assert_eq!(schedule.next_check, 99);
            assert_eq!(schedule.iteration, 0);
            assert!(!schedule.paused);
            assert!(schedule.forced);
            assert_eq!(schedule.ai_used, 0);
        }
    }
    #[test]
    fn pause_boundaries_and_one_forced_cycle_are_persistent() {
        for unread in [98, 99, 100] {
            let mut schedule = RssSchedule::default();
            assert_eq!(schedule.allowed(unread), unread < 99);
            schedule.visit(10);
            assert!(schedule.allowed(unread));
            schedule.finish(20, unread); // All attempts, including HTTP 304/errors, use this path.
            let restarted: RssSchedule =
                serde_json::from_value(serde_json::to_value(&schedule).unwrap()).unwrap();
            assert!(restarted.filter_pending); // Resume filtering, never fetch a second forced cycle.
            schedule.finish_cycle(unread);
            assert!(!schedule.filter_pending);
            assert_eq!(schedule.iteration, 1);
            assert_eq!(schedule.allowed(unread), unread < 99);
            let restored: RssSchedule =
                serde_json::from_value(serde_json::to_value(&schedule).unwrap()).unwrap();
            assert_eq!(schedule, restored);
        }
    }
    #[test]
    fn filtering_keeps_results_rechecks_changed_preferences_and_respects_reads_reactions() {
        let (_root, mut engine, id) = fixture();
        let checks = engine.checks(&id, false).unwrap();
        assert_eq!(checks.len(), 10);
        engine.mark_read(&id, &checks[0].entry.id, "now").unwrap();
        assert!(
            engine
                .react(&id, &checks[1].entry.id, RssReaction::Like)
                .unwrap()
        );
        assert!(
            !engine
                .react(&id, &checks[1].entry.id, RssReaction::Like)
                .unwrap()
        );
        engine
            .apply_decisions(
                &id,
                &checks
                    .iter()
                    .cloned()
                    .map(|c| (c, RssDecision::Hide))
                    .collect::<Vec<_>>(),
            )
            .unwrap();
        let (_, state) = engine.feed(&id).unwrap();
        assert!(!state.hidden(&checks[0].entry.id));
        assert!(!state.hidden(&checks[1].entry.id));
        assert!(state.hidden(&checks[2].entry.id));
        assert_eq!(engine.summaries()[0].unread, 1);
        assert!(engine.checks(&id, false).unwrap().is_empty());
        engine
            .save_preferences(
                &id,
                1,
                RssPreferences {
                    likes: "Science".into(),
                    ..RssPreferences::default()
                },
            )
            .unwrap();
        let rechecks = engine.checks(&id, false).unwrap();
        assert_eq!(rechecks.len(), 8);
        engine
            .apply_decisions(
                &id,
                &rechecks
                    .into_iter()
                    .map(|c| (c, RssDecision::Keep))
                    .collect::<Vec<_>>(),
            )
            .unwrap();
        assert_eq!(engine.summaries()[0].unread, 9);
        assert!(engine.checks(&id, false).unwrap().is_empty());
        engine
            .react(&id, &checks[1].entry.id, RssReaction::Dislike)
            .unwrap();
        engine
            .save_preferences(&id, 2, RssPreferences::default())
            .unwrap();
        assert!(engine.feed(&id).unwrap().1.hidden(&checks[1].entry.id));
        assert!(!engine.feed(&id).unwrap().1.hidden(&checks[2].entry.id));
    }
    #[test]
    fn stale_content_learning_and_manual_edits_never_overwrite_current_state() {
        let (root, mut engine, id) = fixture();
        let old = engine.checks(&id, false).unwrap();
        let (mut cache, _) = engine.feed(&id).unwrap();
        cache.entries[0].summary = "changed".into();
        engine
            .apply_refresh(RssRefreshResult::Fetched {
                item_id: id.clone(),
                cache,
            })
            .unwrap();
        engine
            .apply_decisions(&id, &[(old[0].clone(), RssDecision::Hide)])
            .unwrap();
        assert!(!engine.feed(&id).unwrap().1.hidden(&old[0].entry.id));
        engine
            .react(&id, &old[0].entry.id, RssReaction::Dislike)
            .unwrap();
        let learning = engine.checks(&id, true).unwrap().remove(0);
        engine
            .save_preferences(
                &id,
                1,
                RssPreferences {
                    likes: "Manual text".into(),
                    ..RssPreferences::default()
                },
            )
            .unwrap();
        assert!(
            !engine
                .apply_learning(&id, &learning, &["stale".into()], &[])
                .unwrap()
        );
        let fresh = engine.checks(&id, true).unwrap().remove(0);
        assert!(
            engine
                .apply_learning(
                    &id,
                    &fresh,
                    &["Manual text".into(), "New topic".into(), "new topic".into()],
                    &[]
                )
                .unwrap()
        );
        assert_eq!(
            engine.preferences(&id).unwrap().likes,
            "Manual text\nNew topic"
        );
        assert!(engine.checks(&id, true).unwrap().is_empty());
        let reopened = RssEngine::open(root.path()).unwrap();
        assert_eq!(
            engine.preferences(&id).unwrap(),
            reopened.preferences(&id).unwrap()
        );
        assert!(reopened.feed(&id).unwrap().1.hidden(&old[0].entry.id));
    }
    #[test]
    fn classification_budget_includes_rechecks_and_learning_bypasses_pause() {
        let (_root, engine, id) = fixture();
        engine
            .update_state(&id, |s| {
                s.schedule.ai_used = 98;
                Ok(())
            })
            .unwrap();
        assert_eq!(engine.checks(&id, false).unwrap().len(), 1);
        engine
            .update_state(&id, |s| {
                s.schedule.ai_used = 99;
                s.schedule.paused = true;
                Ok(())
            })
            .unwrap();
        assert!(engine.checks(&id, false).unwrap().is_empty());
        engine.react(&id, "entry/0", RssReaction::Dislike).unwrap();
        assert_eq!(engine.checks(&id, true).unwrap().len(), 1);
    }
    #[test]
    fn size_conflict_unknown_fields_and_read_only_loading() {
        let (root, mut engine, id) = fixture();
        let path = config_path(root.path());
        let mut raw: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        raw["future"] = serde_json::json!({"untouched":true});
        raw["subscriptions"][0]["future"] = serde_json::json!(42);
        raw["subscriptions"][0]["preferences"]["future"] = serde_json::json!(43);
        fs::write(&path, serde_json::to_vec(&raw).unwrap()).unwrap();
        let before = fs::read(&path).unwrap();
        let mut loaded = RssEngine::open(root.path()).unwrap();
        assert_eq!(fs::read(&path).unwrap(), before);
        loaded
            .save_preferences(&id, 1, RssPreferences::default())
            .unwrap();
        let after: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(after["future"], raw["future"]);
        assert_eq!(after["subscriptions"][0]["future"], 42);
        assert_eq!(after["subscriptions"][0]["preferences"]["future"], 43);
        assert!(
            engine
                .save_preferences(&id, 1, RssPreferences::default())
                .is_err()
        );
        assert!(
            loaded
                .save_preferences(
                    &id,
                    2,
                    RssPreferences {
                        likes: "я".repeat(8193),
                        ..RssPreferences::default()
                    }
                )
                .is_err()
        );
    }
}
