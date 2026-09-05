// Copyright 2026 Evgeniy Udodov
// SPDX-License-Identifier: GPL-3.0-only

#![forbid(unsafe_code)]

use std::fs::{self, File, OpenOptions};
use std::io;
use std::os::windows::fs::OpenOptionsExt;
use std::os::windows::io::AsRawHandle;
use std::path::Path;

use windows_acl::acl::{ACL, AceType};
use windows_acl::helper::{current_user, name_to_sid, sid_to_string, string_to_sid};

const READ_CONTROL: u32 = 0x0002_0000;
const WRITE_DAC: u32 = 0x0004_0000;
const GENERIC_READ: u32 = 0x8000_0000;
const GENERIC_WRITE: u32 = 0x4000_0000;
const FILE_ALL_ACCESS: u32 = 0x001f_01ff;
const BACKUP_SEMANTICS: u32 = 0x0200_0000;
const OPEN_REPARSE_POINT: u32 = 0x0020_0000;
const ADMINISTRATORS: &str = "S-1-5-32-544";
const SYSTEM: &str = "S-1-5-18";

fn error(code: u32) -> io::Error {
    io::Error::from_raw_os_error(code as i32)
}
fn acl(file: &File) -> io::Result<ACL> {
    ACL::from_file_handle(file.as_raw_handle().cast(), false).map_err(error)
}
fn user_sid() -> io::Result<Vec<u8>> {
    let name =
        current_user().ok_or_else(|| io::Error::other("cannot resolve current Windows user"))?;
    name_to_sid(&name, None).map_err(error)
}
fn ensure_applied(applied: bool) -> io::Result<()> {
    if applied {
        Ok(())
    } else {
        Err(io::Error::other("Windows did not apply the requested ACL"))
    }
}
fn restrict(file: &File, directory: bool) -> io::Result<()> {
    let mut access = acl(file)?;
    let entries = access.all().map_err(error)?;
    // The exclusively opened handle retains WRITE_DAC while the DACL is rebuilt.
    // windows-acl applies a protected DACL, disabling parent inheritance.
    for entry in entries {
        let sid = string_to_sid(&entry.string_sid).map_err(error)?;
        access
            .remove_entry(sid.as_ptr().cast_mut().cast(), None, None)
            .map_err(error)?;
    }
    let owner = user_sid()?;
    for sid in [
        owner,
        string_to_sid(ADMINISTRATORS).map_err(error)?,
        string_to_sid(SYSTEM).map_err(error)?,
    ] {
        ensure_applied(
            access
                .allow(sid.as_ptr().cast_mut().cast(), directory, FILE_ALL_ACCESS)
                .map_err(error)?,
        )?;
    }
    validate_handle(file)
}
fn validate_acl(file: &File) -> io::Result<()> {
    let owner = user_sid()?;
    let owner = sid_to_string(owner.as_ptr().cast_mut().cast()).map_err(error)?;
    let entries = acl(file)?.all().map_err(error)?;
    let mut has_owner = false;
    let mut has_administrators = false;
    for entry in entries {
        if entry.entry_type != AceType::AccessAllow
            || ![owner.as_str(), ADMINISTRATORS, SYSTEM].contains(&entry.string_sid.as_str())
        {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "private path has an unexpected access rule",
            ));
        }
        if entry.mask & FILE_ALL_ACCESS == FILE_ALL_ACCESS && entry.flags & 0x08 == 0 {
            has_owner |= entry.string_sid == owner;
            has_administrators |= entry.string_sid == ADMINISTRATORS;
        }
    }
    if !has_owner || !has_administrators {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "private path is missing required access rules",
        ));
    }
    Ok(())
}
fn validate_handle(file: &File) -> io::Result<()> {
    validate_acl(file)?;
    let metadata = file.metadata()?;
    if super::is_link(&metadata)
        || (metadata.is_file() && super::file_information(file)?.links != 1)
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "private path is linked",
        ));
    }
    Ok(())
}

pub(super) fn create_private_file(path: &Path) -> io::Result<File> {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .access_mode(GENERIC_READ | GENERIC_WRITE | READ_CONTROL | WRITE_DAC)
        .share_mode(0)
        .custom_flags(OPEN_REPARSE_POINT)
        .open(path)?;
    // No other handle can open this empty file before access is restricted.
    if let Err(error) = restrict(&file, false) {
        drop(file);
        // Preserve the ACL failure, even if cleaning the empty file also fails.
        let _ = fs::remove_file(path);
        return Err(error);
    }
    Ok(file)
}

pub(super) fn create_private_directory(path: &Path) -> io::Result<()> {
    fs::create_dir(path)?;
    let file = OpenOptions::new()
        .access_mode(READ_CONTROL | WRITE_DAC)
        .share_mode(0)
        .custom_flags(BACKUP_SEMANTICS | OPEN_REPARSE_POINT)
        .open(path)?;
    restrict(&file, true)
}

pub(super) fn validate_private(path: &Path) -> io::Result<()> {
    let file = OpenOptions::new()
        .access_mode(READ_CONTROL)
        .custom_flags(BACKUP_SEMANTICS | OPEN_REPARSE_POINT)
        .open(path)?;
    validate_handle(&file)
}

pub(super) fn preserve_permissions(source: &File, destination: &File) -> io::Result<()> {
    apply_permissions(destination, &capture_permissions(source)?)
}

pub(super) fn capture_permissions(file: &File) -> io::Result<super::fs::Permissions> {
    let entries = acl(file)?.all().map_err(error)?;
    let mut rules = Vec::with_capacity(entries.len());
    for entry in entries {
        let allow = match entry.entry_type {
            AceType::AccessAllow => true,
            AceType::AccessDeny => false,
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::Unsupported,
                    "cannot preserve this Windows ACL entry type",
                ));
            }
        };
        rules.push(super::fs::AccessRule {
            sid: entry.string_sid,
            allow,
            flags: entry.flags,
            mask: entry.mask,
        });
    }
    Ok(super::fs::Permissions {
        readonly: file.metadata()?.permissions().readonly(),
        rules,
        private: validate_acl(file).is_ok(),
    })
}

pub(super) fn apply_permissions(
    file: &File,
    permissions: &super::fs::Permissions,
) -> io::Result<()> {
    // Refuse a write to a read-only source before making its temporary read-only.
    // Otherwise a failed replacement could prevent cleanup of the temporary.
    if permissions.readonly {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "source is read-only",
        ));
    }
    let mut access = acl(file)?;
    for entry in access.all().map_err(error)? {
        let sid = string_to_sid(&entry.string_sid).map_err(error)?;
        access
            .remove_entry(sid.as_ptr().cast_mut().cast(), None, None)
            .map_err(error)?;
    }
    for rule in &permissions.rules {
        let sid = string_to_sid(&rule.sid).map_err(error)?;
        ensure_applied(
            access
                .add_entry(
                    sid.as_ptr().cast_mut().cast(),
                    if rule.allow {
                        AceType::AccessAllow
                    } else {
                        AceType::AccessDeny
                    },
                    rule.flags,
                    rule.mask,
                )
                .map_err(error)?,
        )?;
    }
    let mut flags = file.metadata()?.permissions();
    flags.set_readonly(permissions.readonly);
    file.set_permissions(flags)?;
    if capture_permissions(file)? != *permissions {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "Windows did not preserve the complete ACL",
        ));
    }
    Ok(())
}

/// NTFS namespace barrier: flush an empty file and move it with WRITE_THROUGH.
/// A crash may leave an empty marker; it never contains note data.
pub(super) fn sync_directory(path: &Path) -> io::Result<()> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    super::validate_real_path(path)?;
    for _ in 0..32 {
        let id = NEXT.fetch_add(1, Ordering::Relaxed);
        let source = path.join(format!(".notrum-sync-{}-{id}.tmp", std::process::id()));
        let destination = source.with_extension("done");
        let file = match create_private_file(&source) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        };
        let result = (|| {
            file.sync_all()?;
            drop(file);
            atomicwrites::move_atomic(&source, &destination)?;
            fs::remove_file(&destination)
        })();
        if result.is_err() {
            let _ = fs::remove_file(&source);
        }
        if result
            .as_ref()
            .is_err_and(|error| error.kind() == io::ErrorKind::AlreadyExists)
        {
            continue;
        }
        return result;
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "cannot allocate namespace barrier",
    ))
}
