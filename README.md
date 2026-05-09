# Watch Bells

> Computer time for seafarers

[![pipeline status](https://gitlab.com/euan/watch-bells/badges/master/pipeline.svg)](https://gitlab.com/euan/watch-bells/commits/master)
[![test coverage](https://gitlab.com/euan/watch-bells/badges/master/coverage.svg)](https://gitlab.com/euan/watch-bells/commits/master)

Watch Bells makes your computer's time telling a little more nautical.
The app sits in your system tray and rings maritime ship's bells every
half hour, just like a real ship's chronometer.

## Installation

Download the latest release binary for your platform (macOS, Linux, or Windows),
make it executable, and run it. On most systems, you can add it to your startup
applications to run automatically.

### Platform-Specific Notes

**macOS:**

- Requires macOS 10.13 or later
- Tray icon will appear in the menu bar (top right)
- To run at startup, add the application to System Settings → General → Login Items

**Linux:**

- Requires pkg-config, GTK3, XCB, XKBCommon, and ALSA dev headers
- Debian/Ubuntu:

  ```bash
  sudo apt install pkg-config libgtk-3-dev libxcb1-dev libxkbcommon-dev \
    libasound2-dev libxdo-dev
  ```

- Fedora/RHEL:

  ```bash
  sudo dnf install pkg-config gtk3-devel libxcb-devel libxkbcommon-devel \
    alsa-lib-devel libxdo-dev
  ```

- PipeWire users: alsa-lib dev headers are still needed for audio output

**Windows:**

- Requires Windows 7 or later
- Tray icon will appear in the system tray (bottom right)
- To run at startup, create a shortcut in `%APPDATA%\Microsoft\Windows\Start Menu\Programs\Startup`

## Development

Watch Bells is written in Rust for performance and cross-platform compatibility.

### Prerequisites

- Rust 1.70+ (install via [rustup](https://rustup.rs/))
- Git
- Linux only: `libxcb1-dev`, `libxkbcommon-dev`, `libasound2-dev`
  (see Platform-Specific Notes above)

### Building

```bash
git clone https://gitlab.com/euan/watch-bells.git
cd watch-bells
cargo build --release
./target/release/watch-bells
```

The optimized release binary will be in `target/release/watch-bells`
(Unix/Linux) or `target/release/watch-bells.exe` (Windows).

### Running Tests

The project includes comprehensive unit tests for all watch schedules and bell calculations:

```bash
cargo test --verbose
```

All tests verify:

- Each of the 7 watches (First, Middle, Morning, Forenoon,
  Afternoon, FirstDog, LastDog)
- Correct bell counts at every half-hour interval
- The Nore mutiny rule (LastDog never rings 5 bells:
  8-1-2-3-4-1-2-3 pattern)
- Tooltip formatting and next bell boundary calculations

### Code Structure

The application uses an event-driven architecture:

- **Event loop (main thread):** Manages the system tray icon and menu
- **Scheduler thread (background):** Maintains precise half-hour
  boundary detection and triggers bell playback
- **Audio playback:** Spawned on a background thread to keep the UI responsive
- **Configuration:** All platform-specific settings in `.cargo/config.toml`

### CI/CD

The project uses GitLab CI for automated builds on all three platforms:

- **Linux (x86-64):** Native build on latest Rust toolchain
- **Windows (x86-64):** Cross-compiled from Linux using mingw-w64
- **macOS (Apple Silicon):** Native build on hosted runner

See `.gitlab-ci.yml` for build configuration.

## Release History

- 0.5.0
  - Complete rewrite in Rust for performance and cross-platform stability
  - Event-driven architecture replacing polling loop for responsive UI
  - Background scheduler thread for precise half-hour boundary detection
  - Real audio playback using system speakers (embedded WAV assets)
  - Comprehensive test suite (13 unit tests covering all watches and bell logic)
  - Optimized release binary: 3.6 MB on macOS with LTO + full optimization
  - Multi-platform CI on GitLab.com hosted runners
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

1. Fork the repo (<https://gitlab.com/euan/watch-bells/forks/new>)
2. Create a feature branch (`git checkout -b feature-stuffed-crust`)
3. Make some changes (`git commit -am 'Added stuffed crust'`)
4. Push to your repo (`git push origin feature-stuffed-crust`)
5. Create a new Merge Request (<https://gitlab.com/euan/watch-bells/merge_requests/new>)
