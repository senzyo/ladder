use crate::i18n::updates as t;

use std::cmp::Ordering;
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tempfile::TempDir;
use windows_sys::Win32::System::Threading::CREATE_NO_WINDOW;
use zip::ZipArchive;

use crate::update::{UpdateProgress, compare_versions};

const REPOSITORY: &str = "senzyo/ladder";
const MAX_SIZE: u64 = 512 * 1024 * 1024;
const TIMEOUT: Duration = Duration::from_secs(30);
const USER_AGENT: &str = concat!("Ladder/", env!("CARGO_PKG_VERSION"));

#[derive(Clone, Debug)]
pub struct AppRelease {
    pub version: String,
    url: String,
    size: u64,
    sha256: [u8; 32],
}

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    draft: bool,
    prerelease: bool,
    assets: Vec<Asset>,
}

#[derive(Deserialize)]
struct Asset {
    name: String,
    browser_download_url: String,
    size: u64,
    digest: Option<String>,
}

fn architecture() -> &'static str {
    if cfg!(target_arch = "aarch64") {
        "arm64"
    } else {
        "amd64"
    }
}

fn parse_release(data: &[u8], current: &str) -> Result<Option<AppRelease>, String> {
    let release: Release =
        serde_json::from_slice(data).map_err(|_| t::invalid_app_release().to_string())?;
    let version = release
        .tag_name
        .strip_prefix('v')
        .ok_or(t::invalid_app_version())?;
    let version_without_build = version.split('+').next().unwrap();
    if release.draft || release.prerelease || version_without_build.contains('-') {
        return Err(t::app_stable_only().into());
    }
    match compare_versions(version, current) {
        Some(Ordering::Greater) => {}
        Some(_) => return Ok(None),
        None => return Err(t::cannot_compare_app_versions().into()),
    }
    let expected = format!("ladder-{}-v{version}.zip", architecture());
    let official_url = format!(
        "https://github.com/{REPOSITORY}/releases/download/{}/{expected}",
        release.tag_name
    );
    let mut assets = release
        .assets
        .into_iter()
        .filter(|asset| asset.name == expected);
    let asset = assets.next().ok_or(t::missing_app_architecture())?;
    if assets.next().is_some()
        || asset.browser_download_url != official_url
        || asset.size == 0
        || asset.size > MAX_SIZE
    {
        return Err(t::invalid_app_package_info().into());
    }
    let digest = asset
        .digest
        .as_deref()
        .and_then(|value| value.strip_prefix("sha256:"))
        .ok_or(t::missing_app_digest())?;
    let mut sha256 = [0; 32];
    if digest.len() != 64 || !digest.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(t::invalid_app_digest().into());
    }
    for (index, byte) in sha256.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&digest[index * 2..index * 2 + 2], 16).unwrap();
    }
    Ok(Some(AppRelease {
        version: version.into(),
        url: official_url,
        size: asset.size,
        sha256,
    }))
}

pub fn check_latest(prefix: &str) -> Result<Option<AppRelease>, String> {
    let url = crate::github::url(
        prefix,
        &format!("https://api.github.com/repos/{REPOSITORY}/releases/latest"),
    )?;
    let result = ureq::get(&url)
        .config()
        .https_only(true)
        .timeout_global(Some(TIMEOUT))
        .build()
        .header("User-Agent", USER_AGENT)
        .header("Accept", "application/vnd.github+json")
        .call();
    let mut response = match result {
        Err(ureq::Error::StatusCode(404)) => return Err(t::no_app_release().into()),
        other => other.map_err(network_error)?,
    };
    let data = response
        .body_mut()
        .with_config()
        .limit(4 * 1024 * 1024)
        .read_to_vec()
        .map_err(network_error)?;
    parse_release(&data, env!("CARGO_PKG_VERSION"))
}

fn network_error(error: ureq::Error) -> String {
    match error {
        ureq::Error::StatusCode(403 | 429) => t::app_request_limited().into(),
        ureq::Error::StatusCode(404) => t::app_package_gone().into(),
        _ => t::app_request_failed().into(),
    }
}

pub struct PreparedAppUpdate {
    stage: TempDir,
    target: PathBuf,
    sha256: String,
}

pub fn download(
    release: &AppRelease,
    prefix: &str,
    progress: impl FnMut(UpdateProgress),
) -> Result<PreparedAppUpdate, String> {
    let target = std::env::current_exe().map_err(|_| t::cannot_locate_app())?;
    let url = crate::github::url(prefix, &release.url)?;
    download_to(
        release,
        target,
        || {
            let response = ureq::get(&url)
                .config()
                .https_only(true)
                .timeout_resolve(Some(TIMEOUT))
                .timeout_connect(Some(TIMEOUT))
                .timeout_send_request(Some(TIMEOUT))
                .timeout_recv_response(Some(TIMEOUT))
                .timeout_per_read(Some(TIMEOUT))
                .timeout_recv_body(None)
                .build()
                .header("User-Agent", USER_AGENT)
                .call()
                .map_err(network_error)?;
            Ok(response.into_body().into_reader())
        },
        progress,
    )
}

fn download_to<R: Read>(
    release: &AppRelease,
    target: PathBuf,
    open: impl FnOnce() -> Result<R, String>,
    mut progress: impl FnMut(UpdateProgress),
) -> Result<PreparedAppUpdate, String> {
    let root = target.parent().ok_or(t::cannot_locate_app_directory())?;
    let stage = tempfile::Builder::new()
        .prefix(".ladder-app-update-")
        .tempdir_in(root)
        .map_err(|_| t::cannot_create_app_update_directory())?;
    progress(UpdateProgress::Downloading {
        received: 0,
        total: release.size,
    });
    let mut reader = open()?;
    let archive_path = stage.path().join("release.zip");
    let mut archive_file =
        File::create(&archive_path).map_err(|_| t::cannot_create_app_package())?;
    let mut hasher = Sha256::new();
    let mut received = 0;
    let mut buffer = [0; 64 * 1024];
    loop {
        let count = reader
            .read(&mut buffer)
            .map_err(|_| t::app_download_interrupted())?;
        if count == 0 {
            break;
        }
        received += count as u64;
        if received > release.size {
            return Err(t::app_size_mismatch().into());
        }
        archive_file
            .write_all(&buffer[..count])
            .map_err(|_| t::cannot_save_app_package())?;
        hasher.update(&buffer[..count]);
        progress(UpdateProgress::Downloading {
            received,
            total: release.size,
        });
    }
    progress(UpdateProgress::Verifying);
    let digest: [u8; 32] = hasher.finalize().into();
    if received != release.size || digest != release.sha256 {
        return Err(t::app_verification_failed().into());
    }
    archive_file
        .sync_all()
        .map_err(|_| t::cannot_save_app_package())?;
    drop(archive_file);
    progress(UpdateProgress::Extracting);
    let executable = stage.path().join("ladder.exe");
    extract_executable(&archive_path, &executable)?;
    validate_executable(&executable)?;
    let sha256 = file_hash(&executable)?;
    Ok(PreparedAppUpdate {
        stage,
        target,
        sha256,
    })
}

fn extract_executable(archive_path: &Path, executable: &Path) -> Result<(), String> {
    let archive_file = File::open(archive_path).map_err(|_| t::cannot_read_app_package())?;
    let mut archive = ZipArchive::new(archive_file).map_err(|_| t::invalid_app_zip())?;
    let mut found = false;
    let mut total = 0u64;
    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|_| t::corrupt_app_package())?;
        let name = entry
            .name()
            .map_err(|_| t::invalid_app_filename())?
            .into_owned();
        total = total
            .checked_add(entry.size())
            .ok_or(t::app_contents_too_large())?;
        if total > MAX_SIZE
            || entry.is_symlink()
            || !matches!(name.as_str(), "ladder.exe" | "README.md" | "LICENSE")
        {
            return Err(t::unsupported_app_contents().into());
        }
        if name != "ladder.exe" {
            continue;
        }
        if found || entry.is_dir() || entry.size() == 0 {
            return Err(t::invalid_app_executable().into());
        }
        found = true;
        let mut output = File::create(executable).map_err(|_| t::cannot_stage_app())?;
        let copied = std::io::copy(&mut entry.by_ref().take(MAX_SIZE + 1), &mut output)
            .map_err(|_| t::cannot_extract_app())?;
        if copied > MAX_SIZE || copied != entry.size() {
            return Err(t::invalid_new_app_size().into());
        }
        output.sync_all().map_err(|_| t::cannot_save_new_app())?;
    }
    if !found {
        return Err(t::missing_ladder_exe().into());
    }
    Ok(())
}

fn validate_executable(path: &Path) -> Result<(), String> {
    let invalid = || t::invalid_new_app_architecture().to_string();
    let mut file = File::open(path).map_err(|_| invalid())?;
    let length = file.metadata().map_err(|_| invalid())?.len();
    let mut dos = [0; 64];
    file.read_exact(&mut dos).map_err(|_| invalid())?;
    let offset = u64::from(u32::from_le_bytes(dos[60..64].try_into().unwrap()));
    if &dos[..2] != b"MZ" || offset < 64 || offset + 26 > length {
        return Err(invalid());
    }
    file.seek(SeekFrom::Start(offset)).map_err(|_| invalid())?;
    let mut pe = [0; 26];
    file.read_exact(&mut pe).map_err(|_| invalid())?;
    let machine = if cfg!(target_arch = "aarch64") {
        0xaa64
    } else {
        0x8664
    };
    let optional_size = u64::from(u16::from_le_bytes([pe[20], pe[21]]));
    if &pe[..4] != b"PE\0\0"
        || u16::from_le_bytes([pe[4], pe[5]]) != machine
        || optional_size < 64
        || offset + 24 + optional_size > length
        || u16::from_le_bytes([pe[24], pe[25]]) != 0x20b
    {
        return Err(invalid());
    }
    Ok(())
}

fn file_hash(path: &Path) -> Result<String, String> {
    let mut file = File::open(path).map_err(|_| t::cannot_read_new_app())?;
    let mut hasher = Sha256::new();
    let mut buffer = [0; 64 * 1024];
    loop {
        let count = file
            .read(&mut buffer)
            .map_err(|_| t::cannot_verify_new_app())?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    Ok(hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

#[derive(Serialize)]
struct UpdatePlan<'a> {
    target: &'a str,
    sha256: &'a str,
    parent: u32,
}

pub struct UpdateHelper {
    process: Child,
    stage: Option<TempDir>,
}

impl UpdateHelper {
    pub fn commit(mut self) -> Result<(), String> {
        if self
            .process
            .try_wait()
            .map_err(|_| t::cannot_check_update_helper())?
            .is_some()
        {
            return Err(t::update_helper_exited().into());
        }
        let stage = self.stage.as_ref().unwrap();
        fs::write(stage.path().join("commit"), b"commit")
            .map_err(|_| t::cannot_commit_app_update())?;
        let _ = self.stage.take().unwrap().keep();
        Ok(())
    }
}

impl Drop for UpdateHelper {
    fn drop(&mut self) {
        if self.stage.is_some() {
            let _ = self.process.kill();
            let _ = self.process.wait();
        }
    }
}

impl PreparedAppUpdate {
    // 仅在所有核心停止且 DNS 恢复成功后调用；退出前须 commit, 放弃更新则直接 drop。
    pub fn launch(self) -> Result<UpdateHelper, String> {
        let name = self
            .target
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or(t::invalid_executable_name())?;
        let plan = UpdatePlan {
            target: name,
            sha256: &self.sha256,
            parent: std::process::id(),
        };
        let data = serde_json::to_vec(&plan).map_err(|_| t::cannot_create_update_plan())?;
        fs::write(self.stage.path().join("update.json"), data)
            .map_err(|_| t::cannot_save_update_plan())?;
        let script = self.stage.path().join("apply-update.ps1");
        fs::write(&script, include_bytes!("../scripts/apply-update.ps1"))
            .map_err(|_| t::cannot_prepare_update_helper())?;
        let windows = std::env::var_os("SystemRoot").ok_or(t::cannot_locate_windows())?;
        let powershell = Path::new(&windows).join("System32/WindowsPowerShell/v1.0/powershell.exe");
        let process = Command::new(powershell)
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-ExecutionPolicy",
                "Bypass",
                "-File",
            ])
            .arg(&script)
            .creation_flags(CREATE_NO_WINDOW)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|_| t::cannot_start_update_helper())?;
        let mut helper = UpdateHelper {
            process,
            stage: Some(self.stage),
        };
        let deadline = Instant::now() + Duration::from_secs(10);
        while !helper
            .stage
            .as_ref()
            .unwrap()
            .path()
            .join("ready")
            .is_file()
        {
            if helper
                .process
                .try_wait()
                .map_err(|_| t::cannot_check_update_helper())?
                .is_some()
            {
                return Err(t::update_helper_start_failed().into());
            }
            if Instant::now() >= deadline {
                return Err(t::update_helper_timeout().into());
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        Ok(helper)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::io::Cursor;
    use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
    use windows_sys::Win32::Foundation::WAIT_OBJECT_0;
    use windows_sys::Win32::System::Threading::{
        OpenProcess, PROCESS_SYNCHRONIZE, PROCESS_TERMINATE, TerminateProcess, WaitForSingleObject,
    };
    use zip::write::SimpleFileOptions;

    fn release(version: &str) -> serde_json::Value {
        let name = format!("ladder-{}-v{version}.zip", architecture());
        serde_json::json!({ "tag_name": format!("v{version}"), "draft": false, "prerelease": false,
            "assets": [{ "name": name, "browser_download_url": format!("https://github.com/{REPOSITORY}/releases/download/v{version}/{name}"),
                "size": 4096, "digest": format!("sha256:{}", "a".repeat(64)) }] })
    }

    fn package() -> (AppRelease, Vec<u8>, Vec<u8>) {
        let mut executable = vec![0; 152];
        executable[..2].copy_from_slice(b"MZ");
        executable[60..64].copy_from_slice(&64u32.to_le_bytes());
        executable[64..68].copy_from_slice(b"PE\0\0");
        let machine: u16 = if cfg!(target_arch = "aarch64") {
            0xaa64
        } else {
            0x8664
        };
        executable[68..70].copy_from_slice(&machine.to_le_bytes());
        executable[84..86].copy_from_slice(&64u16.to_le_bytes());
        executable[88..90].copy_from_slice(&0x20bu16.to_le_bytes());
        let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
        for (name, contents) in [
            ("ladder.exe", executable.as_slice()),
            ("README.md", b"test documentation".as_slice()),
            ("LICENSE", b"test license".as_slice()),
        ] {
            zip.start_file(name, SimpleFileOptions::default()).unwrap();
            zip.write_all(contents).unwrap();
        }
        let archive = zip.finish().unwrap().into_inner();
        let release = AppRelease {
            version: "4.0.0".into(),
            url: "https://github.com/senzyo/ladder/releases/download/v4.0.0/test.zip".into(),
            size: archive.len() as u64,
            sha256: Sha256::digest(&archive).into(),
        };
        (release, archive, executable)
    }

    pub(crate) fn prepared_update() -> (TempDir, AppRelease, PreparedAppUpdate) {
        let root = tempfile::tempdir().unwrap();
        let target = root.path().join("ladder.exe");
        fs::write(&target, b"old program").unwrap();
        let (release, archive, _) = package();
        let prepared = download_to(&release, target, || Ok(archive.as_slice()), |_| {}).unwrap();
        (root, release, prepared)
    }

    #[test]
    fn downloads_verify_and_stage_the_program_without_replacing_the_current_one() {
        let root = tempfile::tempdir().unwrap();
        let target = root.path().join("ladder.exe");
        fs::write(&target, b"old program").unwrap();
        let (release, archive, executable) = package();
        let mut progress = Vec::new();
        let prepared = download_to(
            &release,
            target.clone(),
            || Ok(archive.as_slice()),
            |value| progress.push(value),
        )
        .unwrap();
        assert_eq!(fs::read(&target).unwrap(), b"old program");
        assert_eq!(prepared.target, target);
        assert_eq!(prepared.stage.path().parent(), Some(root.path()));
        assert_eq!(
            fs::read(prepared.stage.path().join("ladder.exe")).unwrap(),
            executable
        );
        let expected_hash = if cfg!(target_arch = "aarch64") {
            "549fc36846d0e7dddf6bd4da15689f76a6df95971a3929f5e78b51be439afd41"
        } else {
            "b4c68064bf6b6b5982134be37f32df9a0364e7da6438b8e951845960eda93887"
        };
        assert_eq!(prepared.sha256, expected_hash);
        assert_eq!(
            progress.first(),
            Some(&UpdateProgress::Downloading {
                received: 0,
                total: release.size,
            })
        );
        assert!(progress.contains(&UpdateProgress::Downloading {
            received: release.size,
            total: release.size,
        }));
        assert!(progress.ends_with(&[UpdateProgress::Verifying, UpdateProgress::Extracting]));
        let stage = prepared.stage.path().to_owned();
        drop(prepared);
        assert!(!stage.exists());
        assert_eq!(fs::read(&target).unwrap(), b"old program");
    }

    #[test]
    fn rejected_downloads_preserve_the_program_and_remove_staging() {
        let root = tempfile::tempdir().unwrap();
        let target = root.path().join("ladder.exe");
        fs::write(&target, b"old program").unwrap();
        let (release, archive, _) = package();
        for scenario in ["truncated", "oversized", "wrong-hash"] {
            let mut release = release.clone();
            let mut bytes = archive.clone();
            match scenario {
                "truncated" => {
                    bytes.pop();
                }
                "oversized" => bytes.push(0),
                "wrong-hash" => release.sha256 = [0; 32],
                _ => unreachable!(),
            }
            let result = download_to(&release, target.clone(), || Ok(bytes.as_slice()), |_| {});
            let error = result.err().expect(scenario);
            assert!(
                error.contains(if scenario == "oversized" {
                    "大小与发布信息不符"
                } else {
                    "大小或 SHA-256 校验失败"
                }),
                "{scenario}: {error}"
            );
            assert_eq!(fs::read(&target).unwrap(), b"old program", "{scenario}");
            assert_eq!(fs::read_dir(root.path()).unwrap().count(), 1, "{scenario}");
        }
    }

    #[test]
    fn releases_require_newer_stable_official_assets_and_digest() {
        let parse =
            |value: &serde_json::Value| parse_release(&serde_json::to_vec(value).unwrap(), "1.2.3");
        assert!(parse(&release("1.2.3")).unwrap().is_none());
        assert!(parse(&release("1.2.2")).unwrap().is_none());
        assert_eq!(
            parse(&release("1.10.0")).unwrap().unwrap().version,
            "1.10.0"
        );
        let mut value = release("2.0.0");
        value["prerelease"] = true.into();
        assert!(parse(&value).is_err());
        value["prerelease"] = false.into();
        value["assets"][0]["browser_download_url"] = "https://example.org/ladder.zip".into();
        assert!(parse(&value).is_err());
        let mut value = release("2.0.0");
        value["assets"][0]["digest"] = serde_json::Value::Null;
        assert!(parse(&value).is_err());
    }

    #[test]
    fn rewrite_preview_skips_legacy_releases_and_accepts_its_stable_release() {
        let mut legacy = release("2.34.0");
        legacy["assets"] = serde_json::json!([]);
        assert!(
            parse_release(&serde_json::to_vec(&legacy).unwrap(), "3.0.0-alpha.1")
                .unwrap()
                .is_none()
        );
        let stable = parse_release(
            &serde_json::to_vec(&release("3.0.0")).unwrap(),
            "3.0.0-alpha.1",
        )
        .unwrap()
        .unwrap();
        assert_eq!(stable.version, "3.0.0");
    }

    #[test]
    fn releases_reject_prerelease_tags_even_without_github_flag() {
        for version in ["3.0.0-alpha.2", "3.1.0-beta.1", "3.1.0-rc.1+build-2"] {
            let result = parse_release(
                &serde_json::to_vec(&release(version)).unwrap(),
                "3.0.0-alpha.1",
            );
            assert!(result.is_err(), "accepted prerelease tag {version}");
        }
        let stable = parse_release(
            &serde_json::to_vec(&release("3.0.0+build-2")).unwrap(),
            "3.0.0-alpha.1",
        )
        .unwrap()
        .unwrap();
        assert_eq!(stable.version, "3.0.0+build-2");
    }

    #[test]
    fn archives_extract_only_the_program_and_reject_unexpected_paths() {
        let root = tempfile::tempdir().unwrap();
        for (name, valid) in [
            ("ladder.exe", true),
            ("../ladder.exe", false),
            ("ladder-state.json", false),
        ] {
            let archive = root.path().join("release.zip");
            let mut zip = zip::ZipWriter::new(File::create(&archive).unwrap());
            zip.start_file(name, SimpleFileOptions::default()).unwrap();
            zip.write_all(b"program").unwrap();
            zip.finish().unwrap();
            let output = root.path().join("new.exe");
            assert_eq!(extract_executable(&archive, &output).is_ok(), valid);
        }
    }

    #[test]
    fn invalid_executables_are_rejected_without_execution() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("program.exe");
        fs::write(&path, b"not a program").unwrap();
        assert!(validate_executable(&path).is_err());
    }

    #[test]
    fn dropping_uncommitted_helper_stops_it_and_removes_staging() {
        let stage = tempfile::tempdir().unwrap();
        let path = stage.path().to_owned();
        let powershell = Path::new(&std::env::var_os("SystemRoot").unwrap())
            .join("System32/WindowsPowerShell/v1.0/powershell.exe");
        let process = Command::new(powershell)
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "Start-Sleep -Seconds 60",
            ])
            .creation_flags(CREATE_NO_WINDOW)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let mut helper = UpdateHelper {
            process,
            stage: Some(stage),
        };
        assert!(helper.process.try_wait().unwrap().is_none());
        let handle = unsafe {
            OpenProcess(
                PROCESS_SYNCHRONIZE | PROCESS_TERMINATE,
                0,
                helper.process.id(),
            )
        };
        assert!(!handle.is_null());
        let handle = unsafe { OwnedHandle::from_raw_handle(handle) };
        drop(helper);
        let status = unsafe { WaitForSingleObject(handle.as_raw_handle(), 5000) };
        if status != WAIT_OBJECT_0 {
            unsafe { TerminateProcess(handle.as_raw_handle(), 1) };
        }
        assert_eq!(status, WAIT_OBJECT_0, "uncommitted helper remained alive");
        assert!(!path.exists());
    }
}
