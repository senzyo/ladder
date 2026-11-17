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
        serde_json::from_slice(data).map_err(|_| "GitHub 返回了无效的软件发布信息".to_string())?;
    let version = release
        .tag_name
        .strip_prefix('v')
        .ok_or("软件发布版本号无效")?;
    let version_without_build = version.split('+').next().unwrap();
    if release.draft || release.prerelease || version_without_build.contains('-') {
        return Err("软件更新只接受正式发布版本".into());
    }
    match compare_versions(version, current) {
        Some(Ordering::Greater) => {}
        Some(_) => return Ok(None),
        None => return Err("无法比较软件版本号".into()),
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
    let asset = assets
        .next()
        .ok_or("该版本没有适用于当前架构的软件安装包")?;
    if assets.next().is_some()
        || asset.browser_download_url != official_url
        || asset.size == 0
        || asset.size > MAX_SIZE
    {
        return Err("软件安装包发布信息无效".into());
    }
    let digest = asset
        .digest
        .as_deref()
        .and_then(|value| value.strip_prefix("sha256:"))
        .ok_or("软件安装包缺少 GitHub SHA-256 校验信息")?;
    let mut sha256 = [0; 32];
    if digest.len() != 64 || !digest.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("软件安装包 SHA-256 校验信息无效".into());
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
        Err(ureq::Error::StatusCode(404)) => return Err("尚无可用的 Ladder 正式发行版".into()),
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
        ureq::Error::StatusCode(403 | 429) => "GitHub 请求受限，请稍后重试或更换加速前缀".into(),
        ureq::Error::StatusCode(404) => "软件安装包已不存在，请重新检查更新".into(),
        _ => "软件更新请求失败或超时，请检查网络后重试".into(),
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
    mut progress: impl FnMut(UpdateProgress),
) -> Result<PreparedAppUpdate, String> {
    let target = std::env::current_exe().map_err(|_| "无法定位软件程序")?;
    let root = target.parent().ok_or("无法定位程序目录")?;
    let stage = tempfile::Builder::new()
        .prefix(".ladder-app-update-")
        .tempdir_in(root)
        .map_err(|_| "无法创建软件更新目录，请检查程序目录权限")?;
    let url = crate::github::url(prefix, &release.url)?;
    progress(UpdateProgress::Downloading {
        received: 0,
        total: release.size,
    });
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
    let mut reader = response.into_body().into_reader();
    let archive_path = stage.path().join("release.zip");
    let mut archive_file = File::create(&archive_path).map_err(|_| "无法保存软件安装包")?;
    let mut hasher = Sha256::new();
    let mut received = 0;
    let mut buffer = [0; 64 * 1024];
    loop {
        let count = reader
            .read(&mut buffer)
            .map_err(|_| "软件安装包下载中断或超时")?;
        if count == 0 {
            break;
        }
        received += count as u64;
        if received > release.size {
            return Err("软件安装包大小与发布信息不符".into());
        }
        archive_file
            .write_all(&buffer[..count])
            .map_err(|_| "保存软件安装包失败")?;
        hasher.update(&buffer[..count]);
        progress(UpdateProgress::Downloading {
            received,
            total: release.size,
        });
    }
    progress(UpdateProgress::Verifying);
    let digest: [u8; 32] = hasher.finalize().into();
    if received != release.size || digest != release.sha256 {
        return Err("软件安装包大小或 SHA-256 校验失败，当前软件未修改".into());
    }
    archive_file.sync_all().map_err(|_| "保存软件安装包失败")?;
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
    let archive_file = File::open(archive_path).map_err(|_| "无法读取软件安装包")?;
    let mut archive = ZipArchive::new(archive_file).map_err(|_| "软件安装包不是有效的 ZIP")?;
    let mut found = false;
    let mut total = 0u64;
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).map_err(|_| "软件安装包损坏")?;
        let name = entry
            .name()
            .map_err(|_| "软件安装包文件名无效")?
            .into_owned();
        total = total
            .checked_add(entry.size())
            .ok_or("软件安装包内容过大")?;
        if total > MAX_SIZE
            || entry.is_symlink()
            || !matches!(name.as_str(), "ladder.exe" | "README.md" | "LICENSE")
        {
            return Err("软件安装包含不支持的文件或内容过大".into());
        }
        if name != "ladder.exe" {
            continue;
        }
        if found || entry.is_dir() || entry.size() == 0 {
            return Err("软件安装包的程序文件无效".into());
        }
        found = true;
        let mut output = File::create(executable).map_err(|_| "无法暂存新版程序")?;
        let copied = std::io::copy(&mut entry.by_ref().take(MAX_SIZE + 1), &mut output)
            .map_err(|_| "解压软件安装包失败")?;
        if copied > MAX_SIZE || copied != entry.size() {
            return Err("新版程序大小无效".into());
        }
        output.sync_all().map_err(|_| "保存新版程序失败")?;
    }
    if !found {
        return Err("软件安装包缺少 ladder.exe".into());
    }
    Ok(())
}

fn validate_executable(path: &Path) -> Result<(), String> {
    let invalid = || "新版程序不是适用于当前架构的 Windows 程序".to_string();
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
    let mut file = File::open(path).map_err(|_| "无法读取新版程序")?;
    let mut hasher = Sha256::new();
    let mut buffer = [0; 64 * 1024];
    loop {
        let count = file.read(&mut buffer).map_err(|_| "无法校验新版程序")?;
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
            .map_err(|_| "无法检查软件更新助手")?
            .is_some()
        {
            return Err("软件更新助手已退出，当前软件未修改".into());
        }
        let stage = self.stage.as_ref().unwrap();
        fs::write(stage.path().join("commit"), b"commit")
            .map_err(|_| "无法确认软件更新，当前软件未修改")?;
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
    // 仅在所有核心停止且 DNS 恢复成功后调用；退出前须 commit，放弃更新则直接 drop。
    pub fn launch(self) -> Result<UpdateHelper, String> {
        let name = self
            .target
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or("程序文件名无效")?;
        let plan = UpdatePlan {
            target: name,
            sha256: &self.sha256,
            parent: std::process::id(),
        };
        let data = serde_json::to_vec(&plan).map_err(|_| "无法生成软件更新计划")?;
        fs::write(self.stage.path().join("update.json"), data)
            .map_err(|_| "无法保存软件更新计划")?;
        let script = self.stage.path().join("apply-update.ps1");
        fs::write(&script, include_bytes!("../scripts/apply-update.ps1"))
            .map_err(|_| "无法准备软件更新助手")?;
        let windows = std::env::var_os("SystemRoot").ok_or("无法定位 Windows 系统目录")?;
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
            .map_err(|_| "无法启动软件更新助手，当前软件未修改")?;
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
                .map_err(|_| "无法检查软件更新助手")?
                .is_some()
            {
                return Err("软件更新助手启动失败，当前软件未修改".into());
            }
            if Instant::now() >= deadline {
                return Err("软件更新助手未能及时就绪，当前软件未修改".into());
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        Ok(helper)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use zip::write::SimpleFileOptions;

    fn release(version: &str) -> serde_json::Value {
        let name = format!("ladder-{}-v{version}.zip", architecture());
        serde_json::json!({ "tag_name": format!("v{version}"), "draft": false, "prerelease": false,
            "assets": [{ "name": name, "browser_download_url": format!("https://github.com/{REPOSITORY}/releases/download/v{version}/{name}"),
                "size": 4096, "digest": format!("sha256:{}", "a".repeat(64)) }] })
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
        drop(helper);
        assert!(!path.exists());
    }
}
