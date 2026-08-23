use std::{
    collections::BTreeSet,
    env, fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Output},
};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

const APP_NAME: &str = "Watch Bells";
const APP_ICON_NAME: &str = "WatchBells";
const MACOS_BINARY_NAME: &str = "watch-bells";
const MACOS_UNIVERSAL_ZIP_NAME: &str = "watch-bells-macos-universal.zip";
const MACOS_ARM64_MINIMUM_VERSION: &str = "11.0.0";
const MACOS_X86_64_MINIMUM_VERSION: &str = "10.13.0";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ReleaseArtifact {
    label: &'static str,
    file_name: &'static str,
    executable: bool,
}

const RELEASE_ARTIFACTS: [ReleaseArtifact; 4] = [
    ReleaseArtifact {
        label: "macOS universal application",
        file_name: MACOS_UNIVERSAL_ZIP_NAME,
        executable: false,
    },
    ReleaseArtifact {
        label: "Windows x86-64 executable",
        file_name: "watch-bells-windows-x86_64.exe",
        executable: true,
    },
    ReleaseArtifact {
        label: "Linux x86-64 binary",
        file_name: "watch-bells-linux-x86_64",
        executable: true,
    },
    ReleaseArtifact {
        label: "Linux ARM64 binary",
        file_name: "watch-bells-linux-aarch64",
        executable: true,
    },
];

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
            let arm64_binary = take_value(&mut args, "--arm64-binary")?;
            let x86_64_binary = take_value(&mut args, "--x86_64-binary")?;
            let out_dir = take_value(&mut args, "--out")?;
            let version = take_value(&mut args, "--version")?;
            let icon = parse_optional_path(args.collect(), "--icon")?
                .unwrap_or_else(|| PathBuf::from("assets/icon.png"));
            package_macos(
                Path::new(&arm64_binary),
                Path::new(&x86_64_binary),
                Path::new(&out_dir),
                &version,
                &icon,
            )
        }
        Some("validate-release") => {
            let source_dir = take_value(&mut args, "--source")?;
            ensure_no_arguments(args)?;
            validate_release_artifacts(Path::new(&source_dir))?;
            println!("Validated all required release artifacts");
            Ok(())
        }
        Some("publish-downloads") => {
            let source_dir = take_value(&mut args, "--source")?;
            let website_dir = take_value(&mut args, "--website")?;
            let version = take_value(&mut args, "--version")?;
            ensure_no_arguments(args)?;
            publish_downloads(Path::new(&source_dir), Path::new(&website_dir), &version)
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

fn ensure_no_arguments(mut args: impl Iterator<Item = String>) -> Result<()> {
    if let Some(argument) = args.next() {
        return Err(boxed_err(format!("unknown argument: {argument}")));
    }
    Ok(())
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

fn package_macos(
    arm64_binary: &Path,
    x86_64_binary: &Path,
    out_dir: &Path,
    version: &str,
    icon: &Path,
) -> Result<()> {
    validate_release_version(version)?;
    validate_macos_binary(arm64_binary, "arm64")?;
    validate_macos_binary(x86_64_binary, "x86_64")?;
    if !icon.is_file() {
        return Err(boxed_err(format!("icon not found at {}", icon.display())));
    }

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

    let universal_binary = macos_dir.join(MACOS_BINARY_NAME);
    run_command(
        Command::new("lipo")
            .arg("-create")
            .arg(arm64_binary)
            .arg(x86_64_binary)
            .arg("-output")
            .arg(&universal_binary),
        "create universal macOS executable",
    )?;
    make_executable(&universal_binary)?;
    validate_universal_macos_binary(&universal_binary)?;

    build_icns(
        icon,
        &iconset_dir,
        &resources_dir.join(format!("{APP_ICON_NAME}.icns")),
    )?;
    fs::remove_dir_all(&iconset_dir)?;
    write_info_plist(&contents_dir.join("Info.plist"), version)?;

    let zip_path = out_dir.join(MACOS_UNIVERSAL_ZIP_NAME);
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

fn validate_macos_binary(binary: &Path, expected_architecture: &str) -> Result<()> {
    let expected = BTreeSet::from([expected_architecture.to_owned()]);
    let actual = lipo_architectures(binary)?;
    if actual != expected {
        return Err(boxed_err(format!(
            "{} must contain only {expected_architecture}; found {}",
            binary.display(),
            format_architectures(&actual)
        )));
    }
    Ok(())
}

fn validate_universal_macos_binary(binary: &Path) -> Result<()> {
    let expected = BTreeSet::from(["arm64".to_owned(), "x86_64".to_owned()]);
    let actual = lipo_architectures(binary)?;
    if actual != expected {
        return Err(boxed_err(format!(
            "{} must contain arm64 and x86_64; found {}",
            binary.display(),
            format_architectures(&actual)
        )));
    }

    println!(
        "Verified universal macOS executable architectures: {}",
        format_architectures(&actual)
    );
    Ok(())
}

fn lipo_architectures(binary: &Path) -> Result<BTreeSet<String>> {
    if !binary.is_file() {
        return Err(boxed_err(format!(
            "macOS binary not found at {}",
            binary.display()
        )));
    }

    let output = run_command_output(
        Command::new("lipo").arg("-archs").arg(binary),
        "inspect macOS executable architectures",
    )?;
    let output = String::from_utf8(output.stdout)?;
    let architectures = output
        .split_whitespace()
        .map(str::to_owned)
        .collect::<BTreeSet<_>>();
    if architectures.is_empty() {
        return Err(boxed_err(format!(
            "lipo reported no architectures for {}",
            binary.display()
        )));
    }
    Ok(architectures)
}

fn format_architectures(architectures: &BTreeSet<String>) -> String {
    architectures
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .join(", ")
}

fn publish_downloads(source_dir: &Path, website_dir: &Path, version: &str) -> Result<()> {
    let release_version = parse_release_version(version)?;
    validate_release_artifacts(source_dir)?;

    let version_dir = website_dir.join(version);
    let latest_dir = website_dir.join("latest");
    let current_version_path = latest_dir.join("VERSION");
    if current_version_path.is_file() {
        let current_version = fs::read_to_string(&current_version_path)?;
        let current_version = current_version.trim();
        if parse_release_version(current_version)? > release_version {
            return Err(boxed_err(format!(
                "refusing to replace newer latest release {current_version} with {version}"
            )));
        }
    }
    let staging_dir = website_dir.join(format!(".publish-{version}-{}", std::process::id()));
    let staged_version_dir = staging_dir.join("version");
    let staged_latest_dir = staging_dir.join("latest");

    if staging_dir.exists() {
        fs::remove_dir_all(&staging_dir)?;
    }
    fs::create_dir_all(&staged_version_dir)?;
    fs::create_dir_all(&staged_latest_dir)?;

    let stage_result = (|| -> Result<()> {
        copy_release_artifacts(source_dir, &staged_version_dir)?;
        copy_release_artifacts(source_dir, &staged_latest_dir)?;
        write_text(&staged_latest_dir.join("VERSION"), version)
    })();
    if let Err(err) = stage_result {
        fs::remove_dir_all(&staging_dir)?;
        return Err(err);
    }

    if version_dir.exists() {
        fs::remove_dir_all(&version_dir)?;
    }
    fs::rename(&staged_version_dir, &version_dir)?;

    if latest_dir.exists() {
        fs::remove_dir_all(&latest_dir)?;
    }
    fs::rename(&staged_latest_dir, &latest_dir)?;
    fs::remove_dir_all(&staging_dir)?;

    println!("Published download files for {version}");
    Ok(())
}

fn validate_release_artifacts(source_dir: &Path) -> Result<()> {
    if !source_dir.is_dir() {
        return Err(boxed_err(format!(
            "release source directory not found at {}",
            source_dir.display()
        )));
    }

    let expected = RELEASE_ARTIFACTS
        .iter()
        .map(|artifact| artifact.file_name.to_owned())
        .collect::<BTreeSet<_>>();
    let mut actual = BTreeSet::new();

    for entry in fs::read_dir(source_dir)? {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            return Err(boxed_err(format!(
                "unexpected non-file release entry {}",
                entry.path().display()
            )));
        }
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| boxed_err("release artifact filename is not valid UTF-8"))?;
        actual.insert(name);
    }

    if actual != expected {
        let missing = expected.difference(&actual).cloned().collect::<Vec<_>>();
        let unexpected = actual.difference(&expected).cloned().collect::<Vec<_>>();
        return Err(boxed_err(format!(
            "release artifact set is incomplete or unexpected; missing: {}; unexpected: {}",
            format_file_names(&missing),
            format_file_names(&unexpected)
        )));
    }

    for artifact in RELEASE_ARTIFACTS {
        let path = source_dir.join(artifact.file_name);
        if fs::metadata(&path)?.len() == 0 {
            return Err(boxed_err(format!(
                "{} is empty at {}",
                artifact.label,
                path.display()
            )));
        }
    }
    Ok(())
}

fn format_file_names(file_names: &[String]) -> String {
    if file_names.is_empty() {
        "none".to_owned()
    } else {
        file_names.join(", ")
    }
}

fn copy_release_artifacts(source_dir: &Path, destination_dir: &Path) -> Result<()> {
    for artifact in RELEASE_ARTIFACTS {
        let destination = destination_dir.join(artifact.file_name);
        copy_file(&source_dir.join(artifact.file_name), &destination)?;
        if artifact.executable {
            make_executable(&destination)?;
        }
    }
    Ok(())
}

fn validate_release_version(version: &str) -> Result<()> {
    parse_release_version(version).map(|_| ())
}

fn parse_release_version(version: &str) -> Result<[u64; 3]> {
    let components = version.split('.').collect::<Vec<_>>();
    if components.len() != 3
        || components.iter().any(|component| {
            component.is_empty()
                || !component
                    .bytes()
                    .all(|character| character.is_ascii_digit())
                || (component.len() > 1 && component.starts_with('0'))
                || component.parse::<u64>().is_err()
        })
    {
        return Err(boxed_err(format!(
            "release version must be a stable semantic version such as 0.7.0, got {version}"
        )));
    }
    Ok([
        components[0].parse()?,
        components[1].parse()?,
        components[2].parse()?,
    ])
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

    write_icns(iconset_dir, output)
}

fn write_icns(iconset_dir: &Path, output: &Path) -> Result<()> {
    let entries = [
        (*b"icp4", "icon_16x16.png"),
        (*b"ic11", "icon_16x16@2x.png"),
        (*b"icp5", "icon_32x32.png"),
        (*b"icp6", "icon_32x32@2x.png"),
        (*b"ic12", "icon_32x32@2x.png"),
        (*b"ic07", "icon_128x128.png"),
        (*b"ic13", "icon_128x128@2x.png"),
        (*b"ic08", "icon_256x256.png"),
        (*b"ic14", "icon_256x256@2x.png"),
        (*b"ic09", "icon_512x512.png"),
        (*b"ic10", "icon_512x512@2x.png"),
    ];
    let mut encoded_entries = Vec::with_capacity(entries.len());
    let mut total_size = 8_u32;

    for (icon_type, file_name) in entries {
        let data = fs::read(iconset_dir.join(file_name))?;
        if data.is_empty() {
            return Err(boxed_err(format!("generated icon {file_name} is empty")));
        }
        let entry_size = u32::try_from(data.len())?
            .checked_add(8)
            .ok_or_else(|| boxed_err("ICNS entry size overflow"))?;
        total_size = total_size
            .checked_add(entry_size)
            .ok_or_else(|| boxed_err("ICNS file size overflow"))?;
        encoded_entries.push((icon_type, entry_size, data));
    }

    let mut file = fs::File::create(output)?;
    file.write_all(b"icns")?;
    file.write_all(&total_size.to_be_bytes())?;
    for (icon_type, entry_size, data) in encoded_entries {
        file.write_all(&icon_type)?;
        file.write_all(&entry_size.to_be_bytes())?;
        file.write_all(&data)?;
    }
    Ok(())
}

fn write_info_plist(path: &Path, version: &str) -> Result<()> {
    validate_release_version(version)?;
    let mut file = fs::File::create(path)?;
    write!(
        file,
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
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
    <key>LSMinimumSystemVersionByArchitecture</key>
    <dict>
      <key>arm64</key>
      <string>{MACOS_ARM64_MINIMUM_VERSION}</string>
      <key>x86_64</key>
      <string>{MACOS_X86_64_MINIMUM_VERSION}</string>
    </dict>
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
    if !from.is_file() {
        return Err(boxed_err(format!("missing source file {}", from.display())));
    }
    if let Some(parent) = to.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::copy(from, to)?;
    Ok(())
}

#[cfg(unix)]
fn make_executable(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    let mut permissions = fs::metadata(path)?.permissions();
    permissions.set_mode(permissions.mode() | 0o111);
    fs::set_permissions(path, permissions)?;
    Ok(())
}

#[cfg(not(unix))]
fn make_executable(_path: &Path) -> Result<()> {
    Ok(())
}

fn run_command(command: &mut Command, label: &str) -> Result<()> {
    run_command_output(command, label).map(|_| ())
}

fn run_command_output(command: &mut Command, label: &str) -> Result<Output> {
    let output = command.output()?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        return Err(boxed_err(format!(
            "{label} failed with status {}: {stderr}",
            output.status
        )));
    }
    Ok(output)
}

fn print_help() {
    eprintln!("Usage:");
    eprintln!("  cargo run --bin xtask -- package-macos");
    eprintln!("    --arm64-binary <path> --x86_64-binary <path>");
    eprintln!("    --out <dir> --version <v> [--icon <path>]");
    eprintln!("  cargo run --bin xtask -- validate-release --source <dir>");
    eprintln!("  cargo run --bin xtask -- publish-downloads");
    eprintln!("    --source <dir> --website <dir> --version <v>");
}

fn boxed_err(message: impl Into<String>) -> Box<dyn std::error::Error> {
    Box::new(std::io::Error::other(message.into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_TEMP_DIRECTORY: AtomicU64 = AtomicU64::new(0);

    struct TempDirectory(PathBuf);

    impl TempDirectory {
        fn new() -> Self {
            let sequence = NEXT_TEMP_DIRECTORY.fetch_add(1, Ordering::Relaxed);
            let path = env::temp_dir().join(format!(
                "watch-bells-xtask-{}-{sequence}",
                std::process::id()
            ));
            fs::create_dir(&path).expect("create temporary test directory");
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDirectory {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).expect("remove temporary test directory");
        }
    }

    fn create_complete_release(source_dir: &Path) {
        fs::create_dir_all(source_dir).expect("create release source");
        for artifact in RELEASE_ARTIFACTS {
            fs::write(
                source_dir.join(artifact.file_name),
                format!("{} contents", artifact.label),
            )
            .expect("write release artifact");
        }
    }

    #[test]
    fn release_artifact_filenames_are_stable() {
        let filenames = RELEASE_ARTIFACTS
            .iter()
            .map(|artifact| artifact.file_name)
            .collect::<Vec<_>>();
        assert_eq!(
            filenames,
            [
                "watch-bells-macos-universal.zip",
                "watch-bells-windows-x86_64.exe",
                "watch-bells-linux-x86_64",
                "watch-bells-linux-aarch64",
            ]
        );
    }

    #[test]
    fn publication_requires_the_exact_complete_release_set_before_mutating() {
        let temporary = TempDirectory::new();
        let source_dir = temporary.path().join("source");
        let website_dir = temporary.path().join("downloads");
        create_complete_release(&source_dir);
        fs::remove_file(source_dir.join("watch-bells-linux-aarch64"))
            .expect("remove required artifact");
        fs::write(source_dir.join("unexpected.txt"), "unexpected")
            .expect("write unexpected artifact");
        fs::create_dir_all(website_dir.join("latest")).expect("create existing latest");
        fs::write(website_dir.join("latest/VERSION"), "0.6.0\n")
            .expect("write existing version marker");

        let error = publish_downloads(&source_dir, &website_dir, "0.7.0")
            .expect_err("incomplete release must fail")
            .to_string();

        assert!(error.contains("watch-bells-linux-aarch64"));
        assert!(error.contains("unexpected.txt"));
        assert_eq!(
            fs::read_to_string(website_dir.join("latest/VERSION"))
                .expect("read existing version marker"),
            "0.6.0\n"
        );
        assert!(!website_dir.join("0.7.0").exists());
    }

    #[test]
    fn publication_populates_versioned_and_latest_layouts() {
        let temporary = TempDirectory::new();
        let source_dir = temporary.path().join("source");
        let website_dir = temporary.path().join("downloads");
        create_complete_release(&source_dir);
        fs::create_dir_all(website_dir.join("0.5.0")).expect("create historical release");
        fs::write(website_dir.join("0.5.0/keep"), "historical").expect("write historical release");

        publish_downloads(&source_dir, &website_dir, "0.7.0").expect("publish complete release");

        for artifact in RELEASE_ARTIFACTS {
            let expected = fs::read(source_dir.join(artifact.file_name))
                .expect("read source release artifact");
            assert_eq!(
                fs::read(website_dir.join("0.7.0").join(artifact.file_name))
                    .expect("read versioned release artifact"),
                expected
            );
            assert_eq!(
                fs::read(website_dir.join("latest").join(artifact.file_name))
                    .expect("read latest release artifact"),
                expected
            );
        }
        assert_eq!(
            fs::read_to_string(website_dir.join("latest/VERSION"))
                .expect("read new version marker"),
            "0.7.0\n"
        );
        assert_eq!(
            fs::read_to_string(website_dir.join("0.5.0/keep")).expect("read historical release"),
            "historical"
        );
    }

    #[test]
    fn publication_does_not_replace_a_newer_latest_release() {
        let temporary = TempDirectory::new();
        let source_dir = temporary.path().join("source");
        let website_dir = temporary.path().join("downloads");
        create_complete_release(&source_dir);
        fs::create_dir_all(website_dir.join("latest")).expect("create existing latest");
        fs::write(website_dir.join("latest/VERSION"), "0.8.0\n")
            .expect("write newer version marker");

        let error = publish_downloads(&source_dir, &website_dir, "0.7.0")
            .expect_err("release downgrade must fail")
            .to_string();

        assert!(error.contains("newer latest release 0.8.0"));
        assert_eq!(
            fs::read_to_string(website_dir.join("latest/VERSION"))
                .expect("read existing version marker"),
            "0.8.0\n"
        );
        assert!(!website_dir.join("0.7.0").exists());
    }

    #[test]
    fn release_versions_are_safe_stable_semantic_versions() {
        for valid in ["0.7.0", "10.20.300"] {
            validate_release_version(valid).expect("valid semantic version");
        }
        for invalid in ["v0.7.0", "0.7", "0.7.0-beta.1", "01.2.3", "../0.7.0"] {
            assert!(validate_release_version(invalid).is_err(), "{invalid}");
        }
    }

    #[test]
    fn plist_preserves_bundle_metadata_and_architecture_minimums() {
        let temporary = TempDirectory::new();
        let plist_path = temporary.path().join("Info.plist");

        write_info_plist(&plist_path, "0.7.0").expect("write plist");
        let plist = fs::read_to_string(plist_path).expect("read plist");

        assert!(plist.contains("com.euanreid.watch-bells"));
        assert!(plist.contains("<key>LSUIElement</key>"));
        assert!(plist.contains("<string>0.7.0</string>"));
        assert!(plist.contains("<key>arm64</key>\n      <string>11.0.0</string>"));
        assert!(plist.contains("<key>x86_64</key>\n      <string>10.13.0</string>"));
    }

    #[test]
    fn icns_writer_emits_a_complete_big_endian_container() {
        let temporary = TempDirectory::new();
        let iconset_dir = temporary.path().join("WatchBells.iconset");
        let output = temporary.path().join("WatchBells.icns");
        fs::create_dir(&iconset_dir).expect("create iconset");
        for file_name in [
            "icon_16x16.png",
            "icon_16x16@2x.png",
            "icon_32x32.png",
            "icon_32x32@2x.png",
            "icon_128x128.png",
            "icon_128x128@2x.png",
            "icon_256x256.png",
            "icon_256x256@2x.png",
            "icon_512x512.png",
            "icon_512x512@2x.png",
        ] {
            fs::write(iconset_dir.join(file_name), file_name).expect("write test icon");
        }

        write_icns(&iconset_dir, &output).expect("write ICNS container");
        let icns = fs::read(output).expect("read ICNS container");

        assert_eq!(&icns[0..4], b"icns");
        assert_eq!(
            u32::from_be_bytes(icns[4..8].try_into().expect("read ICNS size")) as usize,
            icns.len()
        );
        for icon_type in [
            b"icp4", b"ic11", b"icp5", b"icp6", b"ic12", b"ic07", b"ic13", b"ic08", b"ic14",
            b"ic09", b"ic10",
        ] {
            assert!(
                icns.windows(4).any(|window| window == icon_type),
                "missing {} entry",
                String::from_utf8_lossy(icon_type)
            );
        }
    }
}
