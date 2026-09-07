// Copyright 2026 Evgeniy Udodov
// SPDX-License-Identifier: GPL-3.0-only

#![forbid(unsafe_code)]

use crate::*;
use notrum_core::RssPreferences;

pub(crate) const LIKE: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linejoin="round"><path d="M3 10h4v11H3zM7 10l5-8h1c2 0 2 3 1 6h5c2 0 2 2 2 3l-2 8c0 1-1 2-2 2H7"/></svg>"#;
pub(crate) const DISLIKE: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linejoin="round"><path d="M3 3h4v11H3zM7 14l5 8h1c2 0 2-3 1-6h5c2 0 2-2 2-3l-2-8c0-1-1-2-2-2H7"/></svg>"#;

pub(crate) fn control(
    model: Rc<RefCell<AppModel>>,
    id: ItemId,
    revision: RwSignal<u64>,
    open: RwSignal<bool>,
    palette: Palette,
) -> AnyView {
    model.borrow_mut().rss_filters_open = Some(open);
    let trigger = toolbar_action_button(
        ToolbarAction::AiFilters,
        ToolbarSubject::Feed,
        palette,
        move || open.get(),
        move || open.update(|value| *value = !*value),
    );
    anchored_popover(trigger, open, 480.0, 8.0, false, move || {
        form(model.clone(), id.clone(), revision, open, palette)
    })
    .into_any()
}

fn multiline(value: RwSignal<String>, open: RwSignal<bool>, palette: Palette) -> AnyView {
    use floem::views::editor::command::CommandExecuted;
    use floem::views::editor::keypress::{default_key_handler, key::KeyInput};
    use floem::views::editor::text::WrapMethod;
    floem::views::text_editor::text_editor_keys(
        value.get_untracked(),
        move |editor, key, modifiers| {
            if matches!(
                &key.key,
                KeyInput::Keyboard(Key::Named(NamedKey::Escape), _)
            ) {
                open.set(false);
                CommandExecuted::Yes
            } else {
                default_key_handler(editor)(key, modifiers)
            }
        },
    )
    .update(move |event| {
        if let Some(editor) = event.editor {
            value.set(editor.rope_text().text.to_string());
        }
    })
    .editor_style(|style| style.hide_gutter(true).wrap_method(WrapMethod::EditorWidth))
    .style(move |style| {
        form_field_style(style, palette, value.get().len() > 16 * 1024)
            .width_full()
            .height(112.0)
            .font_size(14.0)
            .color(palette.ink)
            .background(palette.paper)
    })
    .into_any()
}

fn form(
    model: Rc<RefCell<AppModel>>,
    id: ItemId,
    revision: RwSignal<u64>,
    open: RwSignal<bool>,
    palette: Palette,
) -> AnyView {
    let preferences = model
        .borrow()
        .workspace
        .as_ref()
        .and_then(|w| w.rss_preferences(&id).ok())
        .unwrap_or_default();
    let expected = preferences.version;
    let likes = create_rw_signal(preferences.likes);
    let dislikes = create_rw_signal(preferences.dislikes);
    let alias = create_rw_signal(Some(preferences.alias));
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let settings = GlobalSettingsStore::load(home.as_deref()).settings.ai;
    let aliases = settings.aliases.keys().cloned().collect::<Vec<_>>();
    let dropdown = ai_settings::alias_dropdown(
        alias,
        aliases,
        |s| s.unwrap_or_else(|| "default".into()),
        move |s| alias.set(Some(s)),
        || true,
        palette,
    );
    let status_model = model.clone();
    let status_id = id.clone();
    let error = create_rw_signal(false);
    let pending = create_rw_signal(None::<u64>);
    let save_model = model.clone();
    let save_id = id.clone();
    create_effect(move |_| {
        revision.get();
        let completed = save_model.borrow().rss_saves.get(save_id.as_str()).copied();
        if let Some(token) = pending.get()
            && let Some((completed, success)) = completed
            && completed == token
        {
            pending.set(None);
            if success {
                open.set(false);
            } else {
                error.set(true);
            }
        }
    });
    let status = label(move || {
        revision.get();
        if likes.get().len() > 16 * 1024 || dislikes.get().len() > 16 * 1024 {
            return tr!(RssAiTooLong);
        }
        if error.get() {
            return tr!(RssAiConflict);
        }
        match status_model
            .borrow()
            .rss_status
            .get(status_id.as_str())
            .copied()
            .unwrap_or(rss_service::Status::Idle)
        {
            rss_service::Status::Idle | rss_service::Status::Saved => tr!(RssAiReady),
            rss_service::Status::Busy => tr!(RssAiBusy),
            rss_service::Status::Paused => tr!(RssAiPaused),
            rss_service::Status::Retry => tr!(RssAiRetry),
            rss_service::Status::Settings => tr!(RssAiSettings),
            rss_service::Status::Conflict => tr!(RssAiConflict),
        }
    })
    .style(move |s| s.font_size(12.0).color(palette.muted).width_full());
    let retry_model = model.clone();
    let retry_id = id.clone();
    v_stack((
        label(move || tr!(RssAiFilters)).style(move |s| s.font_size(16.0).color(palette.ink)),
        label(move || tr!(RssAiLikes)),
        multiline(likes, open, palette),
        label(move || tr!(RssAiDislikes)),
        multiline(dislikes, open, palette),
        label(move || tr!(AiModels)),
        dropdown,
        status,
        h_stack((
            text_button(
                msg!(RssAiRepeat),
                IconButtonTone::Secondary,
                palette,
                move || {
                    retry_model.borrow_mut().start_rss_refresh(retry_id.clone());
                },
            ),
            empty().style(|s| s.flex_grow(1.0)),
            text_button(
                msg!(Cancel),
                IconButtonTone::Secondary,
                palette,
                move || open.set(false),
            ),
            action_button(
                move || tr!(Save),
                IconButtonTone::Primary,
                palette,
                move || pending.get().is_none(),
                move || {
                    let value = RssPreferences {
                        likes: likes.get_untracked(),
                        dislikes: dislikes.get_untracked(),
                        alias: alias.get_untracked().unwrap_or_else(|| "default".into()),
                        ..RssPreferences::default()
                    };
                    if value.validate().is_err() {
                        error.set(true);
                        return;
                    }
                    let mut model = model.borrow_mut();
                    model.rss_save_sequence += 1;
                    let token = model.rss_save_sequence;
                    let accepted = model.rss_command(rss_service::Command::Preferences(
                        id.clone(),
                        token,
                        expected,
                        value,
                    ));
                    drop(model);
                    if accepted {
                        pending.set(Some(token));
                    } else {
                        error.set(true);
                    }
                },
            ),
        ))
        .style(|s| s.width_full().items_center().gap(8.0)),
    ))
    .style(move |s| {
        s.width(480.0)
            .padding(18.0)
            .gap(8.0)
            .background(palette.paper)
            .color(palette.ink)
            .border(1.0)
            .border_color(palette.divider)
            .border_radius(7.0)
    })
    .on_event(EventListener::KeyDown, move |event| {
        if matches!(event, Event::KeyDown(e) if e.key.logical_key == Key::Named(NamedKey::Escape)) {
            open.set(false);
        }
        EventPropagation::Stop
    })
    .into_any()
}
