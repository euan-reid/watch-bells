# Watch Bells

> Computer time for seafarers

[![CI](https://github.com/euan-reid/watch-bells/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/euan-reid/watch-bells/actions/workflows/ci.yml)

Watch Bells makes your computer's time telling a little more nautical.
The app sits in your system tray and rings maritime ship's bells every
half hour, just like a real ship's chronometer.

## Installation

Download the latest release binary for your platform (macOS, Linux, or Windows)
from <https://www.watchbells.com/downloads/>, make it executable, and run it.
On most systems, you can add it to your startup applications to run automatically.
The same current release files are also available from
[GitHub Releases](https://github.com/euan-reid/watch-bells/releases).

### Platform-Specific Notes

**macOS:**

- The universal application supports both Apple silicon and Intel Macs
- Requires macOS 11 or later on Apple silicon, or macOS 10.13 or later on Intel
- Download the macOS `.zip`, unzip it, and drag `Watch Bells.app` to Applications
  if you want to keep it installed
- Tray icon will appear in the menu bar (top right) with no Dock icon
- To run at startup, add the application to System Settings → General → Login Items

**Linux:**

- Native downloads are available for x86-64 Intel/AMD and ARM64 systems
- Downloads are dynamically linked and require the GTK 3, XCB, XKBCommon,
  ALSA, and xdo runtime libraries
- To build from source on Debian/Ubuntu, install:

  ```bash
  sudo apt install pkg-config libgtk-3-dev libxcb1-dev libxkbcommon-dev \
    libasound2-dev libxdo-dev
  ```

- To build from source on Fedora/RHEL, install:

  ```bash
  sudo dnf install pkg-config gtk3-devel libxcb-devel libxkbcommon-devel \
    alsa-lib-devel libxdo-dev
  ```

- PipeWire users building from source still need the ALSA development headers

**Windows:**

- Requires 64-bit Windows 10 or later on an Intel or AMD processor
- Tray icon will appear in the system tray (bottom right)
- The Windows build is packaged as a GUI app, so double-clicking it will not
  open a console window
- To run at startup, create a shortcut in `%APPDATA%\Microsoft\Windows\Start Menu\Programs\Startup`

### Logging

INFO-level operational events, warnings, and errors are automatically saved
during ordinary GUI launches. The plain-text log is a forensic record of
low-volume activity; DEBUG and noisy routine activity are deliberately not
retained. Log locations are:

- macOS: `~/Library/Logs/Watch Bells/watch-bells.log`
- Windows: `%LOCALAPPDATA%\Watch Bells\watch-bells.log`
- Linux: `$XDG_STATE_HOME/watch-bells/watch-bells.log`, or
  `~/.local/state/watch-bells/watch-bells.log` when `XDG_STATE_HOME` is not set

The current log is rotated live and at startup when it exceeds approximately
512 KiB, retaining one previous `watch-bells.old.log` generation (roughly the
current file plus one previous generation). Rotation and logfile setup are
best effort; they never prevent the application from running.
When stderr is attached to a terminal, `INFO` and above are also printed there.
When `WATCH_BELLS_LOG` is set, `DEBUG` and above are printed for interactive
diagnostics, including GUI launches. DEBUG remains excluded from the persistent
INFO-and-above log.

## Development

Watch Bells is written in Rust for performance and cross-platform compatibility.

### Prerequisites

- [rustup](https://rustup.rs/); the checked-in toolchain file selects stable
  Rust with the formatting and Clippy components
- Git
- Linux only: `pkg-config` and the GTK 3, XCB, XKBCommon, ALSA, and xdo
  development packages listed above

### Building

```bash
git clone https://github.com/euan-reid/watch-bells.git
cd watch-bells
cargo build --release
./target/release/watch-bells
```

The optimized release binary will be in `target/release/watch-bells`
(Unix/Linux) or `target/release/watch-bells.exe` (Windows).

### Running Tests

The project includes comprehensive unit tests for all watch schedules and bell calculations:

```bash
cargo test --workspace --locked
cargo clippy --workspace --locked --all-targets -- -D warnings
```

GUI launches do not require stderr for diagnostics: INFO-level operational
events, warnings, and errors are retained in the local logfile described above.
Set `WATCH_BELLS_LOG` when debug-level interactive output is useful.

All tests verify:

- Each of the 7 watches (First, Middle, Morning, Forenoon,
  Afternoon, FirstDog, LastDog)
- Correct bell counts at every half-hour interval
- The Nore mutiny rule (LastDog never rings 5 bells:
  8-1-2-3-4-1-2-3 pattern)
- Tooltip formatting and next bell boundary calculations

### Packaging

Release packaging and the website download layout live in the dependency-free
`xtask` workspace package, following the
[cargo xtask convention](https://github.com/matklad/cargo-xtask). The
`cargo xtask` alias builds it on first use. The macOS command runs on macOS
because it uses Apple tooling to combine, ad-hoc sign, verify, and package the
two native executables:

```bash
cargo xtask package-macos \
  --arm64-binary <aarch64-apple-darwin-executable> \
  --x86_64-binary <x86_64-apple-darwin-executable> \
  --out dist/macos-universal \
  --version <release-version>
cargo xtask validate-release --source dist/release
cargo xtask publish-downloads \
  --source dist/release \
  --website website/public/downloads \
  --version <release-version>
```

`dist/release` must contain exactly the universal Mac ZIP, Windows x86-64
executable, Linux x86-64 binary, and Linux ARM64 binary. Publication fails before
changing the website tree if that set is incomplete or unexpected.

The public release filenames are:

- `watch-bells-macos-universal.zip`
- `watch-bells-windows-x86_64.exe`
- `watch-bells-linux-x86_64`
- `watch-bells-linux-aarch64`

### Code Structure

The application uses an event-driven architecture:

- **Event loop (main thread):** Manages the system tray icon and menu
- **Scheduler thread (background):** Maintains precise half-hour
  boundary detection and triggers bell playback
- **Audio playback:** Spawned on a background thread to keep the UI responsive
- **Platform integration:** Selected at compile time through Rust target
  configuration

### CI/CD

The project uses GitHub Actions for ordinary validation and tagged releases.
Application changes are checked with workspace formatting, tests, and Clippy,
then compiled natively on the supported systems:

- **Linux:** x86-64 and ARM64 on Ubuntu 22.04 and 24.04
- **Windows:** x86-64 with the MSVC toolchain
- **macOS:** Apple silicon and Intel

Tagged releases use the repository's established unprefixed semantic version
format, such as `0.7.0`, and must identify a commit in `main`. Public Linux
binaries are built on Ubuntu 22.04 for an older glibc/userspace baseline. Both
Mac executables are combined into one verified universal `Watch Bells.app`;
four final files are then published to the website download tree and to a
GitHub Release. See `.github/workflows/ci.yml` and
`.github/workflows/release.yml` for the complete configuration.

## Release History

- 0.7.0
  - Add persistent logging
- 0.6.0
  - Bundle app as GUI application on Windows to avoid console window popping up
  - Bundle app as .app on macOS for easier installation and startup management
- 0.5.1
  - Fix issues relating to system sleep and wake causing incorrect chimes at
  incorrect times
- 0.5.0
  - Complete rewrite in Rust for performance and cross-platform stability
  - Event-driven architecture replacing polling loop for responsive UI
  - Background scheduler thread for precise half-hour boundary detection
  - Real audio playback using system speakers (embedded WAV assets)
  - Comprehensive test suite (13 unit tests covering all watches and bell logic)
  - Optimized release binary: 3.6 MB on macOS with LTO + full optimization
  - Multi-platform hosted CI
- 0.4.1
  - Fix for Nore adjustments
- 0.4.0
  - Bell icon
  - Fix for dog watch name display (correctly show spaces instead
    of underscores, e.g. first dog vs first_dog)
- 0.3.0
  - Bundle sound files, allowing offline usage and avoiding load on freesound.org
- 0.2.0
  - Add mute toggle and hover text for current time
- 0.1.1
  - Correct off-by-one error in bell count
- 0.1.0
  - Initial release (Python)

## Acknowledgements

The ship's bell chimes are public domain and come from Sojan on freesound.org -
thank you to them

## Website

The marketing site lives in `website/` and is plain static HTML/CSS
served by Cloudflare Workers Static Assets.

- Domain: <https://www.watchbells.com>
- Worker config: `website/wrangler.jsonc`
- Static files: `website/public/`
- Downloads landing page: <https://www.watchbells.com/downloads/>
- Versioned release binaries: `website/public/downloads/<tag>/`
- Always-current binaries: `website/public/downloads/latest/`

The tagged GitHub Actions release workflow copies the four final release files
into those `website/public/downloads/` folders and commits them to `main` for
long-term storage. Cloudflare Workers Builds deploys the static site separately;
GitHub Actions contains no Cloudflare credentials or deployment step. The
generated commit uses `GITHUB_TOKEN`, so it deliberately does not start another
GitHub Actions workflow.

Deploy from the `website/` folder:

```bash
cd website
wrangler deploy
```

Project page link:
<https://www.euanreid.com/projects/watch-bells/>

## Maintainers

Euan Reid – [@EuanReid](https://twitter.com/EuanReid)

## Contributing

1. Fork the repository (<https://github.com/euan-reid/watch-bells/fork>)
2. Create a feature branch (`git switch -c feature-stuffed-crust`)
3. Make some changes (`git commit -am 'Added stuffed crust'`)
4. Push to your repo (`git push origin feature-stuffed-crust`)
5. Create a pull request (<https://github.com/euan-reid/watch-bells/compare>)
