<!-- Copyright 2026 Evgeniy Udodov -->
<!-- SPDX-License-Identifier: GPL-3.0-only -->
# Windows builds and acceptance

Supported target: Windows 10/11 x64, local NTFS, portable application.
An installer, code signing, file association registration, ARM64, network shares,
and other filesystems are outside this target. The dependency audit includes
`x86_64-pc-windows-gnu`; the compiler remains Rust 1.88.0.

## Build and automated tests

On macOS or Linux with Docker running:

```sh
make build-windows
make test-windows-build
```

Copy the complete `dist/windows/x86_64` directory to Windows, retaining any DLLs
beside their executable. In Windows PowerShell 5.1 or later:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\tests\Run-Tests.ps1
```

No Rust installation is required. The runner requires a local NTFS temporary
directory, creates isolated Unicode paths with spaces, redirects global settings
to that test profile, creates an NTFS junction fixture, runs every compiled test
executable, and starts the native application with a timed exit. Tests and logs
are retained under the printed temporary directory for diagnosis. Existing
workspaces and the user's real `.notrum.cfg` are not used.

`tests/windows-results.json` records the Windows build, filesystem, individual
test exit codes, and native launch result. A cross-compilation log and this
Windows result are separate evidence. Neither a successful `make check` on
Linux nor compilation of the test EXEs means that Windows execution passed.
There is no committed passing native Windows acceptance result yet.

## Native UI acceptance

Run the kit with `-Interactive` to open its isolated workspace after automated
tests. Record Windows version, GPU/driver, scale settings, and each outcome.
The script deliberately leaves manual acceptance unconfirmed.

- Create, edit, save, rename (including a change of case), trash and restore
  notes. Verify Unicode names, name collisions, and preservation of unknown YAML.
- Search notes; open and save an external Markdown file with spaces and Unicode
  in its absolute path. Restart and switch workspaces to verify global settings.
- Exercise Ctrl+A/C/X/V/Z/Y/S/F, selection, clipboard, undo, native file/folder
  dialogs, and the localized crash dialog using a synthetic failure only.
- Refresh an HTTPS RSS feed, navigate articles, and open the original through
  the existing hardened RSS opener. Check certificate/network error messages.
- Switch all 17 languages, including Arabic and Urdu, with an unsaved editor
  and open menus. Check mixed text and scaling at 100%, 150%, and 200%, including
  moving the window between monitors and the minimum window size.
- Protect a synthetic note, edit it, change the master password, restore a
  backup, and recover unsaved work. Kill the test application during a password
  transition and repeat recovery after restart. Retain incomplete journals on
  any failure; never remove `.notrum/` wholesale.
- Test read-only and externally locked notes, hard links, junctions/reparse
  points, file substitution, and failure injection before/after publication.
  Confirm that errors leave a conflict or recovery, without claiming a save.
- Search persistent service files for the synthetic protected-body sentinel.
  YAML and filenames remain readable; protected bodies must remain age data.
- Measure large-file load, metadata polling, editing, and search performance.
  Windows version snapshots currently read the body in bounded chunks to detect
  writes that restore size and modification time.

## Persistence details

The shared filesystem layer obtains volume/file IDs from open handles, rejects
reparse-point ancestors and non-local Windows prefixes, and retains Windows
ACL entries when replacing a file. A new private file is opened exclusively,
receives a protected DACL for the current user, Administrators, and SYSTEM,
and only then becomes available to the writer. ACL failures are returned.

Replacement uses the checked `MoveFileExW` write-through operation supplied by
`atomicwrites`. Namespace synchronization uses an empty private marker, flushes
it, and moves it with write-through. Successful cleanup leaves no marker; a
crash may leave an empty `.notrum-sync-*` file containing no note data. The
native interruption tests must validate this protocol on the supported NTFS
systems. File synchronization errors are propagated instead of discarded.
Unix continues using file/parent-directory synchronization and mode bits.

Password transaction journals include an explicit platform marker. Windows
entries retain ACLs and content fingerprints; Unix retains inode/change-time
information. Legacy journals without a marker are Unix journals. A journal from
a different platform blocks transaction recovery and is preserved for diagnosis.
Internal relative paths continue to use `/`; absolute external paths and the
Windows home/profile path use native path handling.
