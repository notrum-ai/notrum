// Copyright 2026 Evgeniy Udodov
// SPDX-License-Identifier: GPL-3.0-only

#![forbid(unsafe_code)]

use std::backtrace::Backtrace;
use std::io::Write;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

struct Report {
    summary: String,
    text: String,
}

impl Report {
    // Accept only source locations, never panic payloads or application state.
    fn capture(location: Option<&std::panic::Location<'_>>) -> Self {
        let location = location.map_or_else(
            || "unknown location".to_owned(),
            |location| location.to_string(),
        );
        let summary = format!("В Notrum произошла ошибка.\nМесто сбоя: {location}");
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |duration| duration.as_secs());
        let backtrace = Backtrace::force_capture();
        let text = format!(
            "\n=== Notrum panic: unix={timestamp} pid={} ===\n\
             Version: {}\nPlatform: {} / {}\nThread: {:?}\n\
             {summary}\nPanic payload omitted for privacy.\nBacktrace:\n{backtrace}\n",
            std::process::id(),
            env!("CARGO_PKG_VERSION"),
            std::env::consts::OS,
            std::env::consts::ARCH,
            std::thread::current().id(),
        );
        Self { summary, text }
    }
}

fn enter_once(active: &AtomicBool) -> bool {
    !active.swap(true, Ordering::SeqCst)
}

fn append_report(path: &Path, report: &str) -> std::io::Result<()> {
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    file.write_all(report.as_bytes())?;
    file.flush()?;
    file.sync_data()
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Choice {
    Copy,
    Close,
}

// The dialog and clipboard are injectable so tests never open native UI or exit.
fn present_report(
    report: &Report,
    mut show: impl FnMut(&str, bool) -> Choice,
    mut copy: impl FnMut(&str) -> Result<(), ()>,
) {
    let mut copy_failed = false;
    while show(&report.summary, copy_failed) == Choice::Copy {
        if copy(&report.text).is_ok() {
            return;
        }
        copy_failed = true;
    }
}

fn log_and_present(
    path: &Path,
    report: &Report,
    present: impl FnOnce(&Report),
) -> std::io::Result<()> {
    let logged = append_report(path, &report.text);
    present(report);
    logged
}

#[cfg(target_os = "macos")]
pub(super) fn install() {
    let error_log = std::env::current_dir()
        .unwrap_or_else(|_| std::path::PathBuf::from("."))
        .join("error.log");
    let active = AtomicBool::new(false);
    std::panic::set_hook(Box::new(move |info| {
        if !enter_once(&active) {
            std::process::exit(1);
        }
        let report = Report::capture(info.location());
        // Do not invoke the default hook: it prints arbitrary panic payloads.
        let _ = std::io::stderr().write_all(report.text.as_bytes());
        let _ = log_and_present(&error_log, &report, |report| {
            present_report(report, show_native, copy_native);
        });
        // Never resume the damaged event loop or run editor shutdown/autosave.
        std::process::exit(1);
    }));
}

#[cfg(target_os = "macos")]
fn show_native(summary: &str, copy_failed: bool) -> Choice {
    const COPY: &str = "Скопировать трейс и закрыть";
    let description = if copy_failed {
        format!(
            "{summary}\n\nНе удалось скопировать трейс. Попробуйте ещё раз или закройте приложение."
        )
    } else {
        format!("{summary}\n\nСкопируйте отчёт и передайте разработчику. Приложение будет закрыто.")
    };
    // No Floem window parent: RFD owns the modal dialog and main-thread dispatch.
    let result = rfd::MessageDialog::new()
        .set_title("Ошибка Notrum")
        .set_description(description)
        .set_level(rfd::MessageLevel::Error)
        .set_buttons(rfd::MessageButtons::OkCancelCustom(
            COPY.to_owned(),
            "Закрыть".to_owned(),
        ))
        .show();
    match result {
        rfd::MessageDialogResult::Custom(label) if label == COPY => Choice::Copy,
        _ => Choice::Close,
    }
}

#[cfg(target_os = "macos")]
fn copy_native(report: &str) -> Result<(), ()> {
    use copypasta::ClipboardProvider;

    copypasta::ClipboardContext::new()
        .and_then(|mut clipboard| clipboard.set_contents(report.to_owned()))
        .map_err(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report() -> Report {
        Report {
            summary: "brief error".to_owned(),
            text: "complete report\nframe 1\nframe 2\n".to_owned(),
        }
    }

    #[test]
    fn capture_contains_metadata_and_forced_backtrace() {
        let report = Report::capture(Some(std::panic::Location::caller()));
        assert!(report.summary.contains("crash_dialog.rs:"));
        assert!(report.text.contains(env!("CARGO_PKG_VERSION")));
        assert!(report.text.contains(std::env::consts::OS));
        assert!(report.text.contains(std::env::consts::ARCH));
        assert!(report.text.contains("unix="));
        assert!(report.text.contains("Thread:"));
        assert!(report.text.contains("Backtrace:\n"));
        assert!(!report.text.contains("disabled backtrace"));
        assert!(report.text.contains("crash_dialog::"));
        assert!(Report::capture(None).summary.contains("unknown location"));
    }

    #[test]
    fn capture_does_not_read_thread_name() {
        let secret = "protected body and password must stay private";
        // Thread names, like panic payloads, can contain application data.
        let captured = std::thread::Builder::new()
            .name(secret.to_owned())
            .spawn(|| Report::capture(Some(std::panic::Location::caller())))
            .unwrap()
            .join()
            .unwrap();
        assert!(!captured.summary.contains(secret));
        assert!(!captured.text.contains(secret));
        assert!(captured.text.contains("Panic payload omitted for privacy."));
    }

    #[test]
    fn copy_receives_complete_report_once() {
        let report = report();
        let mut copied = Vec::new();
        let mut shown = 0;
        present_report(
            &report,
            |summary, failed| {
                assert_eq!(summary, report.summary);
                assert!(!failed);
                shown += 1;
                Choice::Copy
            },
            |text| {
                copied.push(text.to_owned());
                Ok(())
            },
        );
        assert_eq!(shown, 1);
        assert_eq!(copied, [report.text]);
    }

    #[test]
    fn closing_does_not_touch_clipboard() {
        present_report(
            &report(),
            |_, _| Choice::Close,
            |_| panic!("closing must not copy"),
        );
    }

    #[test]
    fn failed_copy_can_be_retried_or_closed() {
        for retry in [false, true] {
            let mut shown = Vec::new();
            let mut copies = 0;
            present_report(
                &report(),
                |_, failed| {
                    shown.push(failed);
                    if !failed || retry {
                        Choice::Copy
                    } else {
                        Choice::Close
                    }
                },
                |_| {
                    copies += 1;
                    if copies == 1 { Err(()) } else { Ok(()) }
                },
            );
            assert_eq!(shown, [false, true]);
            assert_eq!(copies, if retry { 2 } else { 1 });
        }
    }

    #[test]
    fn concurrent_panics_admit_only_one_dialog() {
        let active = AtomicBool::new(false);
        std::thread::scope(|scope| {
            let attempts = (0..8)
                .map(|_| scope.spawn(|| enter_once(&active)))
                .collect::<Vec<_>>();
            let entered = attempts
                .into_iter()
                .map(|attempt| usize::from(attempt.join().unwrap()))
                .sum::<usize>();
            assert_eq!(entered, 1);
        });
        assert!(!enter_once(&active));
    }

    #[test]
    fn log_failure_still_presents_the_report() {
        let mut presented = false;
        // Opening a directory as an append-only file must fail.
        let result = log_and_present(&std::env::temp_dir(), &report(), |_| {
            presented = true;
        });
        assert!(result.is_err());
        assert!(presented);
    }
}
