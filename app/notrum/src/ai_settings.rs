// Copyright 2026 Evgeniy Udodov
// SPDX-License-Identifier: GPL-3.0-only

#![forbid(unsafe_code)]

use crate::ai_service::{Action, Failure};
use crate::*;
use notrum_ai::{AiError, AiModel, AiProfile, AiSettings, AiTaskSize, detect_provider};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering as AtomicOrdering},
};

#[derive(Clone)]
struct Controller {
    settings: RwSignal<AiSettings>,
    busy: RwSignal<bool>,
    feedback: RwSignal<Option<i18n::Key>>,
    connection_open: RwSignal<bool>,
    expanded: RwSignal<Option<AiTaskSize>>,
    scroll_target: RwSignal<Option<ViewId>>,
    key: Rc<RefCell<Zeroizing<String>>>,
    key_revision: RwSignal<u64>,
    visible_key: RwSignal<bool>,
    generation: Rc<Cell<u64>>,
    cancellation: Rc<RefCell<Arc<AtomicBool>>>,
    global: Rc<RefCell<GlobalSettingsStore>>,
}

impl Controller {
    fn clear_key(&self) {
        self.key.borrow_mut().zeroize();
        self.key_revision.update(|revision| *revision += 1);
        self.visible_key.set(false);
    }

    fn submit(&self, action: Action) {
        if self.busy.get_untracked() {
            return;
        }
        let Some(home) = self.global.borrow().home() else {
            self.feedback.set(Some(i18n::Key::AiSettingsError));
            return;
        };
        let expected = self.settings.get_untracked();
        let next = match &action {
            Action::Save(size, _) => AiTaskSize::ALL
                .into_iter()
                .find(|candidate| candidate != size && expected.profile(*candidate).is_err()),
            _ => None,
        };
        let save = matches!(action, Action::Save(..));
        let connecting = matches!(action, Action::Connect(..));
        self.busy.set(true);
        self.feedback.set(None);
        let generation = self.generation.get();
        let cancellation = Arc::new(AtomicBool::new(false));
        *self.cancellation.borrow_mut() = cancellation.clone();
        let (sender, receiver) = mpsc::channel();
        thread::spawn(move || {
            #[cfg(feature = "test-utils")]
            if std::env::var("NOTRUM_TEST_AI").as_deref() == Ok("1") {
                let result = crate::ai_service::execute(
                    &home,
                    expected,
                    action,
                    &cancellation,
                    &crate::ai_service::fixtures::Catalog,
                    &crate::ai_service::fixtures::Vault,
                );
                let _ = sender.send(result);
                return;
            }
            let result = crate::ai_service::execute(
                &home,
                expected,
                action,
                &cancellation,
                &notrum_ai::HttpsCatalogTransport,
                &notrum_platform::credentials::SystemCredentials,
            );
            let _ = sender.send(result);
        });
        poll(
            self.clone(),
            Rc::new(receiver),
            generation,
            next,
            save,
            connecting,
        );
    }
}

fn poll(
    controller: Controller,
    receiver: Rc<Receiver<Result<AiSettings, Failure>>>,
    generation: u64,
    next: Option<AiTaskSize>,
    save: bool,
    connecting: bool,
) {
    exec_after(Duration::from_millis(50), move |_| {
        match receiver.try_recv() {
            Ok(result) => {
                controller.busy.set(false);
                let stale = controller.generation.get() != generation;
                // The disk commit may have completed just before cancellation.
                let home = controller.global.borrow().home();
                let loaded = GlobalSettingsStore::load(home.as_deref());
                controller.settings.set(loaded.settings.ai.clone());
                *controller.global.borrow_mut() = loaded.store;
                if stale {
                    return;
                }
                match result {
                    Ok(settings) => {
                        controller
                            .feedback
                            .set(if settings.pending_deletions.is_empty() {
                                None
                            } else {
                                Some(i18n::Key::AiCleanupError)
                            });
                        controller
                            .connection_open
                            .set(settings.connection.is_none());
                        if connecting {
                            controller.clear_key();
                            controller.expanded.set(
                                AiTaskSize::ALL
                                    .into_iter()
                                    .find(|size| settings.profile(*size).is_err()),
                            );
                        } else if save {
                            controller.expanded.set(next);
                        }
                    }
                    Err(error) => controller.feedback.set(Some(error_key(error))),
                }
            }
            Err(mpsc::TryRecvError::Empty) => {
                poll(controller, receiver, generation, next, save, connecting)
            }
            Err(mpsc::TryRecvError::Disconnected) => {
                controller.busy.set(false);
                controller.feedback.set(Some(i18n::Key::AiNetworkError));
            }
        }
    });
}

fn error_key(error: Failure) -> i18n::Key {
    use i18n::Key as K;
    match error {
        Failure::Credentials => K::AiCredentialsError,
        Failure::Settings => K::AiSettingsError,
        Failure::Cancelled => K::Cancel,
        Failure::Cleanup => K::AiCleanupError,
        Failure::Api(error) => match error {
            AiError::KeyFormat => K::AiKeyFormat,
            AiError::Unauthorized => K::AiUnauthorized,
            AiError::Forbidden => K::AiForbidden,
            AiError::RateLimited => K::AiRateLimited,
            AiError::Network => K::AiNetworkError,
            AiError::Response => K::AiResponseError,
            AiError::NoModels => K::AiNoModels,
            AiError::ModelUnavailable => K::AiUnavailable,
            AiError::EffortRequired => K::AiChooseEffort,
            AiError::Incomplete => K::AiConnectFirst,
        },
    }
}

/// The page heading of a settings section, matching the general and
/// encryption pages.
fn page_title(key: i18n::Key, palette: Palette) -> impl IntoView {
    label(move || key.to_string()).style(move |style| {
        style
            .font_size(26.0)
            .font_weight(floem::text::Weight::SEMIBOLD)
            .color(palette.ink)
            .selectable(false)
    })
}

fn page_description(key: i18n::Key, palette: Palette) -> impl IntoView {
    label(move || key.to_string())
        .style(move |style| style.font_size(13.5).color(palette.muted).selectable(false))
}

/// A step of the page: the connection and the task profiles are two sections
/// of one page, titled like the cards of the general settings page.
fn section_title(key: i18n::Key, palette: Palette) -> impl IntoView {
    label(move || key.to_string()).style(move |style| {
        style
            .font_size(15.0)
            .font_weight(floem::text::Weight::SEMIBOLD)
            .color(palette.ink)
            .selectable(false)
    })
}

fn spacer(height: f64) -> impl IntoView {
    empty().style(move |style| style.height(height))
}

/// One row of form actions. Buttons keep their own width instead of
/// stretching across the card, and wrap before they shrink.
fn actions(children: impl IntoView + 'static) -> impl IntoView {
    v_stack((children,)).style(|style| {
        rtl_row(style)
            .gap(8.0)
            .items_center()
            .flex_wrap(floem::taffy::FlexWrap::Wrap)
    })
}

pub(super) fn page(
    signals: SettingsPageSignals,
    global: Rc<RefCell<GlobalSettingsStore>>,
    palette: Palette,
) -> impl IntoView {
    let initial = global.borrow().ai();
    let controller = Controller {
        settings: create_rw_signal(initial),
        busy: create_rw_signal(false),
        feedback: create_rw_signal(None),
        connection_open: create_rw_signal(true),
        expanded: create_rw_signal(None),
        scroll_target: create_rw_signal(None),
        key: Rc::new(RefCell::new(Zeroizing::new(String::with_capacity(4096)))),
        key_revision: create_rw_signal(0),
        visible_key: create_rw_signal(false),
        generation: Rc::new(Cell::new(0)),
        cancellation: Rc::new(RefCell::new(Arc::new(AtomicBool::new(false)))),
        global,
    };
    let lifecycle = controller.clone();
    let was_visible = Rc::new(Cell::new(false));
    create_effect(move |_| {
        let visible = signals.open.get() && signals.section.get() == SettingsSection::Ai;
        if was_visible.replace(visible) == visible {
            return;
        }
        lifecycle.generation.set(lifecycle.generation.get() + 1);
        lifecycle
            .cancellation
            .borrow()
            .store(true, AtomicOrdering::Release);
        lifecycle.clear_key();
        lifecycle.feedback.set(None);
        if visible {
            let home = lifecycle.global.borrow().home();
            let loaded = GlobalSettingsStore::load(home.as_deref());
            let settings = loaded.settings.ai.clone();
            lifecycle.connection_open.set(settings.connection.is_none());
            lifecycle.settings.set(settings.clone());
            lifecycle.expanded.set(
                AiTaskSize::ALL
                    .into_iter()
                    .find(|size| settings.profile(*size).is_err()),
            );
            *lifecycle.global.borrow_mut() = loaded.store;
            if loaded.diagnostic.is_some() {
                lifecycle.feedback.set(Some(i18n::Key::AiSettingsError));
            }
        }
    });
    let settings = controller.settings;
    let feedback = controller.feedback;
    let busy = controller.busy;
    let connection_open = controller.connection_open;
    // Keep form signals alive while changing visibility. Rebuilding nested
    // dynamic containers can leave queued list updates referencing disposed drafts.
    let connection = v_stack((
        connection_form(controller.clone(), palette)
            .style(move |style| style.apply_if(!connection_open.get(), |style| style.hide())),
        connection_summary(controller.clone(), palette)
            .style(move |style| style.apply_if(connection_open.get(), |style| style.hide())),
    ))
    .style(|style| style.width_full());
    // One line for both states of the section: the request in flight and the
    // error it ended with never move the form below them.
    let status = label(move || {
        if busy.get() {
            tr!(AiWorking)
        } else {
            feedback
                .get()
                .map(|key| key.to_string())
                .unwrap_or_default()
        }
    })
    .style(move |style| {
        style
            .width_full()
            .max_width(SETTINGS_CARD_MAX_WIDTH_PX)
            .min_height(18.0)
            .font_size(12.5)
            .line_height(1.4)
            .color(if busy.get() {
                palette.accent
            } else {
                palette.danger
            })
            .selectable(false)
    });
    let cleanup_controller = controller.clone();
    let cleanup = actions(action_button(
        move || tr!(AiRetryCleanup),
        IconButtonTone::Danger,
        palette,
        move || !busy.get(),
        move || cleanup_controller.submit(Action::Cleanup),
    ))
    .style(move |style| {
        style
            .margin_top(10.0)
            .apply_if(settings.get().pending_deletions.is_empty(), |style| {
                style.hide()
            })
    });
    let profiles = v_stack((
        v_stack((
            profile_card(AiTaskSize::Small, controller.clone(), palette),
            profile_card(AiTaskSize::Medium, controller.clone(), palette),
            profile_card(AiTaskSize::Large, controller.clone(), palette),
        ))
        .style(move |style| {
            rtl_column(style)
                .width_full()
                .gap(12.0)
                .apply_if(settings.get().connection.is_none(), |style| style.hide())
        }),
        h_stack((
            svg(ICON_LOCK).style(|style| style.size(16.0, 16.0)),
            settings_hint(i18n::Key::AiConnectFirst, palette),
        ))
        .style(move |style| {
            rtl_row(settings_card_style(style, palette))
                .items_center()
                .gap(10.0)
                .color(palette.muted)
                .apply_if(settings.get().connection.is_some(), |style| style.hide())
        }),
    ))
    .style(|style| style.width_full());
    let scroll_target = controller.scroll_target;
    let connection_section = v_stack((
        section_title(i18n::Key::AiConnect, palette),
        spacer(7.0),
        settings_hint(i18n::Key::AiKeyStorage, palette)
            .style(|style| style.max_width(SETTINGS_CARD_MAX_WIDTH_PX)),
        spacer(12.0),
        connection,
        spacer(10.0),
        status,
        cleanup,
    ))
    .style(|style| rtl_column(style).width_full());
    let models_section = v_stack((
        section_title(i18n::Key::AiModels, palette),
        spacer(7.0),
        label(move || msg!(AiProgress, "count" => settings.get().configured_count()).render())
            .style(move |style| {
                style
                    .font_size(12.5)
                    .selectable(false)
                    .color(if settings.get().ready() {
                        palette.accent
                    } else {
                        palette.muted
                    })
            }),
        spacer(12.0),
        profiles,
    ))
    .style(|style| rtl_column(style).width_full());
    scroll(
        v_stack((
            page_title(i18n::Key::AiAssistant, palette),
            spacer(7.0),
            page_description(i18n::Key::AiDescription, palette),
            spacer(28.0),
            connection_section,
            spacer(24.0),
            models_section,
        ))
        .style(|style| {
            rtl_column(style)
                .width_full()
                .padding_horiz(44.0)
                .padding_vert(38.0)
        }),
    )
    .scroll_to_view(move || scroll_target.get())
    .style(move |style| {
        style
            .min_width(0.0)
            .height_full()
            .flex_grow(1.0)
            .background(palette.canvas)
    })
}

fn connection_summary(controller: Controller, palette: Palette) -> impl IntoView {
    let settings = controller.settings;
    let busy = controller.busy;
    let edit = controller.clone();
    let refresh = controller;
    v_stack((
        v_stack((
            label(move || {
                settings
                    .get()
                    .connection
                    .map(|connection| connection.provider.name().to_owned())
                    .unwrap_or_default()
            })
            .style(move |style| {
                style
                    .font_size(14.0)
                    .font_weight(floem::text::Weight::SEMIBOLD)
                    .color(palette.ink)
                    .selectable(false)
            }),
            label(move || tr!(AiKeySaved))
                .style(move |style| style.font_size(12.5).color(palette.accent).selectable(false)),
            label(move || {
                settings
                    .get()
                    .connection
                    .map(|connection| {
                        let date = chrono::DateTime::from_timestamp(connection.checked_at as i64, 0)
                            .map(|time| {
                                time.with_timezone(&chrono::Local)
                                    .format("%Y-%m-%d %H:%M")
                                    .to_string()
                            })
                            .unwrap_or_default();
                        msg!(AiLastChecked, "value" => date).render()
                    })
                    .unwrap_or_default()
            })
            .style(move |style| style.font_size(12.0).color(palette.muted).selectable(false)),
        ))
        .style(|style| rtl_column(style).width_full().gap(4.0)),
        actions((
            action_button(
                move || tr!(AiEdit),
                IconButtonTone::Secondary,
                palette,
                move || !busy.get(),
                move || {
                    edit.clear_key();
                    edit.connection_open.set(true);
                },
            ),
            action_button(
                move || tr!(AiRefreshModels),
                IconButtonTone::Secondary,
                palette,
                move || !busy.get(),
                move || refresh.submit(Action::Refresh),
            ),
        )),
    ))
    .style(move |style| settings_card_style(style, palette).gap(14.0))
}

fn connection_form(controller: Controller, palette: Palette) -> impl IntoView {
    let busy = controller.busy;
    let settings = controller.settings;
    let revision = controller.key_revision;
    let provider_key = controller.key.clone();
    let provider_state = controller.key.clone();
    // One line under the field: the provider read from the key, or why the key
    // was not recognized.
    let provider_hint = label(move || {
        revision.get();
        let key = provider_key.borrow();
        if key.is_empty() {
            tr!(AiDetectHint)
        } else {
            detect_provider(&key)
                .map(|provider| provider.name().to_owned())
                .unwrap_or_else(|| tr!(AiKeyFormat))
        }
    })
    .style(move |style| {
        revision.get();
        let key = provider_state.borrow();
        let rejected = !key.is_empty() && detect_provider(&key).is_none();
        style
            .width_full()
            .font_size(12.5)
            .line_height(1.4)
            .selectable(false)
            .color(if rejected {
                palette.danger
            } else {
                palette.muted
            })
    });
    let warning_key = controller.key.clone();
    let warning_visible = controller.key.clone();
    let warning = label(move || {
        revision.get();
        let changed = settings.get().connection.is_some_and(|old| {
            detect_provider(&warning_key.borrow()).is_some_and(|provider| provider != old.provider)
        });
        if changed {
            tr!(AiProviderChange)
        } else {
            String::new()
        }
    })
    .style(move |style| {
        revision.get();
        let changed = settings.get().connection.is_some_and(|old| {
            detect_provider(&warning_visible.borrow())
                .is_some_and(|provider| provider != old.provider)
        });
        style
            .width_full()
            .font_size(12.5)
            .line_height(1.4)
            .color(palette.danger)
            .selectable(false)
            .apply_if(!changed, |style| style.hide())
    });
    let secret = secret_input(controller.clone(), palette);
    let submit = controller.clone();
    let cancel = controller.clone();
    let delete = controller.clone();
    let valid_key = controller.key.clone();
    v_stack((
        v_stack((settings_field_label(i18n::Key::AiKeyLabel, palette), secret))
            .style(|style| rtl_column(style).width_full().gap(7.0)),
        provider_hint,
        warning,
        actions((
            action_button(
                move || tr!(AiVerify),
                IconButtonTone::Primary,
                palette,
                move || {
                    revision.get();
                    !busy.get() && detect_provider(&valid_key.borrow()).is_some()
                },
                move || {
                    let key = submit.key.borrow().clone();
                    submit.submit(Action::Connect(key));
                },
            ),
            action_button(
                move || tr!(Cancel),
                IconButtonTone::Secondary,
                palette,
                move || !busy.get(),
                move || {
                    cancel.clear_key();
                    cancel.feedback.set(None);
                    cancel
                        .connection_open
                        .set(cancel.settings.get_untracked().connection.is_none());
                },
            ),
            action_button(
                move || tr!(AiDisconnect),
                IconButtonTone::Danger,
                palette,
                move || !busy.get(),
                move || {
                    delete.clear_key();
                    delete.submit(Action::Disconnect);
                },
            )
            .style(move |style| {
                style.apply_if(settings.get().connection.is_none(), |style| style.hide())
            }),
        )),
    ))
    .style(move |style| settings_card_style(style, palette).gap(12.0))
}

fn secret_input(controller: Controller, palette: Palette) -> impl IntoView {
    let revision = controller.key_revision;
    let visible = controller.visible_key;
    let busy = controller.busy;
    let display = controller.key.clone();
    let entry = controller.key.clone();
    let empty_key = controller.key.clone();
    let selected = Rc::new(Cell::new(false));
    let selected_style = selected.clone();
    let field = MaskedPasswordView::new(
        label(move || {
            revision.get();
            let key = display.borrow();
            if key.is_empty() {
                tr!(AiPasteCredential)
            } else if visible.get() {
                key.to_string()
            } else {
                "•".repeat(key.len().min(32))
            }
        })
        .style(|style| style.min_width(0.0).max_width_full().selectable(false)),
        || {},
        move |event| {
            if busy.get_untracked() {
                return EventPropagation::Stop;
            }
            let Event::KeyDown(event) = event else {
                return EventPropagation::Stop;
            };
            let shortcut = event.modifiers.meta() || event.modifiers.control();
            match &event.key.logical_key {
                Key::Named(NamedKey::Tab) => return EventPropagation::Continue,
                Key::Named(NamedKey::Escape) => {
                    entry.borrow_mut().zeroize();
                    selected.set(false);
                }
                Key::Named(NamedKey::Backspace) | Key::Named(NamedKey::Delete) => {
                    if selected.replace(false) {
                        entry.borrow_mut().zeroize();
                    } else {
                        entry.borrow_mut().pop();
                    }
                }
                Key::Character(value) if shortcut && value.eq_ignore_ascii_case("a") => {
                    selected.set(true)
                }
                Key::Character(value) if shortcut && value.eq_ignore_ascii_case("v") => {
                    if let Ok(value) = Clipboard::get_contents() {
                        let value = Zeroizing::new(value);
                        if value.trim().len() <= 4096 {
                            entry.borrow_mut().zeroize();
                            entry.borrow_mut().push_str(value.trim());
                            selected.set(false);
                        }
                    }
                }
                Key::Character(value) if !shortcut => {
                    if selected.replace(false) {
                        entry.borrow_mut().zeroize();
                    }
                    let mut entry = entry.borrow_mut();
                    if entry.len() + value.len() <= 4096
                        && value.is_ascii()
                        && !value.chars().any(char::is_control)
                    {
                        entry.push_str(value);
                    }
                }
                _ => return EventPropagation::Stop,
            }
            revision.update(|r| *r += 1);
            EventPropagation::Stop
        },
    )
    .style(move |style| {
        revision.get();
        let empty = empty_key.borrow().is_empty();
        settings_secret_style(style, palette, empty, false).apply_if(selected_style.get(), |style| {
            style.background(palette.accent_soft)
        })
    })
    .style(move |style| style.focus(move |style| style.border_color(palette.accent)))
    .keyboard_navigable();
    let reveal_key = controller.key.clone();
    let paste_key = controller.key.clone();
    let paste = controller;
    v_stack((
        field,
        actions((
            action_button(
                move || {
                    if visible.get() {
                        tr!(AiConcealCredential)
                    } else {
                        tr!(AiRevealCredential)
                    }
                },
                IconButtonTone::Secondary,
                palette,
                move || {
                    revision.get();
                    !busy.get() && !reveal_key.borrow().is_empty()
                },
                move || visible.update(|show| *show = !*show),
            ),
            action_button(
                move || tr!(AiPaste),
                IconButtonTone::Secondary,
                palette,
                move || !busy.get(),
                move || match Clipboard::get_contents() {
                    Ok(value) => {
                        let value = Zeroizing::new(value);
                        if value.trim().len() <= 4096 {
                            paste_key.borrow_mut().zeroize();
                            paste_key.borrow_mut().push_str(value.trim());
                            revision.update(|r| *r += 1);
                        } else {
                            paste.feedback.set(Some(i18n::Key::AiKeyFormat));
                        }
                    }
                    Err(_) => paste.feedback.set(Some(i18n::Key::PastePasswordFailed)),
                },
            ),
        )),
    ))
    .style(|style| rtl_column(style).width_full().gap(8.0))
}

fn size_key(size: AiTaskSize) -> i18n::Key {
    match size {
        AiTaskSize::Small => i18n::Key::AiSmall,
        AiTaskSize::Medium => i18n::Key::AiMedium,
        AiTaskSize::Large => i18n::Key::AiLarge,
    }
}

fn profile_card(size: AiTaskSize, controller: Controller, palette: Palette) -> impl IntoView {
    let settings = controller.settings;
    let expanded = controller.expanded;
    let busy = controller.busy;
    let scroll_target = controller.scroll_target;
    let form_controller = controller;
    let view = v_stack((
        reliable_button(
            h_stack((
                v_stack((
                    label(move || size_key(size).to_string()).style(move |style| {
                        style.font_size(14.0).color(palette.ink).selectable(false)
                    }),
                    label(move || {
                        let settings = settings.get();
                        match settings.profiles.get(&size) {
                            Some(profile) => {
                                let name = settings
                                    .connection
                                    .as_ref()
                                    .and_then(|connection| {
                                        connection
                                            .models
                                            .iter()
                                            .find(|model| model.id == profile.model)
                                    })
                                    .map(|model| model.name.as_str())
                                    .unwrap_or(&profile.model);
                                let effort = profile
                                    .effort
                                    .map(|effort| effort.name().to_owned())
                                    .unwrap_or_else(|| tr!(AiManaged));
                                if settings.profile(size).is_ok() {
                                    format!("{name} · {effort}")
                                } else {
                                    format!("{name} · {effort} · {}", tr!(AiUnavailable))
                                }
                            }
                            None => tr!(AiNotConfigured),
                        }
                    })
                    .style(move |style| {
                        let settings = settings.get();
                        style
                            .width_full()
                            .font_size(12.0)
                            .selectable(false)
                            .color(if settings.profile(size).is_ok() {
                                palette.muted
                            } else if settings.profiles.contains_key(&size) {
                                palette.danger
                            } else {
                                palette.muted
                            })
                    }),
                ))
                .style(|style| rtl_column(style).min_width(0.0).flex_grow(1.0).gap(4.0)),
                label(move || {
                    if expanded.get() == Some(size) {
                        tr!(AiCollapse)
                    } else {
                        tr!(AiEdit)
                    }
                })
                .style(move |style| style.font_size(12.5).color(palette.accent).selectable(false)),
            ))
            .style(|style| {
                rtl_row(style)
                    .width_full()
                    .justify_between()
                    .items_center()
                    .gap(12.0)
            }),
            move || {
                if !busy.get_untracked() {
                    expanded.update(|open| {
                        *open = if *open == Some(size) {
                            None
                        } else {
                            Some(size)
                        }
                    });
                }
            },
        )
        .style(move |style| {
            style
                .width_full()
                .cursor(CursorStyle::Pointer)
                .focus(|style| style.outline(1.0).outline_color(palette.accent))
        }),
        profile_form(size, form_controller, palette).style(move |style| {
            style
                .margin_top(14.0)
                .padding_top(14.0)
                .border_top(1.0)
                .border_color(palette.divider)
                .apply_if(expanded.get() != Some(size), |style| style.hide())
        }),
    ))
    .style(move |style| {
        settings_card_style(style, palette).border_color(if expanded.get() == Some(size) {
            palette.accent
        } else {
            palette.divider
        })
    });
    let id = view.id();
    create_effect(move |_| {
        if expanded.get() == Some(size) {
            exec_after(Duration::from_millis(100), move |_| {
                if expanded.get_untracked() == Some(size) {
                    scroll_target.set(Some(id));
                }
            });
        }
    });
    view
}

fn profile_form(size: AiTaskSize, controller: Controller, palette: Palette) -> impl IntoView {
    let settings = controller.settings;
    let busy = controller.busy;
    let saved = settings.get_untracked().profiles.get(&size).cloned();
    let model = create_rw_signal(saved.as_ref().map(|p| p.model.clone()));
    let effort = create_rw_signal(saved.and_then(|p| p.effort));
    let search = create_rw_signal(String::new());
    let choosing = create_rw_signal(false);
    let changed_effort = create_rw_signal(false);
    let expanded = controller.expanded;
    create_effect(move |_| {
        expanded.get();
        let saved = settings.get_untracked().profiles.get(&size).cloned();
        model.set(saved.as_ref().map(|profile| profile.model.clone()));
        effort.set(saved.and_then(|profile| profile.effort));
        search.set(String::new());
        choosing.set(false);
        changed_effort.set(false);
    });
    let choices = dyn_stack(
        move || {
            let query = search.get().to_lowercase();
            settings
                .get()
                .connection
                .map(|c| {
                    c.models
                        .into_iter()
                        .filter(|m| {
                            m.name.to_lowercase().contains(&query)
                                || m.id.to_lowercase().contains(&query)
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default()
        },
        |item| item.clone(),
        move |item: AiModel| {
            let id = item.id.clone();
            let selected_id = id.clone();
            reliable_button(
                label(move || item.name.clone()).style(|style| style.selectable(false)),
                move || {
                    if busy.get_untracked() {
                        return;
                    }
                    let previous = effort.get_untracked();
                    let compatible = previous.is_some_and(|e| item.efforts.contains(&e));
                    if !compatible {
                        effort.set(None);
                    }
                    changed_effort.set(previous.is_some() && !compatible);
                    model.set(Some(id.clone()));
                    choosing.set(false);
                    search.set(String::new());
                },
            )
            .style(move |style| {
                style
                    .width_full()
                    .padding_horiz(12.0)
                    .padding_vert(9.0)
                    .font_size(13.0)
                    .cursor(CursorStyle::Pointer)
                    .color(palette.ink)
                    .background(if model.get().as_ref() == Some(&selected_id) {
                        palette.accent_soft
                    } else {
                        palette.paper
                    })
                    .hover(|style| style.background(palette.accent_soft))
                    .focus(|style| style.background(palette.accent_soft))
            })
        },
    )
    .style(|style| rtl_column(style).flex_col().width_full());
    let selected = label(move || {
        let current = model.get();
        settings
            .get()
            .connection
            .and_then(|c| {
                c.models
                    .into_iter()
                    .find(|m| Some(&m.id) == current.as_ref())
            })
            .map(|m| m.name)
            .unwrap_or_else(|| current.unwrap_or_else(|| tr!(AiChooseModel)))
    })
    .style(move |style| {
        style
            .min_width(0.0)
            .selectable(false)
            .color(if model.get().is_some() {
                palette.ink
            } else {
                palette.muted
            })
    });
    let available = move || {
        settings.get().connection.and_then(|c| {
            c.models
                .into_iter()
                .find(|m| Some(&m.id) == model.get().as_ref())
        })
    };
    let effort_choices = dyn_stack(
        move || available().map(|m| m.efforts).unwrap_or_default(),
        |value| value.name(),
        move |value| {
            reliable_button(
                text(value.name()).style(|style| style.selectable(false)),
                move || {
                    if !busy.get_untracked() {
                        effort.set(Some(value));
                        changed_effort.set(false);
                    }
                },
            )
            .style(move |style| {
                let chosen = effort.get() == Some(value);
                style
                    .height(30.0)
                    .padding_horiz(12.0)
                    .items_center()
                    .justify_center()
                    .font_size(12.5)
                    .cursor(CursorStyle::Pointer)
                    .border_radius(5.0)
                    .border(1.0)
                    .border_color(if chosen { palette.accent } else { palette.divider })
                    .color(if chosen { palette.accent } else { palette.ink })
                    .background(if chosen {
                        palette.accent_soft
                    } else {
                        palette.paper
                    })
                    .hover(move |style| style.background(palette.accent_soft))
                    .focus(|style| style.outline(1.0).outline_color(palette.accent))
            })
        },
    )
    .style(|style| {
        rtl_row(style)
            .gap(6.0)
            .flex_wrap(floem::taffy::FlexWrap::Wrap)
    });
    let save = controller.clone();
    let cancel = controller;
    v_stack((
        v_stack((
            settings_field_label(i18n::Key::AiModelLabel, palette),
            reliable_button(
                h_stack((
                    selected,
                    svg(ICON_CHEVRON_DOWN).style(|style| style.size(12.0, 12.0)),
                ))
                .style(|style| {
                    rtl_row(style)
                        .width_full()
                        .min_width(0.0)
                        .items_center()
                        .justify_between()
                        .gap(8.0)
                }),
                move || {
                    if !busy.get_untracked() {
                        choosing.update(|open| *open = !*open);
                    }
                },
            )
            .style(move |style| {
                settings_control_style(style, palette)
                    .cursor(CursorStyle::Pointer)
                    .border_color(if choosing.get() {
                        palette.accent
                    } else {
                        palette.divider
                    })
                    .focus(|style| style.border_color(palette.accent))
            }),
        ))
        .style(|style| rtl_column(style).width_full().gap(7.0)),
        v_stack((
            localized_input::LocalizedInput::new(search, i18n::Key::AiSearchModels)
                .style(move |style| settings_input_style(style, palette)),
            scroll(choices).style(move |style| {
                style
                    .width_full()
                    .max_height(180.0)
                    .background(palette.paper)
                    .border(1.0)
                    .border_color(palette.divider)
                    .border_radius(6.0)
            }),
        ))
        .style(move |style| {
            rtl_column(style)
                .width_full()
                .gap(8.0)
                .apply_if(!choosing.get(), |style| style.hide())
        }),
        v_stack((
            label(move || match available() {
                Some(m) if m.efforts.is_empty() => tr!(AiManaged),
                _ if effort.get().is_none() => tr!(AiChooseEffort),
                _ => tr!(AiEffortLabel),
            })
            .style(move |style| {
                style
                    .font_size(12.5)
                    .selectable(false)
                    .color(if effort.get().is_none() {
                        palette.ink
                    } else {
                        palette.muted
                    })
            }),
            effort_choices,
            settings_hint(i18n::Key::AiEffortHint, palette).style(move |style| {
                style.apply_if(
                    available().is_none_or(|model| model.efforts.is_empty()),
                    |style| style.hide(),
                )
            }),
            label(move || tr!(AiEffortReset))
                .style(move |style| {
                    style
                        .width_full()
                        .font_size(12.5)
                        .line_height(1.4)
                        .color(palette.danger)
                        .selectable(false)
                })
                .style(move |style| style.apply_if(!changed_effort.get(), |style| style.hide())),
        ))
        .style(move |style| {
            rtl_column(style)
                .width_full()
                .gap(8.0)
                .apply_if(available().is_none(), |style| style.hide())
        }),
        actions((
            action_button(
                move || tr!(Save),
                IconButtonTone::Primary,
                palette,
                move || {
                    !busy.get()
                        && model.get().is_some_and(|model| {
                            settings
                                .get()
                                .validate_profile(&AiProfile {
                                    model,
                                    effort: effort.get(),
                                })
                                .is_ok()
                        })
                },
                move || {
                    if let Some(model) = model.get_untracked() {
                        save.submit(Action::Save(
                            size,
                            AiProfile {
                                model,
                                effort: effort.get_untracked(),
                            },
                        ));
                    }
                },
            ),
            action_button(
                move || tr!(Cancel),
                IconButtonTone::Secondary,
                palette,
                move || !busy.get(),
                move || cancel.expanded.set(None),
            ),
        )),
    ))
    .style(|style| rtl_column(style).width_full().gap(14.0))
}
