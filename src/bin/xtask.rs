use std::{
    env,
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::Command,
};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

const APP_NAME: &str = "Watch Bells";
const APP_ICON_NAME: &str = "WatchBells";
const MACOS_BINARY_NAME: &str = "watch-bells";
const MACOS_ZIP_NAME: &str = "watch-bells-macos-aarch64.zip";
const LINUX_BINARY_NAME: &str = "watch-bells-linux-x86_64";
const WINDOWS_BINARY_NAME: &str = "watch-bells-windows-x86_64.exe";

fn main() {
    if let Err(err) = run() {
        eprintln!("xtask error: {err}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let mut args = env::args().skip(1);
    match args.next().as_deref() {
        Some("package-macos") => {
            let binary = take_value(&mut args, "--binary")?;
            let out_dir = take_value(&mut args, "--out")?;
            let version = take_value(&mut args, "--version")?;
            let icon = parse_optional_path(args.collect(), "--icon")?
                .unwrap_or_else(|| PathBuf::from("assets/icon.png"));
            package_macos(&binary, &out_dir, &version, &icon)
        }
        Some("publish-downloads") => {
            let source_dir = take_value(&mut args, "--source")?;
            let website_dir = take_value(&mut args, "--website")?;
            let version = take_value(&mut args, "--version")?;
            publish_downloads(&source_dir, &website_dir, &version)
        }
        Some("help") | None => {
            print_help();
            Ok(())
        }
        Some(command) => Err(format!("unknown xtask command: {command}").into()),
    }
}

fn take_value(args: &mut impl Iterator<Item = String>, flag: &str) -> Result<String> {
    let value = args
        .next()
        .ok_or_else(|| boxed_err(format!("missing {flag} value")))?;
    if value != flag {
        return Err(boxed_err(format!("expected {flag}, got {value}")));
    }

    args.next()
        .ok_or_else(|| boxed_err(format!("missing value after {flag}")))
}

fn parse_optional_path(args: Vec<String>, flag: &str) -> Result<Option<PathBuf>> {
    let mut iter = args.into_iter();
    let mut result = None;

    while let Some(arg) = iter.next() {
        if arg == flag {
            let value = iter
                .next()
                .ok_or_else(|| boxed_err(format!("missing value after {flag}")))?;
            result = Some(PathBuf::from(value));
            continue;
        }

        if let Some(value) = arg.strip_prefix(&format!("{flag}=")) {
            result = Some(PathBuf::from(value));
            continue;
        }

        return Err(boxed_err(format!("unknown argument: {arg}")));
    }

    Ok(result)
}

fn package_macos(binary: &str, out_dir: &str, version: &str, icon: &Path) -> Result<()> {
    let binary = Path::new(binary);
    if !binary.exists() {
        return Err(boxed_err(format!(
            "macOS binary not found at {}",
            binary.display()
        )));
    }
    if !icon.exists() {
        return Err(boxed_err(format!("icon not found at {}", icon.display())));
    }

    let out_dir = Path::new(out_dir);
    fs::create_dir_all(out_dir)?;

    let app_dir = out_dir.join(format!("{APP_NAME}.app"));
    let contents_dir = app_dir.join("Contents");
    let macos_dir = contents_dir.join("MacOS");
    let resources_dir = contents_dir.join("Resources");
    let iconset_dir = resources_dir.join(format!("{APP_ICON_NAME}.iconset"));

    if app_dir.exists() {
        fs::remove_dir_all(&app_dir)?;
    }

    fs::create_dir_all(&macos_dir)?;
    fs::create_dir_all(&resources_dir)?;
    fs::create_dir_all(&iconset_dir)?;

    copy_file(binary, &macos_dir.join(MACOS_BINARY_NAME))?;
    make_executable(&macos_dir.join(MACOS_BINARY_NAME))?;
    build_icns(icon, &iconset_dir, &resources_dir.join(format!("{APP_ICON_NAME}.icns")))?;
    fs::remove_dir_all(&iconset_dir)?;
    write_info_plist(&contents_dir.join("Info.plist"), version)?;

    let zip_path = out_dir.join(MACOS_ZIP_NAME);
    if zip_path.exists() {
        fs::remove_file(&zip_path)?;
    }

    run_command(
        Command::new("ditto")
            .args(["-c", "-k", "--sequesterRsrc", "--keepParent"])
            .arg(&app_dir)
            .arg(&zip_path),
        "create macOS zip",
    )?;

    println!("Created {}", zip_path.display());
    Ok(())
}

fn publish_downloads(source_dir: &str, website_dir: &str, version: &str) -> Result<()> {
    let source_dir = Path::new(source_dir);
    let website_dir = Path::new(website_dir);
    let version_dir = website_dir.join(version);
    let latest_dir = website_dir.join("latest");

    if version_dir.exists() {
        fs::remove_dir_all(&version_dir)?;
    }
    if latest_dir.exists() {
        fs::remove_dir_all(&latest_dir)?;
    }

    fs::create_dir_all(&version_dir)?;
    fs::create_dir_all(&latest_dir)?;

    copy_file(
        &source_dir.join("linux").join(LINUX_BINARY_NAME),
        &version_dir.join(LINUX_BINARY_NAME),
    )?;
    copy_file(
        &source_dir.join("linux").join(LINUX_BINARY_NAME),
        &latest_dir.join(LINUX_BINARY_NAME),
    )?;
    copy_file(
        &source_dir.join("windows").join(WINDOWS_BINARY_NAME),
        &version_dir.join(WINDOWS_BINARY_NAME),
    )?;
    copy_file(
        &source_dir.join("windows").join(WINDOWS_BINARY_NAME),
        &latest_dir.join(WINDOWS_BINARY_NAME),
    )?;

    let macos_zip = source_dir.join("macos").join(MACOS_ZIP_NAME);
    if macos_zip.exists() {
        copy_file(&macos_zip, &version_dir.join(MACOS_ZIP_NAME))?;
        copy_file(&macos_zip, &latest_dir.join(MACOS_ZIP_NAME))?;
    }

    write_text(&latest_dir.join("VERSION"), version)?;
    println!("Published download files for {version}");
    Ok(())
}

fn build_icns(icon: &Path, iconset_dir: &Path, output: &Path) -> Result<()> {
    for size in [16, 32, 128, 256, 512] {
        let small = iconset_dir.join(format!("icon_{size}x{size}.png"));
        let large = iconset_dir.join(format!("icon_{size}x{size}@2x.png"));
        let size = size.to_string();
        let doubled = (size.parse::<u32>()? * 2).to_string();
        run_command(
            Command::new("sips")
                .args(["-z", &size, &size])
                .arg(icon)
                .args(["--out"])
                .arg(&small),
            &format!("generate {size}x{size} icon"),
        )?;
        run_command(
            Command::new("sips")
                .args(["-z", &doubled, &doubled])
                .arg(icon)
                .args(["--out"])
                .arg(&large),
            &format!("generate {size}x{size}@2x icon"),
        )?;
    }

    run_command(
        Command::new("iconutil")
            .args(["-c", "icns"])
            .arg(iconset_dir)
            .args(["-o"])
            .arg(output),
        "build icns",
    )
}

fn write_info_plist(path: &Path, version: &str) -> Result<()> {
    let mut file = fs::File::create(path)?;
    write!(
        file,
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
  <dict>
    <key>CFBundleDevelopmentRegion</key>
    <string>en</string>
    <key>CFBundleExecutable</key>
    <string>{MACOS_BINARY_NAME}</string>
    <key>CFBundleIconFile</key>
    <string>{APP_ICON_NAME}</string>
    <key>CFBundleIdentifier</key>
    <string>com.euanreid.watch-bells</string>
    <key>CFBundleInfoDictionaryVersion</key>
    <string>6.0</string>
    <key>CFBundleName</key>
    <string>{APP_NAME}</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>CFBundleShortVersionString</key>
    <string>{version}</string>
    <key>CFBundleVersion</key>
    <string>{version}</string>
    <key>LSUIElement</key>
    <true/>
    <key>NSHighResolutionCapable</key>
    <true/>
  </dict>
</plist>
"#
    )?;
    Ok(())
}

fn write_text(path: &Path, text: &str) -> Result<()> {
    fs::write(path, format!("{text}\n"))?;
    Ok(())
}

fn copy_file(from: &Path, to: &Path) -> Result<()> {
    if !from.exists() {
        return Err(format!("missing source file {}", from.display()).into());
    }
    if let Some(parent) = to.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::copy(from, to)?;
    Ok(())
}

fn make_executable(path: &Path) -> Result<()> {
    let status = Command::new("chmod").arg("+x").arg(path).status()?;
    if !status.success() {
        return Err(boxed_err(format!("chmod failed for {}", path.display())));
    }
    Ok(())
}

fn run_command(command: &mut Command, label: &str) -> Result<()> {
    let status = command.status()?;
    if !status.success() {
        return Err(boxed_err(format!("{label} failed with status {status}")));
    }
    Ok(())
}

fn print_help() {
    eprintln!(
        "Usage:\n  cargo run --bin xtask -- package-macos --binary <path> --out <dir> --version <v> [--icon <path>]\n  cargo run --bin xtask -- publish-downloads --source <dir> --website <dir> --version <v>"
    );
}

fn boxed_err(message: impl Into<String>) -> Box<dyn std::error::Error> {
    Box::new(std::io::Error::other(message.into()))
}
