// Copyright 2026 Evgeniy Udodov
// SPDX-License-Identifier: GPL-3.0-only

#![forbid(unsafe_code)]

//! The update settings page, the startup check and the prompt that offers a
//! published release.
//!
//! Checking and installing run on a worker thread; the user interface only
//! reads the stage the worker reports. An automatic check holds a release back
//! for a day after publication, a manual check never does, and nothing is
//! installed without the user pressing the button.

use crate::ai_settings::{actions, page_description, page_title, spacer};
use crate::*;
use notrum_update::{
    CheckMode, Decision, Installation, Release, UpdateError, UpdateTransport, Version,
};
use std::sync::mpsc::{self, Receiver};
use std::thread;

/// Long enough for the workspace to finish opening before the check starts.
const STARTUP_DELAY_MS: u64 = 1500;
const POLL_MS: u64 = 60;
/// Distance between the prompt card and the window corner.
const PROMPT_INSET_PX: f64 = 18.0;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Stage {
    /// Nothing has been checked yet in this session.
    Idle,
    Checking,
    UpToDate,
    Available(Box<Release>),
    /// Newer, but published too recently for an automatic check.
    Held(Box<Release>),
    /// Newer, without a package this platform can install.
    Unpackaged(Box<Release>),
    Downloading {
        release: Box<Release>,
        received: u64,
        total: Option<u64>,
    },
    Installed(Version),
    Failed(UpdateError),
    /// This build cannot replace itself, so only the message is shown.
    Unsupported(UpdateError),
}

impl Stage {
    fn release(&self) -> Option<&Release> {
        match self {
            Self::Available(release)
            | Self::Held(release)
            | Self::Unpackaged(release)
            | Self::Downloading { release, .. } => Some(release),
            _ => None,
        }
    }

    fn busy(&self) -> bool {
        matches!(self, Self::Checking | Self::Downloading { .. })
    }
}

enum Message {
    Progress(u64, Option<u64>),
    Checked(Result<Decision, UpdateError>),
    Installed(Result<Version, UpdateError>),
}

#[derive(Clone)]
pub(crate) struct Updates {
    stage: RwSignal<Stage>,
    /// The prompt that appears over the workspace after a startup check.
    prompt: RwSignal<bool>,
    automatic: RwSignal<bool>,
    generation: Rc<Cell<u64>>,
    installation: Rc<Option<Installation>>,
    global: Rc<RefCell<GlobalSettingsStore>>,
}

impl Updates {
    pub(crate) fn new(global: Rc<RefCell<GlobalSettingsStore>>) -> Self {
        let installation = installation();
        if let Some(installation) = installation.as_ref() {
            // A previous update may have left the replaced files behind.
            installation.cleanup();
        }
        let settings = global.borrow().updates();
        let stage = match installation.as_ref() {
            Some(_) => Stage::Idle,
            None => Stage::Unsupported(UpdateError::NotInstalled),
        };
        Self {
            stage: create_rw_signal(stage),
            prompt: create_rw_signal(false),
            automatic: create_rw_signal(settings.automatic),
            generation: Rc::new(Cell::new(0)),
            installation: Rc::new(installation),
            global,
        }
    }

    /// Runs the background check that every start performs.
    pub(crate) fn start(&self) {
        if self.installation.is_none() || !self.automatic.get_untracked() {
            return;
        }
        let controller = self.clone();
        exec_after(Duration::from_millis(STARTUP_DELAY_MS), move |_| {
            if controller.stage.try_get_untracked().is_none() {
                return;
            }
            controller.check(CheckMode::Automatic);
        });
    }

    fn version(&self) -> Version {
        Version::parse(env!("CARGO_PKG_VERSION")).unwrap_or(Version::new(0, 0, 0))
    }

    fn check(&self, mode: CheckMode) {
        if self.installation.is_none() || self.stage.get_untracked().busy() {
            return;
        }
        self.stage.set(Stage::Checking);
        let current = self.version();
        let (sender, receiver) = mpsc::channel();
        thread::spawn(move || {
            let transport = transport();
            let result =
                notrum_update::check(transport.as_ref(), current, mode, notrum_update::now_ms());
            let _ = sender.send(Message::Checked(result));
        });
        self.poll(receiver, mode);
    }

    fn install(&self) {
        let Some(installation) = (*self.installation).clone() else {
            return;
        };
        let stage = self.stage.get_untracked();
        let Some(release) = stage.release().cloned() else {
            return;
        };
        if stage.busy() {
            return;
        }
        self.stage.set(Stage::Downloading {
            release: Box::new(release.clone()),
            received: 0,
            total: None,
        });
        let (sender, receiver) = mpsc::channel();
        let progress = sender.clone();
        thread::spawn(move || {
            let transport = transport();
            let version = release.version;
            let result = notrum_update::install(
                transport.as_ref(),
                &installation,
                &release,
                &mut |received, total| {
                    let _ = progress.send(Message::Progress(received, total));
                },
            );
            let _ = sender.send(Message::Installed(result.map(|()| version)));
        });
        self.poll(receiver, CheckMode::Manual);
    }

    /// Reads worker messages on the user interface thread. The generation
    /// counter drops results of a run the user has already replaced.
    fn poll(&self, receiver: Receiver<Message>, mode: CheckMode) {
        let generation = self.generation.get() + 1;
        self.generation.set(generation);
        let controller = self.clone();
        let receiver = Rc::new(receiver);
        schedule(controller, receiver, generation, mode);
    }

    fn apply(&self, message: Message, mode: CheckMode) -> bool {
        match message {
            Message::Progress(received, total) => {
                if let Stage::Downloading { release, .. } = self.stage.get_untracked() {
                    self.stage.set(Stage::Downloading {
                        release,
                        received,
                        total,
                    });
                }
                false
            }
            Message::Checked(Ok(decision)) => {
                self.stage.set(match decision {
                    Decision::UpToDate => Stage::UpToDate,
                    Decision::Available(release) => {
                        if mode == CheckMode::Automatic && !self.dismissed(release.version) {
                            self.prompt.set(true);
                        }
                        Stage::Available(Box::new(release))
                    }
                    Decision::Held { release, .. } => Stage::Held(Box::new(release)),
                    Decision::Unpackaged(release) => Stage::Unpackaged(Box::new(release)),
                });
                true
            }
            Message::Checked(Err(error)) | Message::Installed(Err(error)) => {
                // A failed startup check leaves the workspace alone; a failed
                // installation keeps the prompt open to report it.
                self.stage.set(match error {
                    UpdateError::NotInstalled | UpdateError::ReadOnly => Stage::Unsupported(error),
                    error => Stage::Failed(error),
                });
                true
            }
            Message::Installed(Ok(version)) => {
                self.stage.set(Stage::Installed(version));
                true
            }
        }
    }

    fn dismissed(&self, version: Version) -> bool {
        self.global
            .borrow()
            .updates()
            .dismissed
            .is_some_and(|dismissed| dismissed == version.to_string())
    }

    /// Remembers that this version was declined, so the prompt does not
    /// reappear at every start until a newer release is published.
    fn dismiss(&self) {
        self.prompt.set(false);
        let Some(release) = self.stage.get_untracked().release().cloned() else {
            return;
        };
        let mut settings = self.global.borrow().updates();
        settings.dismissed = Some(release.version.to_string());
        self.store(settings);
    }

    fn set_automatic(&self, automatic: bool) {
        self.automatic.set(automatic);
        let mut settings = self.global.borrow().updates();
        settings.automatic = automatic;
        self.store(settings);
    }

    fn store(&self, settings: UpdateSettings) {
        if let Err(error) = self.global.borrow_mut().set_updates(settings) {
            self.stage
                .set(Stage::Failed(UpdateError::Io(error.to_string())));
        }
    }

    fn open_page(&self) {
        let Some(release) = self.stage.get_untracked().release().cloned() else {
            return;
        };
        if let Err(error) = open_rss_original(&release.page_url) {
            self.stage
                .set(Stage::Failed(UpdateError::Io(error.to_string())));
        }
    }
}

fn schedule(
    controller: Updates,
    receiver: Rc<Receiver<Message>>,
    generation: u64,
    mode: CheckMode,
) {
    exec_after(Duration::from_millis(POLL_MS), move |_| {
        if controller.stage.try_get_untracked().is_none() {
            return;
        }
        if controller.generation.get() != generation {
            return;
        }
        loop {
            match receiver.try_recv() {
                Ok(message) => {
                    if controller.apply(message, mode) {
                        return;
                    }
                }
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    if controller.stage.get_untracked().busy() {
                        controller.stage.set(Stage::Failed(UpdateError::Network));
                    }
                    return;
                }
            }
        }
        schedule(controller, receiver, generation, mode);
    });
}

fn transport() -> Box<dyn UpdateTransport> {
    #[cfg(feature = "test-utils")]
    if let Ok(directory) = std::env::var("NOTRUM_TEST_UPDATE") {
        return Box::new(fixtures::Directory::new(PathBuf::from(directory)));
    }
    Box::new(notrum_update::HttpsTransport)
}

fn installation() -> Option<Installation> {
    #[cfg(feature = "test-utils")]
    if let Ok(root) = std::env::var("NOTRUM_TEST_UPDATE_ROOT") {
        return fixtures::installation(&PathBuf::from(root));
    }
    Installation::locate().ok()
}

/// The prompt that offers a release found by the startup check. It never
/// blocks the workspace: the window stays usable behind it.
pub(crate) fn prompt_view(updates: Updates, palette: Palette) -> impl IntoView {
    let stage = updates.stage;
    let prompt = updates.prompt;
    let install = updates.clone();
    let later = updates.clone();
    let title = label(move || match stage.get() {
        Stage::Installed(version) => {
            UiText::from(msg!(UpdateInstalledRestart, "version" => version.to_string()))
        }
        stage => stage.release().map_or_else(UiText::default, |release| {
            UiText::from(msg!(UpdateAvailable, "version" => release.version.to_string()))
        }),
    })
    .style(move |style| style.font_size(13.5).color(palette.ink).selectable(false));
    let status = label(move || status_text(&stage.get()))
        .style(move |style| style.font_size(12.5).color(palette.muted).selectable(false))
        .style(move |style| {
            style.apply_if(matches!(stage.get(), Stage::Available(_)), |style| {
                style.hide()
            })
        });
    let update_button = action_button(
        move || tr!(UpdateInstall),
        IconButtonTone::Primary,
        palette,
        move || matches!(stage.get(), Stage::Available(_)),
        move || install.install(),
    )
    .style(move |style| {
        style.apply_if(
            !matches!(stage.get(), Stage::Available(_) | Stage::Downloading { .. }),
            |style| style.hide(),
        )
    });
    let dismiss = action_button(
        move || tr!(UpdateLater),
        IconButtonTone::Secondary,
        palette,
        || true,
        move || later.dismiss(),
    );
    let card = v_stack((
        title,
        status,
        h_stack((
            empty().style(|style| style.flex_grow(1.0)),
            dismiss,
            update_button,
        ))
        .style(|style| rtl_row(style).width_full().items_center().gap(8.0)),
    ))
    .style(move |style| {
        rtl_column(style)
            .width(330.0)
            .gap(10.0)
            .padding(16.0)
            .background(palette.paper)
            .color(palette.ink)
            .border(1.0)
            .border_color(palette.divider)
            .border_radius(9.0)
    });
    // The card is placed in the corner of the window instead of inside a
    // full-window container, so that only the card itself sits over the
    // workspace and the rest of the window keeps receiving pointer events.
    card.style(move |style| {
        let style = style
            .absolute()
            .inset_bottom(PROMPT_INSET_PX)
            .apply_if(i18n::current().is_rtl(), |style| {
                style.inset_left(PROMPT_INSET_PX)
            })
            .apply_if(!i18n::current().is_rtl(), |style| {
                style.inset_right(PROMPT_INSET_PX)
            });
        if prompt.get() { style } else { style.hide() }
    })
}

/// The settings page: the manual check, the result and the startup switch.
pub(crate) fn page(
    signals: SettingsPageSignals,
    updates: Updates,
    palette: Palette,
) -> impl IntoView {
    let stage = updates.stage;
    let automatic = updates.automatic;
    // Leaving the page cancels nothing, but a finished result should not be
    // presented as fresh the next time the page opens.
    let lifecycle = updates.clone();
    let was_visible = Rc::new(Cell::new(false));
    create_effect(move |_| {
        let visible = signals.open.get() && signals.section.get() == SettingsSection::Updates;
        if was_visible.replace(visible) == visible || visible {
            return;
        }
        if matches!(lifecycle.stage.get_untracked(), Stage::Failed(_)) {
            lifecycle.stage.set(Stage::Idle);
        }
    });

    let check = updates.clone();
    let install = updates.clone();
    let page_open = updates.clone();
    let toggle = updates.clone();
    let status_card = v_stack((
        label(|| msg!(UpdateInstalledVersion, "version" => env!("CARGO_PKG_VERSION")))
            .style(move |style| style.font_size(15.0).color(palette.ink).selectable(false)),
        label(move || status_text(&stage.get()))
            .style(move |style| style.font_size(12.5).line_height(1.4).color(palette.muted)),
        actions((
            action_button(
                move || tr!(UpdateCheckNow),
                IconButtonTone::Secondary,
                palette,
                move || !stage.get().busy() && !matches!(stage.get(), Stage::Unsupported(_)),
                move || check.check(CheckMode::Manual),
            ),
            action_button(
                move || tr!(UpdateInstall),
                IconButtonTone::Primary,
                palette,
                move || matches!(stage.get(), Stage::Available(_) | Stage::Held(_)),
                move || install.install(),
            )
            .style(move |style| {
                style.apply_if(
                    !matches!(
                        stage.get(),
                        Stage::Available(_) | Stage::Held(_) | Stage::Downloading { .. }
                    ),
                    |style| style.hide(),
                )
            }),
            action_button(
                move || tr!(UpdateOpenPage),
                IconButtonTone::Secondary,
                palette,
                || true,
                move || page_open.open_page(),
            )
            .style(move |style| {
                style.apply_if(stage.get().release().is_none(), |style| style.hide())
            }),
        ))
        .style(|style| style.margin_top(4.0)),
    ))
    .style(move |style| settings_card_style(style, palette).gap(10.0));

    let notes_card = v_stack((
        settings_field_label(i18n::Key::UpdateNotes, palette),
        label(move || {
            stage
                .get()
                .release()
                .map(|release| release.notes.clone())
                .unwrap_or_default()
        })
        .style(move |style| style.font_size(12.5).line_height(1.4).color(palette.muted)),
    ))
    .style(move |style| {
        settings_card_style(style, palette).gap(8.0).apply_if(
            stage
                .get()
                .release()
                .is_none_or(|release| release.notes.is_empty()),
            |style| style.hide(),
        )
    });

    let automatic_card = v_stack((
        label(move || tr!(UpdateAutomatic))
            .style(move |style| style.font_size(15.0).color(palette.ink).selectable(false)),
        settings_hint(i18n::Key::UpdateAutomaticHint, palette),
        label(move || {
            if automatic.get() {
                tr!(UpdateAutomaticEnabled)
            } else {
                tr!(UpdateAutomaticDisabled)
            }
        })
        .style(move |style| style.font_size(12.5).color(palette.muted).selectable(false)),
        actions((action_button(
            move || {
                if automatic.get() {
                    tr!(UpdateAutomaticDisable)
                } else {
                    tr!(UpdateAutomaticEnable)
                }
            },
            IconButtonTone::Secondary,
            palette,
            || true,
            move || toggle.set_automatic(!automatic.get_untracked()),
        ),))
        .style(|style| style.margin_top(4.0)),
    ))
    .style(move |style| settings_card_style(style, palette).gap(10.0));

    scroll(
        v_stack((
            page_title(i18n::Key::Updates, palette),
            spacer(7.0),
            page_description(i18n::Key::UpdatesDescription, palette),
            spacer(28.0),
            status_card,
            spacer(20.0),
            notes_card,
            spacer(20.0),
            automatic_card,
        ))
        .style(|style| {
            rtl_column(style)
                .width_full()
                .padding_horiz(44.0)
                .padding_vert(38.0)
        }),
    )
    .style(move |style| {
        style
            .min_width(0.0)
            .height_full()
            .flex_grow(1.0)
            .background(palette.canvas)
    })
}

fn status_text(stage: &Stage) -> UiText {
    let message = match stage {
        Stage::Idle => return UiText::default(),
        Stage::Checking => msg!(UpdateChecking),
        Stage::UpToDate => msg!(UpdateUpToDate),
        Stage::Available(release) => {
            msg!(UpdateAvailable, "version" => release.version.to_string())
        }
        Stage::Held(release) => msg!(UpdateHeld, "version" => release.version.to_string()),
        Stage::Unpackaged(release) => {
            msg!(UpdateUnpackaged, "version" => release.version.to_string())
        }
        Stage::Downloading {
            received, total, ..
        } => match total {
            Some(total) if *total > 0 => {
                msg!(UpdateDownloadingPercent, "percent" => received * 100 / total)
            }
            _ => msg!(UpdateDownloading),
        },
        Stage::Installed(version) => {
            msg!(UpdateInstalledRestart, "version" => version.to_string())
        }
        Stage::Failed(error) | Stage::Unsupported(error) => failure(error),
    };
    message.into()
}

fn failure(error: &UpdateError) -> i18n::Message {
    match error {
        UpdateError::NotInstalled => msg!(UpdateNotInstalled),
        UpdateError::ReadOnly => msg!(UpdateReadOnly),
        UpdateError::Network => msg!(UpdateNetworkFailed),
        UpdateError::RateLimited => msg!(UpdateRateLimited),
        UpdateError::Response => msg!(UpdateResponseFailed),
        UpdateError::NoPackage => msg!(UpdateNoPackage),
        UpdateError::Checksum => msg!(UpdateChecksumFailed),
        UpdateError::Package(detail) => msg!(UpdateFailed, "error" => (*detail).to_owned()),
        UpdateError::Io(detail) => msg!(UpdateFailed, "error" => detail.clone()),
    }
}

/// Fixtures let the acceptance run drive a complete update without a network
/// and without touching the installation the test runs from.
#[cfg(feature = "test-utils")]
pub(crate) mod fixtures {
    use super::*;

    pub(crate) struct Directory(PathBuf);

    impl Directory {
        pub(crate) fn new(path: PathBuf) -> Self {
            Self(path)
        }
    }

    impl UpdateTransport for Directory {
        fn fetch(
            &self,
            url: &str,
            _accept: &str,
            limit: u64,
            progress: &mut dyn FnMut(u64, Option<u64>),
        ) -> Result<Vec<u8>, UpdateError> {
            let name = if url.starts_with("https://api.github.com/") {
                "latest.json"
            } else {
                url.rsplit('/').next().unwrap_or_default()
            };
            if name.is_empty() || name.contains("..") {
                return Err(UpdateError::Response);
            }
            let bytes = std::fs::read(self.0.join(name)).map_err(|_| UpdateError::Network)?;
            if bytes.len() as u64 > limit {
                return Err(UpdateError::Response);
            }
            progress(bytes.len() as u64, Some(bytes.len() as u64));
            Ok(bytes)
        }
    }

    pub(crate) fn installation(root: &Path) -> Option<Installation> {
        if cfg!(target_os = "macos") {
            Installation::mac_app(root.join("Notrum.app")).ok()
        } else if cfg!(windows) {
            Installation::windows(root.join("Notrum.exe")).ok()
        } else {
            Installation::linux(root.join("notrum")).ok()
        }
    }
}
