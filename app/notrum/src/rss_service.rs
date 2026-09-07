// Copyright 2026 Evgeniy Udodov
// SPDX-License-Identifier: GPL-3.0-only

#![forbid(unsafe_code)]

//! One coordinator per workspace session. Only workers touch credentials/network/save files.
use crate::settings::GlobalSettingsStore;
use notrum_ai::{
    AiError, AiSettings, ApiKey, FilterInput, FilterOutput, GenerationTransport,
    HttpsGenerationTransport,
};
use notrum_core::{
    AI_VISIT_LIMIT, ItemId, RssCheck, RssDecision, RssEngine, RssPreferences, RssReaction,
    RssRefreshResult, execute_rss_refresh,
};
use notrum_platform::credentials::{CredentialStore, SystemCredentials};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{
    Arc, Mutex,
    mpsc::{self, Receiver, SyncSender},
};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

static RSS_REQUESTS: AtomicUsize = AtomicUsize::new(0);
static AI_REQUESTS: AtomicUsize = AtomicUsize::new(0);

struct RequestSlot(&'static AtomicUsize);
impl RequestSlot {
    fn take(counter: &'static AtomicUsize, limit: usize) -> Option<Self> {
        counter
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |used| {
                (used < limit).then_some(used + 1)
            })
            .ok()
            .map(|_| Self(counter))
    }
}
impl Drop for RequestSlot {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

pub(crate) enum Command {
    Visit(ItemId),
    Preferences(ItemId, u64, u64, RssPreferences),
    Reaction(ItemId, String, u64, RssReaction),
    Read(ItemId, String, String),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Status {
    Idle,
    Busy,
    Paused,
    Retry,
    Settings,
    Conflict,
    Saved,
}

pub(crate) struct Snapshot {
    pub engine: RssEngine,
    pub refreshing: BTreeSet<String>,
    pub status: BTreeMap<String, Status>,
    pub saves: BTreeMap<String, (u64, bool)>,
    pub reaction_sequence: u64,
}

pub(crate) struct Service {
    pub sender: SyncSender<Command>,
    pub receiver: Receiver<Snapshot>,
    alive: Arc<Mutex<bool>>,
}

impl Drop for Service {
    fn drop(&mut self) {
        *self.alive.lock().expect("RSS session gate") = false;
    }
}

enum Completion {
    Fetch(ItemId, u64, Result<RssRefreshResult, ()>),
    Ai(
        ItemId,
        Vec<RssCheck>,
        bool,
        AiSettings,
        Result<FilterOutput, AiError>,
    ),
}

impl Service {
    pub fn start(root: PathBuf) -> Self {
        let (sender, commands) = mpsc::sync_channel(64);
        let (snapshots, receiver) = mpsc::sync_channel(1);
        let alive = Arc::new(Mutex::new(true));
        let gate = alive.clone();
        thread::spawn(move || run(root, commands, snapshots, gate));
        Self {
            sender,
            receiver,
            alive,
        }
    }
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u128::from(u64::MAX)) as u64
}

fn settings() -> AiSettings {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    GlobalSettingsStore::load(home.as_deref()).settings.ai
}

struct Coordinator {
    root: PathBuf,
    fetching: BTreeSet<String>,
    cycles: BTreeSet<String>,
    ai_busy: bool,
    status: BTreeMap<String, Status>,
    blocked: BTreeMap<String, AiSettings>,
    retry: BTreeSet<String>,
    saves: BTreeMap<String, (u64, bool)>,
    reaction_sequence: u64,
}

fn run(
    root: PathBuf,
    commands: Receiver<Command>,
    snapshots: SyncSender<Snapshot>,
    alive: Arc<Mutex<bool>>,
) {
    let (results, completions) = mpsc::sync_channel(3);
    let mut coordinator = Coordinator {
        root,
        fetching: BTreeSet::new(),
        cycles: BTreeSet::new(),
        ai_busy: false,
        status: BTreeMap::new(),
        blocked: BTreeMap::new(),
        retry: BTreeSet::new(),
        saves: BTreeMap::new(),
        reaction_sequence: 0,
    };
    let mut dirty = true;
    let mut last_tick = 0;
    loop {
        let command = match commands.recv_timeout(Duration::from_millis(100)) {
            Ok(c) => Some(c),
            Err(mpsc::RecvTimeoutError::Timeout) => None,
            Err(_) => break,
        };
        // Cancellation and durable writes share a gate. A disposed session cannot apply results.
        let active = alive.lock().expect("RSS session gate");
        if !*active {
            break;
        }
        let Ok(mut engine) = RssEngine::open(&coordinator.root) else {
            continue;
        };
        let Ok(_lock) = engine.operation_lock() else {
            continue;
        };
        let Ok(fresh) = RssEngine::open(&coordinator.root) else {
            continue;
        };
        engine = fresh;
        if let Some(command) = command {
            dirty = true;
            coordinator.command(&mut engine, command);
        }
        // User edits/reactions already queued take precedence over arriving AI
        // responses. Drain only the bounded command capacity per turn.
        for _ in 0..64 {
            let Ok(command) = commands.try_recv() else {
                break;
            };
            dirty = true;
            coordinator.command(&mut engine, command);
        }
        while let Ok(result) = completions.try_recv() {
            dirty = true;
            coordinator.complete(&mut engine, result);
        }
        if last_tick != now() / 1000 || dirty {
            last_tick = now() / 1000;
            dirty |= coordinator.dispatch(&mut engine, &results);
        }
        if dirty
            && snapshots
                .try_send(Snapshot {
                    engine,
                    refreshing: coordinator.fetching.clone(),
                    status: coordinator.status.clone(),
                    saves: coordinator.saves.clone(),
                    reaction_sequence: coordinator.reaction_sequence,
                })
                .is_ok()
        {
            dirty = false;
        }
    }
}

impl Coordinator {
    fn command(&mut self, engine: &mut RssEngine, command: Command) {
        let id = match &command {
            Command::Visit(id)
            | Command::Preferences(id, ..)
            | Command::Reaction(id, ..)
            | Command::Read(id, ..) => id.clone(),
        };
        let result = match command {
            Command::Visit(_) => {
                self.cycles.remove(id.as_str());
                self.blocked.remove(id.as_str());
                self.retry.remove(id.as_str());
                engine.update_state(&id, |s| {
                    s.schedule.visit(now());
                    s.ai_error = None;
                    Ok(())
                })
            }
            Command::Preferences(_, token, expected, preferences) => {
                let result = engine.save_preferences(&id, expected, preferences);
                self.saves
                    .insert(id.as_str().into(), (token, result.is_ok()));
                if result.is_ok() {
                    self.cycles.insert(id.as_str().into());
                    self.blocked.remove(id.as_str());
                    self.retry.remove(id.as_str());
                    if engine
                        .update_state(&id, |s| {
                            s.ai_error = None;
                            Ok(())
                        })
                        .is_err()
                    {
                        self.saves.insert(id.as_str().into(), (token, false));
                    }
                }
                result
            }
            Command::Reaction(_, entry, token, reaction) => {
                self.reaction_sequence = self.reaction_sequence.max(token);
                self.retry.remove(id.as_str());
                self.blocked.remove(id.as_str());
                engine.react(&id, &entry, reaction).and_then(|changed| {
                    if changed {
                        engine.update_state(&id, |s| {
                            s.ai_error = None;
                            Ok(())
                        })
                    } else {
                        Ok(())
                    }
                })
            }
            Command::Read(_, entry, timestamp) => {
                engine.mark_read(&id, &entry, &timestamp).map(|_| ())
            }
        };
        self.status.insert(
            id.as_str().into(),
            if result.is_ok() {
                Status::Saved
            } else {
                Status::Conflict
            },
        );
    }

    fn complete(&mut self, engine: &mut RssEngine, result: Completion) {
        match result {
            Completion::Fetch(id, visit, result) => {
                self.fetching.remove(id.as_str());
                if engine.preferences(&id).is_err() {
                    return;
                }
                let success = result.is_ok();
                let apply = result.map_or(Ok(()), |result| engine.apply_refresh(result));
                let Ok((feed, state)) = engine.feed(&id) else {
                    return;
                };
                let unread = state.all_unread(&feed);
                let saved = engine.update_state(&id, |s| {
                    // An explicit visit during an in-flight fetch reuses that fetch.
                    if s.schedule.visit == visit || s.schedule.forced {
                        s.schedule.finish(now(), unread);
                    }
                    Ok(())
                });
                self.retry.remove(id.as_str());
                self.cycles.insert(id.as_str().into());
                self.status.insert(
                    id.as_str().into(),
                    if apply.is_err() || saved.is_err() {
                        Status::Conflict
                    } else if success {
                        Status::Idle
                    } else {
                        Status::Retry
                    },
                );
            }
            Completion::Ai(id, checks, learning, expected_settings, result) => {
                self.ai_busy = false;
                if engine.preferences(&id).is_err() {
                    return;
                }
                if settings() != expected_settings {
                    return;
                }
                match result {
                    Ok(output) => {
                        let result = if learning {
                            engine
                                .apply_learning(&id, &checks[0], &output.likes, &output.dislikes)
                                .map(|_| ())
                        } else {
                            let decisions = checks
                                .into_iter()
                                .zip(output.decisions)
                                .map(|(check, decision)| {
                                    (
                                        check,
                                        match decision {
                                            notrum_ai::FilterDecision::Keep => RssDecision::Keep,
                                            notrum_ai::FilterDecision::Hide => RssDecision::Hide,
                                        },
                                    )
                                })
                                .collect::<Vec<_>>();
                            engine.apply_decisions(&id, &decisions)
                        };
                        self.status.insert(
                            id.as_str().into(),
                            if result.is_ok() {
                                Status::Idle
                            } else {
                                Status::Conflict
                            },
                        );
                        if result.is_err() {
                            self.retry.insert(id.as_str().into());
                        }
                        if learning && result.is_ok() {
                            self.cycles.insert(id.as_str().into());
                        }
                    }
                    Err(error) => {
                        let blocked = !matches!(
                            error,
                            AiError::Network | AiError::RateLimited | AiError::Response
                        );
                        if blocked {
                            self.blocked.insert(id.as_str().into(), expected_settings);
                        } else {
                            self.retry.insert(id.as_str().into());
                        }
                        self.status.insert(
                            id.as_str().into(),
                            if blocked {
                                Status::Settings
                            } else {
                                Status::Retry
                            },
                        );
                        if engine
                            .update_state(&id, |s| {
                                s.ai_error =
                                    Some(if blocked { "settings" } else { "temporary" }.into());
                                s.ai_error_model = s.model_version.clone();
                                s.ai_error_iteration = s.schedule.iteration;
                                Ok(())
                            })
                            .is_err()
                        {
                            self.status.insert(id.as_str().into(), Status::Conflict);
                        }
                    }
                }
            }
        }
    }

    fn dispatch(&mut self, engine: &mut RssEngine, results: &SyncSender<Completion>) -> bool {
        let mut changed = false;
        let ids = engine
            .subscriptions()
            .iter()
            .filter(|s| !s.deleted)
            .map(|s| s.id.clone())
            .collect::<Vec<_>>();
        let current_settings = settings();
        for id in &ids {
            if engine
                .feed(id)
                .is_ok_and(|(_, state)| state.schedule.filter_pending)
            {
                self.cycles.insert(id.as_str().into());
            }
            let Ok(preferences) = engine.preferences(id) else {
                continue;
            };
            let selection = current_settings.resolve(&preferences.alias).ok();
            let model_version = serde_json::to_string(&(
                current_settings
                    .connection
                    .as_ref()
                    .map(|c| (c.provider, &c.credential, c.checked_at)),
                selection,
            ))
            .unwrap_or_default();
            if let Ok((_, state)) = engine.feed(id)
                && state.model_version != model_version
            {
                if engine
                    .update_state(id, |state| {
                        state.model_version = model_version;
                        state.ai_error = None;
                        state.schedule.filter_pending = true;
                        for entry in state.entries.values_mut() {
                            entry.content_version.clear();
                        }
                        Ok(())
                    })
                    .is_ok()
                {
                    self.cycles.insert(id.as_str().into());
                    self.retry.remove(id.as_str());
                    self.blocked.remove(id.as_str());
                    changed = true;
                } else {
                    self.status.insert(id.as_str().into(), Status::Conflict);
                }
            }
            if let Ok((_, state)) = engine.feed(id)
                && let Some(error) = state.ai_error
            {
                if error == "settings" && state.ai_error_model == state.model_version {
                    self.blocked
                        .insert(id.as_str().into(), current_settings.clone());
                    self.status.insert(id.as_str().into(), Status::Settings);
                } else if error == "temporary"
                    && state.ai_error_iteration == state.schedule.iteration
                {
                    self.retry.insert(id.as_str().into());
                    self.status.insert(id.as_str().into(), Status::Retry);
                }
            }
        }
        let unblocked = self
            .blocked
            .iter()
            .filter(|(_, old)| **old != current_settings)
            .map(|(id, _)| id.clone())
            .collect::<Vec<_>>();
        for id in unblocked {
            self.blocked.remove(&id);
            self.cycles.insert(id);
        }
        for id in &ids {
            if self.fetching.len() >= 2 {
                break;
            }
            if self.fetching.contains(id.as_str()) {
                continue;
            }
            let Ok((feed, state)) = engine.feed(id) else {
                continue;
            };
            // A model/preference recheck must not consume a newly requested
            // visit before its forced download has started.
            if self.cycles.contains(id.as_str()) && state.schedule.iteration != 0 {
                continue;
            }
            if state.schedule.next_check > now() {
                continue;
            }
            let mut schedule = state.schedule.clone();
            if !schedule.allowed(state.all_unread(&feed)) {
                if schedule != state.schedule {
                    changed = true;
                    if engine
                        .update_state(id, |s| {
                            s.schedule = schedule;
                            Ok(())
                        })
                        .is_err()
                    {
                        self.status.insert(id.as_str().into(), Status::Conflict);
                    } else {
                        self.status.insert(id.as_str().into(), Status::Paused);
                    }
                }
                continue;
            }
            let Ok(request) = engine.refresh_request(id) else {
                continue;
            };
            let Some(slot) = RequestSlot::take(&RSS_REQUESTS, 2) else {
                break;
            };
            self.fetching.insert(id.as_str().into());
            changed = true;
            let results = results.clone();
            let id = id.clone();
            let visit = state.schedule.visit;
            thread::spawn(move || {
                let _slot = slot;
                let result = execute_rss_refresh(request).map_err(|_| ());
                let _ = results.send(Completion::Fetch(id, visit, result));
            });
        }
        if self.ai_busy {
            return changed;
        }
        // Learning always wins over classification, including for paused feeds.
        for learning in [true, false] {
            for id in &ids {
                if self.blocked.contains_key(id.as_str()) || self.retry.contains(id.as_str()) {
                    continue;
                }
                if !learning
                    && (!self.cycles.contains(id.as_str()) || self.fetching.contains(id.as_str()))
                {
                    continue;
                }
                let Ok(mut checks) = engine.checks(id, learning) else {
                    continue;
                };
                if checks.is_empty() {
                    continue;
                }
                let Some(slot) = RequestSlot::take(&AI_REQUESTS, 1) else {
                    return changed;
                };
                let Ok(preferences) = engine.preferences(id) else {
                    continue;
                };
                let Ok((_, state)) = engine.feed(id) else {
                    continue;
                };
                let reaction = learning.then(|| {
                    state.entries[&checks[0].entry.id].reaction == Some(RssReaction::Like)
                });
                let profile = current_settings.resolve(&preferences.alias).cloned();
                let mut input = FilterInput {
                    likes: preferences.likes,
                    dislikes: preferences.dislikes,
                    entries: Vec::new(),
                    reaction,
                };
                for check in &checks {
                    let mut text = format!("{}\n{}", check.entry.title, check.entry.summary);
                    let mut end = text.len().min(notrum_ai::MAX_RSS_TEXT_BYTES);
                    while !text.is_char_boundary(end) {
                        end -= 1;
                    }
                    text.truncate(end);
                    input.entries.push(text);
                }
                if let (Ok(profile), Some(connection)) = (&profile, &current_settings.connection) {
                    while input.entries.len() > 1
                        && notrum_ai::generation_body(connection.provider, profile, &input).is_err()
                    {
                        input.entries.pop();
                        checks.pop();
                    }
                }
                if !learning
                    && engine
                        .update_state(id, |s| {
                            if !s.schedule.allowed(state.all_unread(&engine.feed(id)?.0))
                                || s.schedule.ai_used.saturating_add(checks.len() as u16)
                                    > AI_VISIT_LIMIT
                            {
                                return Err(notrum_core::EngineError::Conflict);
                            }
                            s.schedule.ai_used += checks.len() as u16;
                            Ok(())
                        })
                        .is_err()
                {
                    continue;
                }
                self.ai_busy = true;
                self.status.insert(id.as_str().into(), Status::Busy);
                let results = results.clone();
                let id = id.clone();
                let settings = current_settings.clone();
                thread::spawn(move || {
                    let _slot = slot;
                    let result = generate(&settings, profile, &input);
                    let _ = results.send(Completion::Ai(id, checks, learning, settings, result));
                });
                return true;
            }
        }
        // No remaining runnable checks: release the forced cycle, including on errors.
        let finished = self
            .cycles
            .iter()
            .filter(|id| !self.fetching.contains(*id))
            .cloned()
            .collect::<Vec<_>>();
        for key in finished {
            let Some(id) = ids.iter().find(|id| id.as_str() == key) else {
                self.cycles.remove(&key);
                continue;
            };
            if let Ok((feed, state)) = engine.feed(id) {
                let unread = state.all_unread(&feed);
                if engine
                    .update_state(id, |s| {
                        s.schedule.finish_cycle(unread);
                        Ok(())
                    })
                    .is_err()
                {
                    self.status.insert(key.clone(), Status::Conflict);
                }
            }
            self.cycles.remove(&key);
            changed = true;
        }
        changed
    }
}

fn generate(
    settings: &AiSettings,
    profile: Result<notrum_ai::AiProfile, AiError>,
    input: &FilterInput,
) -> Result<FilterOutput, AiError> {
    let profile = profile?;
    #[cfg(feature = "test-utils")]
    if std::env::var("NOTRUM_TEST_RSS_AI").as_deref() == Ok("1") {
        thread::sleep(Duration::from_millis(700));
        return Ok(if input.reaction.is_some() {
            FilterOutput {
                decisions: vec![],
                likes: if input.reaction == Some(true) {
                    vec!["Useful articles".into()]
                } else {
                    vec![]
                },
                dislikes: if input.reaction == Some(false) {
                    vec!["Promotions".into()]
                } else {
                    vec![]
                },
            }
        } else {
            FilterOutput {
                decisions: input
                    .entries
                    .iter()
                    .map(|text| {
                        if text.contains("Promotion") {
                            notrum_ai::FilterDecision::Hide
                        } else {
                            notrum_ai::FilterDecision::Keep
                        }
                    })
                    .collect(),
                likes: vec![],
                dislikes: vec![],
            }
        });
    }
    let connection = settings.connection.as_ref().ok_or(AiError::Incomplete)?;
    let value = SystemCredentials
        .read(&connection.credential)
        .map_err(|_| AiError::Unauthorized)?;
    let (provider, key) = ApiKey::parse(value)?;
    if provider != connection.provider {
        return Err(AiError::KeyFormat);
    }
    HttpsGenerationTransport.generate(provider, &profile, &key, input)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn request_slots_survive_session_replacement_and_release_on_drop() {
        static COUNT: AtomicUsize = AtomicUsize::new(0);
        let first = RequestSlot::take(&COUNT, 2).unwrap();
        let second = RequestSlot::take(&COUNT, 2).unwrap();
        assert!(RequestSlot::take(&COUNT, 2).is_none());
        drop(first);
        assert!(RequestSlot::take(&COUNT, 2).is_some());
        drop(second);
        assert_eq!(COUNT.load(Ordering::Acquire), 0);
    }
    #[test]
    fn http_304_and_errors_advance_once_and_visit_reuses_inflight_attempt() {
        let root = std::env::temp_dir().join(format!("rss-coordinator-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let mut engine = RssEngine::open(&root).unwrap();
        let id = engine
            .create_subscription("https://example.test/feed", vec![], false, "now")
            .unwrap();
        let mut c = Coordinator {
            root: root.clone(),
            fetching: BTreeSet::new(),
            cycles: BTreeSet::new(),
            ai_busy: false,
            status: BTreeMap::new(),
            blocked: BTreeMap::new(),
            retry: BTreeSet::new(),
            saves: BTreeMap::new(),
            reaction_sequence: 0,
        };
        c.command(&mut engine, Command::Visit(id.clone()));
        c.fetching.insert(id.as_str().into());
        c.command(&mut engine, Command::Visit(id.clone()));
        assert_eq!(c.fetching.len(), 1);
        c.complete(
            &mut engine,
            Completion::Fetch(
                id.clone(),
                1,
                Ok(RssRefreshResult::NotModified {
                    item_id: id.clone(),
                    fetched_at: "now".into(),
                }),
            ),
        );
        assert_eq!(engine.feed(&id).unwrap().1.schedule.iteration, 1);
        assert!(c.fetching.is_empty());
        c.complete(&mut engine, Completion::Fetch(id.clone(), 2, Err(())));
        assert_eq!(engine.feed(&id).unwrap().1.schedule.iteration, 2);
        assert_eq!(c.status[id.as_str()], Status::Retry);
        std::fs::remove_dir_all(root).unwrap();
    }
}
