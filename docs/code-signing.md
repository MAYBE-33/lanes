# Code signing policy

Lanes' Windows releases are code-signed so that Windows can verify who
published them and that they have not been altered, and so that Smart App
Control allows them to run.

Free code signing provided by [SignPath.io](https://about.signpath.io/),
certificate by [SignPath Foundation](https://signpath.org/).

> **Status:** signing through the SignPath Foundation programme is being set
> up. Until it is in place, releases are published **unsigned**, and each
> release says so. Once it is, every file listed below is signed for every
> release.

## What is signed

For each release, built by GitHub Actions from a tagged commit in this
repository:

- `Lanes.exe` - the core
- `Lanes.Window.exe` - the window
- `Lanes-<version>-setup.exe` - the installer, and the uninstaller it contains

The publisher shown by Windows is **SignPath Foundation**, which holds the
certificate on behalf of open-source projects. Only binaries built from this
repository's source, by its release workflow, are submitted for signing.

## Team roles

| Role | Who |
| --- | --- |
| Committers and reviewers | [MAYBE-33](https://github.com/MAYBE-33) |
| Approvers | [MAYBE-33](https://github.com/MAYBE-33) |

Changes from anyone outside the committers are reviewed before they are merged.
Every signing request is approved by hand by an approver. All team members use
multi-factor authentication for GitHub and SignPath.

## Privacy

This program will not transfer any information to other networked systems
unless specifically requested by the user or the person installing or operating
it.

Lanes makes no network connections of its own: no telemetry, no update check,
no analytics. Its only network use is the local API it serves on `127.0.0.1`,
which is not reachable from other computers, for the mixer window and the
Stream Deck plugin running on the same PC. Everything it stores - settings,
snapshots of your audio settings, and a log of the changes it makes - stays in
`%LOCALAPPDATA%\Lanes` on your PC. The microphone level meter measures the
signal's peak and discards the audio; nothing is recorded.

## Verifying a download

Right-click the file > **Properties** > **Digital Signatures**. A signed Lanes
release shows **SignPath Foundation** as the signer. Each release also lists the
SHA-256 hash of every file; compare it with
`Get-FileHash <file> -Algorithm SHA256` in PowerShell.
