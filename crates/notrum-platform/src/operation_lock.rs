// Copyright 2026 Evgeniy Udodov
// SPDX-License-Identifier: GPL-3.0-only

#![forbid(unsafe_code)]

use std::cell::RefCell;
use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io;
use std::marker::PhantomData;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use crate::diagnostics::{Operation, Stage, io_result};

thread_local! {
    static HELD: RefCell<HashMap<PathBuf, (File, usize)>> = RefCell::new(HashMap::new());
}

/// A short, reentrant transaction lock. The empty marker is never removed:
/// unlinking it would allow another process to lock a different inode.
pub struct OperationLock {
    directory: PathBuf,
    thread: PhantomData<Rc<()>>,
}

impl OperationLock {
    pub fn directory(directory: &Path) -> io::Result<Self> {
        let directory = directory.canonicalize()?;
        super::validate_real_path(&directory)?;
        HELD.with(|held| {
            let mut held = held.borrow_mut();
            if let Some((_, depth)) = held.get_mut(&directory) {
                *depth += 1;
                return Ok(());
            }
            let path = directory.join(".notrum-operation.lock");
            // Publish an empty restricted marker before opening a shared handle.
            match super::create_private_file(&path) {
                Ok(file) => drop(file),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                Err(error) => return io_result(Operation::Lock, Stage::Create, Err(error)),
            }
            io_result(
                Operation::Lock,
                Stage::Validate,
                super::validate_private(&path),
            )?;
            let mut options = OpenOptions::new();
            options.read(true).write(true);
            #[cfg(windows)]
            {
                use std::os::windows::fs::OpenOptionsExt;
                options.share_mode(3); // Readers/writers, never deletion while held.
            }
            let file = io_result(Operation::Lock, Stage::Open, options.open(&path))?;
            if file.metadata()?.len() != 0 || super::file_information(&file)?.links != 1 {
                return Err(io::Error::other("invalid operation lock marker"));
            }
            io_result(
                Operation::Lock,
                Stage::Acquire,
                fs4::fs_std::FileExt::lock_exclusive(&file),
            )?;
            held.insert(directory.clone(), (file, 1));
            Ok::<_, io::Error>(())
        })?;
        Ok(Self {
            directory,
            thread: PhantomData,
        })
    }

    /// Workspace notes share their root lock with recovery and password changes;
    /// unrelated external files use their containing directory.
    pub fn file(path: &Path) -> io::Result<Self> {
        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let directory = if parent.file_name().is_some_and(|name| name == "notes") {
            parent.parent().unwrap_or(parent)
        } else {
            parent
        };
        Self::directory(directory)
    }
}

impl Drop for OperationLock {
    fn drop(&mut self) {
        HELD.with(|held| {
            let mut held = held.borrow_mut();
            if let Some((_, depth)) = held.get_mut(&self.directory) {
                *depth -= 1;
                if *depth == 0 {
                    held.remove(&self.directory);
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::Duration;

    #[test]
    fn simultaneous_first_users_serialize_on_one_marker() {
        let root =
            std::env::temp_dir().join(format!("notrum-operation-first-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let start = std::sync::Arc::new(std::sync::Barrier::new(2));
        let entered = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let workers = (0..2)
            .map(|_| {
                let root = root.clone();
                let start = start.clone();
                let entered = entered.clone();
                std::thread::spawn(move || -> io::Result<_> {
                    start.wait();
                    let _lock = OperationLock::directory(&root)?;
                    assert_eq!(entered.fetch_add(1, std::sync::atomic::Ordering::SeqCst), 0);
                    let identity = super::super::file_information(&File::open(
                        root.join(".notrum-operation.lock"),
                    )?)?
                    .identity;
                    std::thread::sleep(Duration::from_millis(20));
                    assert_eq!(entered.fetch_sub(1, std::sync::atomic::Ordering::SeqCst), 1);
                    Ok(identity)
                })
            })
            .collect::<Vec<_>>();
        let results = workers
            .into_iter()
            .map(|worker| worker.join().unwrap())
            .collect::<Vec<_>>();
        eprintln!(
            "NATIVE_ASSERT operation=ConcurrentLock success={}",
            results.iter().all(Result::is_ok)
        );
        assert_eq!(results[0].as_ref().unwrap(), results[1].as_ref().unwrap());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn serializes_independent_handles_and_allows_nested_operations() {
        let root =
            std::env::temp_dir().join(format!("notrum-operation-test-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let outer = OperationLock::directory(&root).unwrap();
        drop(OperationLock::directory(&root).unwrap());
        let (send, receive) = mpsc::channel();
        let other = root.clone();
        let worker = std::thread::spawn(move || {
            let _lock = OperationLock::directory(&other).unwrap();
            send.send(()).unwrap();
        });
        assert!(receive.recv_timeout(Duration::from_millis(50)).is_err());
        drop(outer);
        receive.recv_timeout(Duration::from_secs(5)).unwrap();
        worker.join().unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }
}
