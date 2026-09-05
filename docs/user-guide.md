# User guide

## AI settings

Open Settings → AI settings. Paste an OpenAI or Anthropic API key; the provider
is detected locally. Connect fetches the model catalog and stores the key in the
system credential store. This checks catalog access, not generation permissions
or billing. Use Change key to replace or remove it, or Refresh models to update
the available models. Verification never sends notes.

After connecting, Model aliases lets you name model/effort combinations for use
throughout the product. Changing an alias updates the model used by every task
that refers to that name. There is no fixed number of aliases. Use Add alias to
create one, or click an existing alias to change its name, model, or effort.
Names are case-sensitive, trimmed on save, and must be unique and nonempty.

The built-in `default` starts with GPT-5.6 Luna for OpenAI or Claude Sonnet 5 for
Anthropic, both with `high` effort. These reviewed defaults are explicit because
the catalogs do not expose a consistent model-tier field. An available dated
snapshot is used if the canonical model ID is absent. If neither is listed,
`default` is shown as unavailable; select an accessible model before using it.
Refreshing the catalog or replacing a key for the same provider never changes a
saved alias automatically.

You can change the model and effort of `default`, but cannot delete or rename
it. Deleted, renamed, or unknown alias names resolve to `default`. An existing
alias with an unavailable model or effort returns an error instead of silently
switching models; the same applies if `default` itself is unavailable. New
aliases start with the model and effort from `default`. Changing the model keeps
a compatible effort, otherwise selecting `high` or the first supported level.
The model dropdown only includes models with a supported effort parameter. Choose effort from the second
dropdown; there is no separate model search field.

Settings apply across workspaces. Switching providers clears the aliases and
creates a new provider-specific `default`. Network errors retain the previous
connection. On Linux, unlock the desktop Secret Service if key storage fails.
No key is written to a settings file. The retired Small/Medium/Large profiles
are not migrated to aliases.

Assistant commands and text generation will be added separately.

[Back to README](../README.md)

The interface defaults to English, independently of your operating system.

## Language

Choose **Settings → General → Language**. Language names are displayed in their
own language. The choice applies immediately and is remembered for all workspaces
in `~/.notrum.cfg`. Existing installations without a saved choice use English.

Available languages are English, Spanish, Russian, Simplified and Traditional
Chinese, Brazilian and European Portuguese, Hindi, Arabic, French, Bengali,
Indonesian, Urdu, German, Japanese, Turkish, and Korean. Arabic and Urdu place
the navigation sidebar on the right. Notes, tags, filenames, and RSS articles
retain their original content. New notes receive a title in the selected language.

Translations are included in the application and work offline. System-owned
file dialog controls follow the operating system's language. Technical diagnostic
details remain in English.

## Workspaces and settings

On first launch without an explicit or saved workspace, Notrum offers to
create `~/Downloads/Notes` or choose another folder. Filesystem changes happen
only after confirmation. The selected folder is the workspace root: its notes
live in `notes/`, for example `~/Downloads/Notes/notes/`.
An unavailable saved workspace brings you back to the folder selection screen.

After successfully opening a workspace, Notrum remembers its absolute path in
`~/.notrum.cfg`. Workspace layout and the list of external files are stored in
`<workspace>/.notrum/settings.json`. Use the settings screen to change the
workspace. See [Storage and security](storage.md) for what to preserve when
moving or backing up a workspace.

## Notes and organization

Notes use UTF-8 Markdown with YAML front matter compatible with Notable.
Categories come from YAML tags. Notes can be favorited, pinned, and soft-deleted.
Categories and Favorites support manual order and automatic sorting.
Editing uses autosave; recovery and conflict handling help preserve work after
a crash or an external edit. Local search indexes workspace notes.

Notrum preserves unknown front matter fields and unrelated files. Opening a
workspace does not rewrite notes. It does not scan nested note directories or
follow note symlinks.

## External files and desktop opening

External `.md`, `.markdown`, and `.txt` files open as complete UTF-8 text without
parsing YAML front matter. They remain at their original locations and are not
copied into `notes/` or added to the workspace search index. Their ordered list
is saved separately for each workspace.

The close control in External removes only the sidebar reference;
it never deletes the external file.

The macOS bundle declares support for these extensions, but does not replace
your existing default editor automatically. In Finder, select a file, open
Get Info (`⌘I`), choose Notrum under Open with, and select Change All if you
want it to become the default. Double-clicking a file or choosing Open with
then delivers it to the running application or launches Notrum. The file
appears in the active workspace's External group.

On Linux, run the package's `python3 Register.py` to add Notrum to Open With;
`python3 Register.py --remove` removes that registration. Python 3 is needed only
for registration. On Windows, run `powershell -NoProfile -File .\Register.ps1`;
add `-Remove` to unregister. Neither registration changes your default editor.
Register again after moving the portable package.

You can also pass files directly: `notrum --open first.md second.txt`, optionally
with `--workspace /path/to/workspace`. A directory argument alone keeps its
workspace-selection meaning. Requests wait for workspace selection on first
launch. Linux and Windows can open a new window; Finder normally reuses the
running macOS application.

Different windows keep independent editor state. A conflicting save or recovery
write is reported instead of overwriting another window's work. Independent
settings changes are merged; conflicting changes to the same setting require
closing and reopening the workspace. Resolve conflicts before closing a window
with unsaved edits.

Search and Find hints show Cmd on macOS and Ctrl on Windows/Linux. The latter
also support Ctrl+Home/End, Ctrl+Shift+Home/End and Ctrl+Y. AltGr text input stays
available.

## RSS and Atom

Choose `+` → RSS feed and enter a direct HTTPS feed URL. RSS 1.0, RSS 2.0,
and Atom are supported. Subscriptions appear alongside notes; entries open in
a native, read-only feed view. A feed refreshes when opened and through its
toolbar button. There is no background refresh schedule.

The first ten entries in the first successful response are marked unread.
Scrolling alone does not change read status: open a card by clicking it or
using `J`/`K`. Selecting a card scrolls it to the top of the feed. Read cards
are dimmed, with additional contrast for the selected card.

Cards show a bold sans-serif title, the author and local date, then a serif
Markdown excerpt. They neither execute HTML nor load remote images. Clicking
a linked title opens the original article in the system browser and marks the
card read. If an entry has no original link, a suitable HTTPS link from its
excerpt can be used instead. Without a suitable link, the title is plain text.
Other excerpt links are displayed as text.

Feeds are fetched over HTTPS without cookies or authentication. Opening an
article explicitly hands its HTTPS URL to the system browser. Subscriptions,
cached entries, and read status have different storage roles; see the
[storage guide](storage.md).

## Protected notes

Protected notes encrypt their Markdown body, while the filename and YAML
metadata stay readable. They require the workspace master password.
The password can be changed under Settings → Encryption.

Changing the password does not re-encrypt existing backup history. Keep the
previous password if you need to read older encrypted backups. See
[Storage and security](storage.md) for the recovery and backup details.
