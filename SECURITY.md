# Security policy

## Supported versions

Only the latest release on the
[releases page](https://github.com/tsubaie/RusTXT/releases/latest) receives
fixes. There are no maintenance branches for older versions; a fix ships as a
new release.

## Reporting a vulnerability

Please do not open a public issue for a security problem. Use GitHub's private
vulnerability reporting for this repository:

<https://github.com/tsubaie/RusTXT/security/advisories/new>

Include the RusTXT version, the operating system, and steps to reproduce.
Reports are acknowledged as soon as they are read; this is a one-person project
with no fixed response window. When a fix is released the advisory is published
with credit to the reporter unless they ask otherwise.

## Scope

In scope:

- The RusTXT application: file loading and saving, the recovery database,
  configuration and theme parsing, single-instance forwarding, the print view,
  and anything else reachable through a file, a command-line argument, or the
  user interface.
- The release packaging and the `install.sh` scripts in this repository.
- The vendored Iced patches under `vendor/` (upstream Iced bugs should go to
  Iced, but a patch that introduces a problem belongs here).

Out of scope:

- Problems that require an attacker to already have the user's account or write
  access to the user's home directory. RusTXT stores its data as the user and
  does not defend against the user's own account being compromised.
- Vulnerabilities in the operating system, the desktop portal, the browser used
  for printing, or third-party packagings not produced by this repository.

## Data handling

Be aware of what RusTXT keeps on disk; it is written for recoverability, not
for secrecy.

**Recovery database.** Every open tab is snapshotted to a SQLite database about
250 ms after each edit. It lives at:

| Platform | Location |
|---|---|
| Linux | `$XDG_DATA_HOME/rustxt/session.db`, by default `~/.local/share/rustxt/session.db` |
| macOS | `~/.local/share/rustxt/session.db` |
| Windows | `%LOCALAPPDATA%\rustxt\session.db` |

`RUSTXT_DATA_DIR` moves it. The database contains, in plaintext:

- the full text of every unsaved note and of every open file as last snapshotted;
- the undo/redo history of each tab (the changed spans, which together
  reconstruct earlier versions of the text), up to roughly 8 MiB per tab;
- the paths of open and recently closed files, cursor and scroll positions, and
  window geometry;
- recently closed tabs: unsaved notes are kept until *Discard changes and close*
  is chosen; the twenty most recently closed saved files are remembered by path.

Nothing in it is encrypted. Anyone who can read the file can read everything
you typed, including text you later deleted while the undo history still holds
it. *Discard changes and close* deletes a note's snapshot and its undo history
from the database; SQLite may leave the bytes in free pages until the file is
vacuumed or overwritten.

Starting with 0.6.0 the database is created with mode `0600` on Linux and
macOS. Databases created by earlier releases keep whatever mode they were
created with (the process umask, typically `0644`); run
`chmod 600 ~/.local/share/rustxt/session.db*` once if that matters to you. On
Windows the file inherits the permissions of `%LOCALAPPDATA%`, which is private
to the user by default.

**Configuration.** `config.toml` and custom themes contain only appearance
settings and are written with the process umask.

**Print view.** *File → Print…* writes the current document as HTML to
`print.html` in the cache directory (`~/.cache/rustxt/` on Linux and macOS,
`%LOCALAPPDATA%\rustxt\cache\` on Windows) and opens it in the default browser.
The file is overwritten on the next print and is not deleted afterwards; delete
it yourself if the document was sensitive. The browser may keep its own copies
or history entries.

**Saved files.** Saves go through a temporary file in the target directory and
an atomic rename, preserving the original permissions and writing through
symlinks. The temporary file briefly exists next to the target.

**Network.** The application makes no network requests. The About dialog opens
the releases page in the browser only when you click the link. The updater
that existed in the GTK-era releases is removed.

## Release integrity

Every release ships a `SHA256SUMS` file; verify downloads against it. Build
provenance attestations for the release artifacts are being added to the
release workflow; where present they let you confirm that an artifact was built
by this repository's workflow from a given commit.

Binaries are **not** code-signed with a developer certificate: Windows
executables are unsigned, and macOS bundles are ad-hoc signed and not notarized.
A checksum match confirms the download is the one the release workflow
produced; it does not vouch for the identity of the publisher the way a signed
binary would. Linux packages (`.deb`, `.rpm`, `.pkg.tar.zst`) are likewise
unsigned and are meant to be installed from the release page after checking the
checksum.
