use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tempfile::TempDir;
use ureq::{RequestBuilder, typestate::WithoutBody};
use zip::ZipArchive;

use crate::managed_child::JobProcess;
use crate::runtime::Core;

const NETWORK_TIMEOUT: Duration = Duration::from_secs(30);
const VERSION_TIMEOUT: Duration = Duration::from_secs(3);
const MAX_ARCHIVE_SIZE: u64 = 512 * 1024 * 1024;
const MAX_EXTRACTED_SIZE: u64 = 1024 * 1024 * 1024;
const USER_AGENT: &str = concat!("Ladder/", env!("CARGO_PKG_VERSION"));

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum UpdateChannel {
    #[default]
    Stable,
    Preview,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UpdateProgress {
    Checking,
    Downloading { received: u64, total: u64 },
    Verifying,
    Extracting,
    Installing,
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
    #[serde(rename = "browser_download_url")]
    url: String,
    size: u64,
    digest: Option<String>,
}

struct Download {
    version: String,
    url: String,
    size: u64,
    sha256: [u8; 32],
}

fn repository(core: Core) -> &'static str {
    match core {
        Core::SingBox => "SagerNet/sing-box",
        Core::Xray => "XTLS/Xray-core",
    }
}

fn asset_name(core: Core, version: &str) -> String {
    match core {
        Core::SingBox => format!(
            "sing-box-{version}-windows-{}.zip",
            if cfg!(target_arch = "aarch64") {
                "arm64"
            } else {
                "amd64"
            }
        ),
        Core::Xray => if cfg!(target_arch = "aarch64") {
            "Xray-windows-arm64-v8a.zip"
        } else {
            "Xray-windows-64.zip"
        }
        .into(),
    }
}

fn artifacts(core: Core) -> &'static [&'static str] {
    match core {
        Core::SingBox => &["sing-box.exe", "libcronet.dll"],
        Core::Xray => &["xray.exe", "wintun.dll", "geoip.dat", "geosite.dat"],
    }
}

fn numeric_version(version: &str) -> Option<Vec<u32>> {
    let values: Vec<_> = version.split('.').collect();
    if !(2..=4).contains(&values.len())
        || values
            .iter()
            .any(|part| part.is_empty() || !part.bytes().all(|b| b.is_ascii_digit()))
    {
        return None;
    }
    values.into_iter().map(|part| part.parse().ok()).collect()
}

fn needs_update(local: Option<&str>, remote: &str) -> bool {
    let Some(local) = local else {
        return true;
    };
    compare_versions(remote, local).is_none_or(|order| order.is_gt())
}

fn version_parts(version: &str) -> Option<(Vec<u32>, Option<&str>)> {
    fn valid_identifiers(value: &str) -> bool {
        value.split('.').all(|part| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        })
    }
    if version.len() > 64 {
        return None;
    }
    let version = match version.split_once('+') {
        Some((version, build)) if valid_identifiers(build) => version,
        Some(_) => return None,
        None => version,
    };
    let (base, prerelease) = match version.split_once('-') {
        Some((base, prerelease)) if valid_identifiers(prerelease) => (base, Some(prerelease)),
        Some(_) => return None,
        None => (version, None),
    };
    Some((numeric_version(base)?, prerelease))
}

fn compare_versions(left: &str, right: &str) -> Option<std::cmp::Ordering> {
    use std::cmp::Ordering;
    let (mut left, left_pre) = version_parts(left)?;
    let (mut right, right_pre) = version_parts(right)?;
    let length = left.len().max(right.len());
    left.resize(length, 0);
    right.resize(length, 0);
    let order = left.cmp(&right);
    if order != Ordering::Equal {
        return Some(order);
    }
    Some(match (left_pre, right_pre) {
        (None, None) => Ordering::Equal,
        (None, Some(_)) => Ordering::Greater,
        (Some(_), None) => Ordering::Less,
        (Some(left), Some(right)) => {
            let mut left = left.split('.');
            let mut right = right.split('.');
            loop {
                let order = match (left.next(), right.next()) {
                    (None, None) => break Ordering::Equal,
                    (None, Some(_)) => break Ordering::Less,
                    (Some(_), None) => break Ordering::Greater,
                    (Some(left), Some(right)) => {
                        match (
                            left.bytes().all(|byte| byte.is_ascii_digit()),
                            right.bytes().all(|byte| byte.is_ascii_digit()),
                        ) {
                            (true, true) => {
                                let left = left.trim_start_matches('0');
                                let right = right.trim_start_matches('0');
                                left.len().cmp(&right.len()).then_with(|| left.cmp(right))
                            }
                            (true, false) => Ordering::Less,
                            (false, true) => Ordering::Greater,
                            (false, false) => left.cmp(right),
                        }
                    }
                };
                if order != Ordering::Equal {
                    break order;
                }
            }
        }
    })
}

fn parse_release(core: Core, channel: UpdateChannel, data: &[u8]) -> Result<Download, String> {
    let format_error = |_| "官方发布信息格式无效，请稍后重试".to_string();
    let release: Release = match channel {
        UpdateChannel::Stable => serde_json::from_slice(data).map_err(format_error)?,
        UpdateChannel::Preview => {
            let releases: Vec<Release> = serde_json::from_slice(data).map_err(format_error)?;
            releases
                .into_iter()
                .filter(|release| {
                    let version = release.tag_name.trim_start_matches('v');
                    !release.draft
                        && version_parts(version).is_some()
                        && release
                            .assets
                            .iter()
                            .any(|asset| asset.name == asset_name(core, version))
                })
                .max_by(|left, right| {
                    compare_versions(
                        left.tag_name.trim_start_matches('v'),
                        right.tag_name.trim_start_matches('v'),
                    )
                    .expect("release versions were validated")
                })
                .ok_or("官方发布列表中没有适用于当前架构的可安装版本")?
        }
    };
    let version = release
        .tag_name
        .strip_prefix('v')
        .unwrap_or(&release.tag_name);
    let Some((_, prerelease)) = version_parts(version) else {
        return Err("官方响应不是可安装的版本".into());
    };
    if release.draft
        || (channel == UpdateChannel::Stable && (release.prerelease || prerelease.is_some()))
    {
        return Err("官方响应不是所选通道的可安装版本".into());
    }
    let name = asset_name(core, version);
    let mut assets = release
        .assets
        .into_iter()
        .filter(|asset| asset.name == name);
    let asset = assets
        .next()
        .ok_or("官方发布版本缺少当前架构的 Windows 安装包")?;
    if assets.next().is_some() {
        return Err("官方发布信息包含重复的安装包".into());
    }
    let expected_url = format!(
        "https://github.com/{}/releases/download/{}/{}",
        repository(core),
        release.tag_name,
        name
    );
    if asset.url != expected_url {
        return Err("安装包下载地址不是对应的官方发布地址".into());
    }
    if asset.size == 0 || asset.size > MAX_ARCHIVE_SIZE {
        return Err("官方安装包大小无效或超出限制".into());
    }
    let digest = asset
        .digest
        .as_deref()
        .and_then(|value| value.strip_prefix("sha256:"))
        .filter(|value| value.len() == 64 && value.bytes().all(|b| b.is_ascii_hexdigit()))
        .ok_or("官方安装包缺少有效的 SHA-256 校验值")?;
    let mut sha256 = [0; 32];
    for (index, value) in sha256.iter_mut().enumerate() {
        *value = u8::from_str_radix(&digest[index * 2..index * 2 + 2], 16)
            .expect("digest was validated");
    }
    Ok(Download {
        version: version.into(),
        url: asset.url,
        size: asset.size,
        sha256,
    })
}

fn network_error(error: ureq::Error) -> String {
    match error {
        ureq::Error::StatusCode(403 | 429) => "GitHub 暂时限制请求，请稍后重试".into(),
        ureq::Error::StatusCode(404) => "官方发布文件暂不可用，请稍后重试".into(),
        ureq::Error::StatusCode(code) => format!("下载服务返回 HTTP {code}，请稍后重试"),
        ureq::Error::Timeout(_) => "连接或下载超时，请检查网络后重试".into(),
        _ => "无法连接官方发布服务，请检查网络、代理和证书后重试".into(),
    }
}

fn check_cancelled(cancelled: &AtomicBool) -> Result<(), String> {
    if cancelled.load(Ordering::Acquire) {
        Err("已取消更新".into())
    } else {
        Ok(())
    }
}

pub fn prepare(
    core: Core,
    channel: UpdateChannel,
    root: &Path,
    local_version: Option<&str>,
    cancelled: &AtomicBool,
    mut progress: impl FnMut(UpdateProgress),
) -> Result<Option<PreparedUpdate>, String> {
    check_cancelled(cancelled)?;
    progress(UpdateProgress::Checking);
    let endpoint = match channel {
        UpdateChannel::Stable => "releases/latest",
        UpdateChannel::Preview => "releases?per_page=30",
    };
    let url = format!(
        "https://api.github.com/repos/{}/{endpoint}",
        repository(core)
    );
    let data = ureq::get(&url)
        .config()
        .https_only(true)
        .timeout_global(Some(NETWORK_TIMEOUT))
        .build()
        .header("User-Agent", USER_AGENT)
        .header("Accept", "application/vnd.github+json")
        .call()
        .map_err(network_error)?
        .body_mut()
        .with_config()
        .limit(16 * 1024 * 1024)
        .read_to_vec()
        .map_err(network_error)?;
    check_cancelled(cancelled)?;
    let download = parse_release(core, channel, &data)?;
    if !needs_update(local_version, &download.version) {
        return Ok(None);
    }
    let mut prepared = PreparedUpdate::new(core, root, download.version.clone())?;
    let archive = prepared.directory.path().join("package.zip");
    let request = ureq::get(&download.url).config().https_only(true).build();
    download_archive(
        request,
        &download,
        &archive,
        cancelled,
        &mut progress,
        NETWORK_TIMEOUT,
    )?;
    check_cancelled(cancelled)?;
    progress(UpdateProgress::Extracting);
    prepared.extract(core, &archive, cancelled)?;
    let staged_executable = prepared
        .directory
        .path()
        .join("new")
        .join(artifacts(core)[0]);
    let staged_version = read_version(core, &staged_executable, VERSION_TIMEOUT)
        .map_err(|message| format!("安装包验证失败：{message}；现有核心未修改"))?;
    if staged_version != prepared.version {
        return Err("安装包中的核心版本与官方发布信息不符，现有核心未修改".into());
    }
    check_cancelled(cancelled)?;
    Ok(Some(prepared))
}

fn download_archive(
    request: RequestBuilder<WithoutBody>,
    download: &Download,
    path: &Path,
    cancelled: &AtomicBool,
    progress: &mut impl FnMut(UpdateProgress),
    timeout: Duration,
) -> Result<(), String> {
    check_cancelled(cancelled)?;
    progress(UpdateProgress::Downloading {
        received: 0,
        total: download.size,
    });
    let response = request
        .config()
        .timeout_resolve(Some(timeout))
        .timeout_connect(Some(timeout))
        .timeout_send_request(Some(timeout))
        .timeout_recv_response(Some(timeout))
        .timeout_per_read(Some(timeout))
        .timeout_recv_body(None)
        .build()
        .header("User-Agent", USER_AGENT)
        .call()
        .map_err(network_error)?;
    let mut reader = response.into_body().into_reader();
    let mut file = File::create(path).map_err(|e| format!("创建下载临时文件失败: {e}"))?;
    let mut received = 0;
    let mut buffer = [0; 64 * 1024];
    let mut hasher = Sha256::new();
    loop {
        check_cancelled(cancelled)?;
        let count = reader
            .read(&mut buffer)
            .map_err(|_| "下载中断或超时，请检查网络后重试".to_string())?;
        if count == 0 {
            break;
        }
        received += count as u64;
        if received > download.size {
            return Err("下载大小与官方发布信息不符，请重试".into());
        }
        file.write_all(&buffer[..count])
            .map_err(|e| format!("保存下载文件失败: {e}"))?;
        hasher.update(&buffer[..count]);
        progress(UpdateProgress::Downloading {
            received,
            total: download.size,
        });
    }
    check_cancelled(cancelled)?;
    if received != download.size {
        return Err("安装包下载不完整，请重试".into());
    }
    progress(UpdateProgress::Verifying);
    let digest: [u8; 32] = hasher.finalize().into();
    if digest != download.sha256 {
        return Err("安装包 SHA-256 校验失败，现有核心未修改，请重试".into());
    }
    file.sync_all()
        .map_err(|e| format!("保存下载文件失败: {e}"))?;
    Ok(())
}

#[derive(PartialEq)]
enum InstallState {
    Prepared,
    Installed,
    RolledBack,
    Committed,
}

struct Replacement {
    name: &'static str,
    backed_up: bool,
    installed: bool,
}

pub struct PreparedUpdate {
    pub version: String,
    core: Core,
    directory: TempDir,
    target: PathBuf,
    files: Vec<Replacement>,
    state: InstallState,
}

impl PreparedUpdate {
    fn new(core: Core, root: &Path, version: String) -> Result<Self, String> {
        let directory = tempfile::Builder::new()
            .prefix(&format!(".ladder-update-{}-", core.label()))
            .tempdir_in(root)
            .map_err(|e| format!("创建更新暂存目录失败: {e}"))?;
        fs::create_dir(directory.path().join("new"))
            .map_err(|e| format!("创建更新暂存目录失败: {e}"))?;
        Ok(Self {
            version,
            core,
            directory,
            target: core.core_dir(root),
            files: Vec::new(),
            state: InstallState::Prepared,
        })
    }

    fn extract(
        &mut self,
        core: Core,
        archive: &Path,
        cancelled: &AtomicBool,
    ) -> Result<(), String> {
        let file = File::open(archive).map_err(|e| format!("打开安装包失败: {e}"))?;
        let mut archive = ZipArchive::new(file).map_err(|_| "安装包不是有效的 ZIP 文件")?;
        let mut total = 0u64;
        for index in 0..archive.len() {
            check_cancelled(cancelled)?;
            let mut entry = archive.by_index(index).map_err(|_| "无法读取安装包条目")?;
            let name = entry.name().map_err(|_| "安装包包含无效文件名")?;
            if entry.enclosed_name().is_none()
                || Path::new(name.as_ref())
                    .components()
                    .any(|part| !matches!(part, Component::Normal(_)))
                || name.contains(':')
                || name.split(['/', '\\']).any(|part| part == "..")
                || entry.is_symlink()
            {
                return Err("安装包包含不安全路径或符号链接".into());
            }
            if entry.is_dir() {
                continue;
            }
            let Some(name) = Path::new(name.as_ref())
                .file_name()
                .and_then(|value| value.to_str())
            else {
                return Err("安装包包含无效文件名".into());
            };
            let Some(&name) = artifacts(core)
                .iter()
                .find(|allowed| allowed.eq_ignore_ascii_case(name))
            else {
                continue;
            };
            if self.files.iter().any(|file| file.name == name) {
                return Err(format!("安装包包含重复的 {name}"));
            }
            let size = entry.size();
            total = total
                .checked_add(size)
                .filter(|size| *size <= MAX_EXTRACTED_SIZE)
                .ok_or("安装包解压大小超出限制")?;
            if size == 0 {
                return Err(format!("安装包中的 {name} 为空"));
            }
            let mut output = File::create(self.directory.path().join("new").join(name))
                .map_err(|e| format!("创建暂存文件失败: {e}"))?;
            let mut written = 0;
            let mut buffer = [0; 64 * 1024];
            loop {
                check_cancelled(cancelled)?;
                let count = entry
                    .read(&mut buffer)
                    .map_err(|_| "安装包解压失败或内容损坏")?;
                if count == 0 {
                    break;
                }
                written += count as u64;
                if written > size {
                    return Err("安装包解压大小与条目信息不符".into());
                }
                output
                    .write_all(&buffer[..count])
                    .map_err(|e| format!("写入暂存文件失败: {e}"))?;
            }
            if written != size {
                return Err("安装包条目不完整".into());
            }
            output
                .sync_all()
                .map_err(|e| format!("保存暂存文件失败: {e}"))?;
            self.files.push(Replacement {
                name,
                backed_up: false,
                installed: false,
            });
        }
        for required in &artifacts(core)[..2] {
            if !self.files.iter().any(|file| file.name == *required) {
                return Err(format!("安装包缺少 {required}"));
            }
        }
        Ok(())
    }

    pub fn install(&mut self) -> Result<(), String> {
        if self.state != InstallState::Prepared {
            return Err("该更新已执行过替换，请重新下载".into());
        }
        fs::create_dir_all(&self.target).map_err(|e| format!("创建核心目录失败: {e}"))?;
        for file in &self.files {
            match fs::symlink_metadata(self.target.join(file.name)) {
                Ok(metadata) if !metadata.is_file() || metadata.is_symlink() => {
                    return Err(format!("{} 不是可替换的普通文件", file.name));
                }
                Ok(_) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(format!("无法检查 {}: {error}", file.name)),
            }
        }
        for name in artifacts(self.core) {
            let backup = self.target.join(format!("{name}.old"));
            match fs::symlink_metadata(&backup) {
                Ok(metadata) if !metadata.is_file() || metadata.is_symlink() => {
                    return Err(format!("{name}.old 不是可清理的普通备份文件"));
                }
                Ok(_) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(format!("无法检查 {name}.old: {error}")),
            }
        }
        // 只有进入实际替换时才清理上次备份；下载、校验和无需更新均不触碰 .old。
        for name in artifacts(self.core) {
            match fs::remove_file(self.target.join(format!("{name}.old"))) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => {
                    return Err(format!(
                        "清理 {name}.old 失败，请关闭占用该备份的程序后重试: {error}"
                    ));
                }
            }
        }
        // 回滚失败时保留暂存内容，旧版本独立保留在核心目录的 .old 文件。
        self.directory.disable_cleanup(true);
        self.state = InstallState::Installed;
        let result = self.replace();
        if let Err(message) = result {
            return match self.rollback() {
                Ok(()) => Err(format!("{message}；已恢复到更新前的文件")),
                Err(rollback) => Err(format!("{message}；{rollback}")),
            };
        }
        Ok(())
    }

    fn replace(&mut self) -> Result<(), String> {
        for file in &mut self.files {
            let target = self.target.join(file.name);
            match fs::rename(&target, self.target.join(format!("{}.old", file.name))) {
                Ok(()) => file.backed_up = true,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => {
                    return Err(format!(
                        "备份 {} 失败，请关闭占用该文件的程序后重试: {error}",
                        file.name
                    ));
                }
            }
            fs::rename(self.directory.path().join("new").join(file.name), &target)
                .map_err(|error| format!("替换 {} 失败: {error}", file.name))?;
            file.installed = true;
        }
        Ok(())
    }

    pub fn rollback(&mut self) -> Result<(), String> {
        match self.state {
            InstallState::Prepared | InstallState::RolledBack => return Ok(()),
            InstallState::Committed => return Err("更新已提交，无法再自动回滚".into()),
            InstallState::Installed => {}
        }
        let mut failed = Vec::new();
        for file in self.files.iter_mut().rev() {
            let target = self.target.join(file.name);
            if file.installed {
                match fs::remove_file(&target) {
                    Ok(()) => file.installed = false,
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                        file.installed = false
                    }
                    Err(_) => {
                        failed.push(file.name);
                        continue;
                    }
                }
            }
            if file.backed_up {
                match fs::rename(self.target.join(format!("{}.old", file.name)), &target) {
                    Ok(()) => file.backed_up = false,
                    Err(_) => failed.push(file.name),
                }
            }
        }
        if !failed.is_empty() {
            return Err(format!(
                "无法自动恢复 {}；请关闭占用文件的程序，旧文件保留在 {} 的同名 .old 文件中",
                failed.join("、"),
                self.target.display()
            ));
        }
        self.state = InstallState::RolledBack;
        self.directory.disable_cleanup(false);
        Ok(())
    }

    pub fn commit(&mut self) -> Result<(), String> {
        if self.state != InstallState::Installed {
            return Err("更新尚未安装，无法提交".into());
        }
        // .old 留到下一次安装，提交只清理本次下载和暂存内容。
        self.state = InstallState::Committed;
        match fs::remove_dir_all(self.directory.path()) {
            Ok(()) => {
                self.directory.disable_cleanup(false);
                Ok(())
            }
            Err(_) => Err(format!(
                "核心已更新，暂存文件清理失败，可稍后删除 {}",
                self.directory.path().display()
            )),
        }
    }

    #[cfg(test)]
    pub(crate) fn for_test(
        core: Core,
        root: &Path,
        version: &str,
        files: &[(&str, &[u8])],
    ) -> Self {
        let mut prepared = Self::new(core, root, version.into()).unwrap();
        for (name, contents) in files {
            let name = artifacts(core)
                .iter()
                .copied()
                .find(|allowed| allowed == name)
                .unwrap();
            fs::write(prepared.directory.path().join("new").join(name), contents).unwrap();
            prepared.files.push(Replacement {
                name,
                backed_up: false,
                installed: false,
            });
        }
        prepared
    }
}

impl Drop for PreparedUpdate {
    fn drop(&mut self) {
        if self.state == InstallState::Installed {
            let _ = self.rollback();
        }
    }
}

fn parse_version(core: Core, output: &str) -> Option<String> {
    let first = output.lines().next()?;
    let prefix = match core {
        Core::SingBox => "sing-box version ",
        Core::Xray => "Xray ",
    };
    let version = first.strip_prefix(prefix)?.split_whitespace().next()?;
    if version.len() > 64
        || !version
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b".-+".contains(&b))
    {
        return None;
    }
    version_parts(version)?;
    Some(version.into())
}

pub fn local_version(core: Core, root: &Path) -> Result<Option<String>, String> {
    let executable = core.executable(root);
    match fs::metadata(&executable) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("无法读取核心文件: {error}")),
        Ok(metadata) if !metadata.is_file() => return Err("核心路径不是普通文件".into()),
        Ok(_) => {}
    }
    read_version(core, &executable, VERSION_TIMEOUT).map(Some)
}

fn read_version(core: Core, executable: &Path, timeout: Duration) -> Result<String, String> {
    validate_executable(executable)?;
    // 用临时文件接收输出，避免异常版本命令填满管道或后代持有管道导致等待不返回。
    let mut output = tempfile::tempfile().map_err(|e| format!("创建版本检测临时文件失败: {e}"))?;
    let child = JobProcess::spawn_version(executable, &output)
        .map_err(|_| "无法运行核心版本命令，可重新安装官方核心".to_string())?;
    let deadline = Instant::now() + timeout;
    let result = loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => break Ok(()),
            Ok(Some(_)) => break Err("核心版本命令失败，可重新安装官方核心".to_string()),
            Err(_) => break Err("无法读取核心版本命令状态".to_string()),
            Ok(None) => {}
        }
        if Instant::now() >= deadline {
            break Err("读取核心版本超时，可重新安装官方核心".into());
        }
        if output
            .metadata()
            .is_ok_and(|metadata| metadata.len() > 16 * 1024)
        {
            break Err("核心版本命令输出异常，可重新安装官方核心".into());
        }
        thread::sleep(Duration::from_millis(10));
    };
    let cleanup = child
        .terminate()
        .map_err(|_| "版本检测进程尚未完全退出，请重试".to_string());
    result?;
    cleanup?;
    drop(child);
    output
        .seek(SeekFrom::Start(0))
        .map_err(|_| "无法读取核心版本")?;
    let mut text = String::new();
    output
        .take(16 * 1024)
        .read_to_string(&mut text)
        .map_err(|_| "无法读取核心版本")?;
    parse_version(core, &text).ok_or_else(|| "无法识别核心版本，可重新安装官方核心".into())
}

fn validate_executable(executable: &Path) -> Result<(), String> {
    let invalid = || "核心不是有效的 Windows 程序，可重新安装官方核心".to_string();
    let mut file = File::open(executable).map_err(|_| invalid())?;
    let length = file.metadata().map_err(|_| invalid())?.len();
    let mut dos = [0; 64];
    file.read_exact(&mut dos).map_err(|_| invalid())?;
    let pe_offset = u64::from(u32::from_le_bytes(dos[60..64].try_into().unwrap()));
    if &dos[..2] != b"MZ" || pe_offset < 64 || pe_offset + 26 > length {
        return Err(invalid());
    }
    file.seek(SeekFrom::Start(pe_offset))
        .map_err(|_| invalid())?;
    let mut pe = [0; 26];
    file.read_exact(&mut pe).map_err(|_| invalid())?;
    let optional_size = u64::from(u16::from_le_bytes([pe[20], pe[21]]));
    let magic = u16::from_le_bytes([pe[24], pe[25]]);
    if &pe[..4] != b"PE\0\0"
        || optional_size < 64
        || pe_offset + 24 + optional_size > length
        || !matches!(magic, 0x10b | 0x20b)
    {
        return Err(invalid());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io;
    use std::net::{TcpListener, TcpStream};
    use std::os::windows::fs::OpenOptionsExt;
    use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
    use std::os::windows::process::CommandExt;
    use std::process::{Child, Command, Stdio};
    use std::sync::OnceLock;
    use windows_sys::Win32::Foundation::{WAIT_OBJECT_0, WAIT_TIMEOUT};
    use windows_sys::Win32::System::Threading::{
        CREATE_NO_WINDOW, OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject,
    };
    use zip::write::SimpleFileOptions;

    fn release(core: Core) -> serde_json::Value {
        release_version(core, "1.2.3", false)
    }

    fn release_version(core: Core, version: &str, prerelease: bool) -> serde_json::Value {
        let name = asset_name(core, version);
        serde_json::json!({
            "tag_name": format!("v{version}"), "draft": false, "prerelease": prerelease,
            "assets": [{ "name": name, "size": 123,
                "browser_download_url": format!("https://github.com/{}/releases/download/v{version}/{name}", repository(core)),
                "digest": format!("sha256:{}", "ab".repeat(32))
            }]
        })
    }

    #[test]
    fn preview_channels_select_the_newest_installable_release_for_both_cores() {
        for core in Core::ALL {
            let mut draft = release_version(core, "9.0.0", true);
            draft["draft"] = true.into();
            let mut missing_arch = release_version(core, "8.0.0", true);
            missing_arch["assets"] = serde_json::json!([]);
            let releases = serde_json::json!([
                release_version(core, "1.3.0-beta.2", true),
                draft,
                release_version(core, "1.2.3", false),
                missing_arch,
                release_version(core, "1.3.0-beta.10", true),
                release_version(core, "1.3.0-alpha.20", true)
            ]);
            let parsed = parse_release(
                core,
                UpdateChannel::Preview,
                &serde_json::to_vec(&releases).unwrap(),
            )
            .unwrap();
            assert_eq!(parsed.version, "1.3.0-beta.10");

            // Xray 预览版也可能使用纯数字标签；正式版更高时仍应选择正式版。
            for (preview, stable, expected) in [
                ("26.11.10", "26.11.9", "26.11.10"),
                ("26.9.30", "26.11.9", "26.11.9"),
            ] {
                let releases = serde_json::json!([
                    release_version(core, preview, true),
                    release_version(core, stable, false)
                ]);
                assert_eq!(
                    parse_release(
                        core,
                        UpdateChannel::Preview,
                        &serde_json::to_vec(&releases).unwrap()
                    )
                    .unwrap()
                    .version,
                    expected
                );
            }
            let mut invalid = release_version(core, "2.0.0-beta.1", true);
            invalid["assets"][0]["digest"] = serde_json::Value::Null;
            let releases = serde_json::json!([invalid, release(core)]);
            assert!(
                parse_release(
                    core,
                    UpdateChannel::Preview,
                    &serde_json::to_vec(&releases).unwrap()
                )
                .err()
                .unwrap()
                .contains("SHA-256")
            );
        }
    }

    #[test]
    fn only_official_stable_archives_with_complete_sha256_are_accepted() {
        for core in Core::ALL {
            let good = release(core);
            let parsed = parse_release(
                core,
                UpdateChannel::Stable,
                &serde_json::to_vec(&good).unwrap(),
            )
            .unwrap();
            assert_eq!(parsed.version, "1.2.3");
            assert_eq!(parsed.sha256, [0xab; 32]);
            for field in ["draft", "prerelease"] {
                let mut bad = good.clone();
                bad[field] = true.into();
                assert!(
                    parse_release(
                        core,
                        UpdateChannel::Stable,
                        &serde_json::to_vec(&bad).unwrap()
                    )
                    .is_err()
                );
            }
            for digest in [
                "sha256:abcd",
                "md5:abcd",
                "",
                &format!("sha256:{}", "zz".repeat(32)),
            ] {
                let mut bad = good.clone();
                bad["assets"][0]["digest"] = digest.into();
                assert!(
                    parse_release(
                        core,
                        UpdateChannel::Stable,
                        &serde_json::to_vec(&bad).unwrap()
                    )
                    .is_err()
                );
            }
            for url in [
                "https://example.invalid/core.zip",
                "http://github.com/core.zip",
            ] {
                let mut bad = good.clone();
                bad["assets"][0]["browser_download_url"] = url.into();
                assert!(
                    parse_release(
                        core,
                        UpdateChannel::Stable,
                        &serde_json::to_vec(&bad).unwrap()
                    )
                    .is_err()
                );
            }
            let mut bad = good.clone();
            bad["tag_name"] = "v1.2.3-beta.1".into();
            assert!(
                parse_release(
                    core,
                    UpdateChannel::Stable,
                    &serde_json::to_vec(&bad).unwrap()
                )
                .is_err()
            );
            let mut bad = good.clone();
            bad["assets"] = serde_json::json!([good["assets"][0], good["assets"][0]]);
            assert!(
                parse_release(
                    core,
                    UpdateChannel::Stable,
                    &serde_json::to_vec(&bad).unwrap()
                )
                .is_err()
            );
            let mut bad = good.clone();
            bad["assets"][0]["digest"] = serde_json::Value::Null;
            assert!(
                parse_release(
                    core,
                    UpdateChannel::Stable,
                    &serde_json::to_vec(&bad).unwrap()
                )
                .is_err()
            );
        }
    }

    #[test]
    fn version_parsing_preserves_prereleases_and_avoids_downgrades() {
        assert_eq!(
            parse_version(Core::SingBox, "sing-box version 1.14.4\nEnvironment: go"),
            Some("1.14.4".into())
        );
        assert_eq!(
            parse_version(
                Core::Xray,
                "Xray 26.11.9 (Xray, Penetrates Everything.) build"
            ),
            Some("26.11.9".into())
        );
        assert_eq!(
            parse_version(Core::SingBox, "sing-box version 1.15.0-beta.1"),
            Some("1.15.0-beta.1".into())
        );
        assert_eq!(
            parse_version(Core::Xray, "PRIVATE_CONFIG_VALUE 26.11.9"),
            None
        );
        assert!(needs_update(Some("1.2.3-beta.1"), "1.2.3"));
        assert!(needs_update(Some("1.2.9"), "1.2.10"));
        assert!(needs_update(None, "1.2.3"));
        assert!(!needs_update(Some("1.3.0-beta.1"), "1.2.3"));
        assert!(!needs_update(Some("1.2.3"), "1.2.3"));
        assert!(!needs_update(Some("1.2.3+build"), "1.2.3"));
        for (older, newer) in [
            ("1.2.3-alpha.9", "1.2.3-beta.1"),
            ("1.2.3-beta.2", "1.2.3-beta.10"),
            ("1.2.3-beta.10", "1.2.3-rc.1"),
            ("1.2.3-rc.1", "1.2.3"),
            ("1.2.3", "1.3.0-alpha.1"),
            ("26.9.30", "26.11.9"),
        ] {
            assert!(needs_update(Some(older), newer));
            assert!(!needs_update(Some(newer), older));
            assert!(!needs_update(Some(newer), newer));
        }
        assert!(!needs_update(Some("1.2.3-beta.1+build"), "1.2.3-beta.1"));
        assert!(!needs_update(Some("1.2.3.0"), "1.2.3"));
        for invalid in ["1.2.3-", "1.2.3-beta..1", "1.2.3+", "1.2.3+build+other"] {
            assert!(version_parts(invalid).is_none());
        }
    }

    fn zip_file(root: &Path, files: &[(&str, &[u8])]) -> PathBuf {
        let path = root.join("test.zip");
        let mut writer = zip::ZipWriter::new(File::create(&path).unwrap());
        for (name, contents) in files {
            writer
                .start_file(*name, SimpleFileOptions::default())
                .unwrap();
            writer.write_all(contents).unwrap();
        }
        writer.finish().unwrap();
        path
    }

    #[test]
    fn archives_are_fully_staged_without_touching_existing_resources() {
        let root = tempfile::tempdir().unwrap();
        let target = Core::SingBox.core_dir(root.path());
        fs::create_dir(&target).unwrap();
        fs::write(target.join("sing-box.exe"), b"old").unwrap();
        fs::write(target.join("cache.db"), b"cache").unwrap();
        let archive = zip_file(
            root.path(),
            &[
                ("sing-box-1.2.3-windows-amd64/sing-box.exe", b"new"),
                ("sing-box-1.2.3-windows-amd64/libcronet.dll", b"dll"),
                (
                    "sing-box-1.2.3-windows-amd64/config.json",
                    b"do not install",
                ),
            ],
        );
        let mut prepared = PreparedUpdate::new(Core::SingBox, root.path(), "1.2.3".into()).unwrap();
        prepared
            .extract(Core::SingBox, &archive, &AtomicBool::new(false))
            .unwrap();
        assert_eq!(fs::read(target.join("sing-box.exe")).unwrap(), b"old");
        assert!(!target.join("libcronet.dll").exists());
        prepared.install().unwrap();
        prepared.commit().unwrap();
        assert_eq!(fs::read(target.join("sing-box.exe")).unwrap(), b"new");
        assert_eq!(fs::read(target.join("libcronet.dll")).unwrap(), b"dll");
        assert_eq!(fs::read(target.join("cache.db")).unwrap(), b"cache");
        assert!(!target.join("config.json").exists());
        assert_eq!(fs::read(target.join("sing-box.exe.old")).unwrap(), b"old");
        assert!(!target.join("libcronet.dll.old").exists());
    }

    #[test]
    fn committed_backups_remain_until_the_next_install_and_only_known_files_are_cleaned() {
        let root = tempfile::tempdir().unwrap();
        for core in Core::ALL {
            let target = core.core_dir(root.path());
            fs::create_dir(&target).unwrap();
            for name in artifacts(core) {
                fs::write(target.join(name), b"version 1").unwrap();
            }
            fs::write(target.join("custom.old"), b"custom").unwrap();
            let files: Vec<_> = artifacts(core)
                .iter()
                .map(|&name| (name, b"version 2".as_slice()))
                .collect();
            let mut prepared = PreparedUpdate::for_test(core, root.path(), "2.0.0", &files);
            prepared.install().unwrap();
            prepared.commit().unwrap();
            drop(prepared);
            let prepared = PreparedUpdate::for_test(
                core,
                root.path(),
                "3.0.0",
                &[(artifacts(core)[0], b"version 3")],
            );
            drop(prepared);
            for name in artifacts(core) {
                assert_eq!(
                    fs::read(target.join(format!("{name}.old"))).unwrap(),
                    b"version 1"
                );
            }
            let mut prepared = PreparedUpdate::for_test(
                core,
                root.path(),
                "3.0.0",
                &[(artifacts(core)[0], b"version 3")],
            );
            prepared.install().unwrap();
            prepared.commit().unwrap();
            assert_eq!(
                fs::read(target.join(format!("{}.old", artifacts(core)[0]))).unwrap(),
                b"version 2"
            );
            for name in &artifacts(core)[1..] {
                assert!(!target.join(format!("{name}.old")).exists());
                assert_eq!(fs::read(target.join(name)).unwrap(), b"version 2");
            }
            assert_eq!(fs::read(target.join("custom.old")).unwrap(), b"custom");
        }
        assert_eq!(
            fs::read(Core::SingBox.core_dir(root.path()).join("sing-box.exe.old")).unwrap(),
            b"version 2"
        );
    }

    #[test]
    fn locked_or_non_file_old_backups_abort_before_changing_the_installed_core() {
        let root = tempfile::tempdir().unwrap();
        let target = Core::Xray.core_dir(root.path());
        fs::create_dir(&target).unwrap();
        fs::write(target.join("xray.exe"), b"current").unwrap();
        let backup = target.join("xray.exe.old");
        fs::write(&backup, b"previous").unwrap();
        let lock = fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(&backup)
            .unwrap();
        let mut prepared =
            PreparedUpdate::for_test(Core::Xray, root.path(), "2.0.0", &[("xray.exe", b"new")]);
        assert!(
            prepared
                .install()
                .unwrap_err()
                .contains("清理 xray.exe.old 失败")
        );
        assert_eq!(fs::read(target.join("xray.exe")).unwrap(), b"current");
        drop(lock);
        assert_eq!(fs::read(&backup).unwrap(), b"previous");
        fs::remove_file(&backup).unwrap();
        fs::create_dir(&backup).unwrap();
        assert!(
            prepared
                .install()
                .unwrap_err()
                .contains("不是可清理的普通备份文件")
        );
        assert_eq!(fs::read(target.join("xray.exe")).unwrap(), b"current");
    }

    #[test]
    fn missing_libraries_unsafe_paths_and_duplicate_artifacts_abort_before_replacement() {
        let root = tempfile::tempdir().unwrap();
        let cases: &[&[(&str, &[u8])]] = &[
            &[("sing-box.exe", b"exe")],
            &[
                ("sing-box.exe", b"exe"),
                ("libcronet.dll", b"dll"),
                ("../outside.txt", b"outside"),
            ],
            &[
                ("sing-box.exe", b"exe"),
                ("libcronet.dll", b"dll"),
                ("a/../outside.txt", b"outside"),
            ],
            &[
                ("sing-box.exe", b"exe"),
                ("libcronet.dll", b"dll"),
                ("a/SING-BOX.EXE", b"duplicate"),
            ],
            &[("sing-box.exe", b""), ("libcronet.dll", b"dll")],
        ];
        for files in cases {
            let archive = zip_file(root.path(), files);
            let mut prepared =
                PreparedUpdate::new(Core::SingBox, root.path(), "1.2.3".into()).unwrap();
            assert!(
                prepared
                    .extract(Core::SingBox, &archive, &AtomicBool::new(false))
                    .is_err()
            );
            assert!(!Core::SingBox.core_dir(root.path()).exists());
        }
    }

    #[test]
    fn a_locked_later_artifact_rolls_back_replacements_and_removes_new_files() {
        let root = tempfile::tempdir().unwrap();
        let target = Core::Xray.core_dir(root.path());
        fs::create_dir(&target).unwrap();
        fs::write(target.join("xray.exe"), b"old exe").unwrap();
        fs::write(target.join("geosite.dat"), b"old geosite").unwrap();
        let lock = fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(target.join("geosite.dat"))
            .unwrap();
        let mut prepared = PreparedUpdate::for_test(
            Core::Xray,
            root.path(),
            "1.2.3",
            &[
                ("xray.exe", b"new exe"),
                ("geoip.dat", b"new geoip"),
                ("geosite.dat", b"new geosite"),
            ],
        );
        assert!(prepared.install().unwrap_err().contains("已恢复到更新前"));
        assert_eq!(fs::read(target.join("xray.exe")).unwrap(), b"old exe");
        assert!(!target.join("geoip.dat").exists());
        drop(lock);
        assert_eq!(
            fs::read(target.join("geosite.dat")).unwrap(),
            b"old geosite"
        );
    }

    #[test]
    fn rollback_failure_retains_backup_and_can_be_retried_after_unlocking() {
        let root = tempfile::tempdir().unwrap();
        let target = Core::Xray.core_dir(root.path());
        fs::create_dir(&target).unwrap();
        fs::write(target.join("xray.exe"), b"old").unwrap();
        let mut prepared =
            PreparedUpdate::for_test(Core::Xray, root.path(), "1.2.3", &[("xray.exe", b"new")]);
        prepared.install().unwrap();
        let backup = target.join("xray.exe.old");
        let lock = fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(target.join("xray.exe"))
            .unwrap();
        assert!(prepared.rollback().unwrap_err().contains("旧文件保留在"));
        assert_eq!(fs::read(&backup).unwrap(), b"old");
        drop(lock);
        prepared.rollback().unwrap();
        assert_eq!(fs::read(target.join("xray.exe")).unwrap(), b"old");
        assert!(!backup.exists());
    }

    #[test]
    fn dropping_an_uncommitted_update_restores_old_files_and_preserves_blocked_backups() {
        let root = tempfile::tempdir().unwrap();
        let target = Core::Xray.core_dir(root.path());
        fs::create_dir(&target).unwrap();
        fs::write(target.join("xray.exe"), b"old").unwrap();
        let mut prepared =
            PreparedUpdate::for_test(Core::Xray, root.path(), "1.2.3", &[("xray.exe", b"new")]);
        prepared.install().unwrap();
        drop(prepared);
        assert_eq!(fs::read(target.join("xray.exe")).unwrap(), b"old");
        let mut prepared =
            PreparedUpdate::for_test(Core::Xray, root.path(), "1.2.3", &[("xray.exe", b"new")]);
        prepared.install().unwrap();
        let backup = target.join("xray.exe.old");
        let lock = fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(target.join("xray.exe"))
            .unwrap();
        drop(prepared);
        assert_eq!(fs::read(backup).unwrap(), b"old");
        drop(lock);
    }

    fn server(replies: Vec<Vec<u8>>) -> (String, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let worker = thread::spawn(move || {
            for response in replies {
                let deadline = Instant::now() + Duration::from_secs(5);
                let mut stream = loop {
                    match listener.accept() {
                        Ok((stream, _)) => break stream,
                        Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                            assert!(Instant::now() < deadline, "test client never connected");
                            thread::sleep(Duration::from_millis(5));
                        }
                        Err(error) => panic!("{error}"),
                    }
                };
                stream.set_nonblocking(false).unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                read_request(&mut stream);
                stream.write_all(&response).unwrap();
            }
        });
        (url, worker)
    }

    fn read_request(stream: &mut TcpStream) {
        let mut bytes = Vec::new();
        loop {
            let mut byte = [0];
            assert_eq!(stream.read(&mut byte).unwrap(), 1);
            bytes.push(byte[0]);
            if bytes.ends_with(b"\r\n\r\n") {
                break;
            }
            assert!(bytes.len() < 8192);
        }
    }

    fn download_for(data: &[u8]) -> Download {
        Download {
            version: "1.2.3".into(),
            url: String::new(),
            size: data.len() as u64,
            sha256: Sha256::digest(data).into(),
        }
    }

    #[test]
    fn interrupted_and_invalid_downloads_can_retry_without_changing_installed_core() {
        let body = b"verified archive";
        let good = [
            format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            )
            .as_bytes(),
            body,
        ]
        .concat();
        let truncated =
            b"HTTP/1.1 200 OK\r\nContent-Length: 16\r\nConnection: close\r\n\r\nshort".to_vec();
        let wrong = good
            .iter()
            .copied()
            .take(good.len() - 1)
            .chain(*b"!")
            .collect();
        let (url, server) = server(vec![truncated, wrong, good]);
        let root = tempfile::tempdir().unwrap();
        let target = Core::Xray.core_dir(root.path());
        fs::create_dir(&target).unwrap();
        fs::write(target.join("xray.exe"), b"old").unwrap();
        let path = root.path().join("download.zip");
        let cancelled = AtomicBool::new(false);
        let download = download_for(body);
        let mut progress = Vec::new();
        for success in [false, false, true] {
            let result = download_archive(
                ureq::get(&url),
                &download,
                &path,
                &cancelled,
                &mut |update| progress.push(update),
                Duration::from_secs(2),
            );
            assert_eq!(result.is_ok(), success, "{result:?}");
            assert_eq!(fs::read(target.join("xray.exe")).unwrap(), b"old");
        }
        server.join().unwrap();
        assert_eq!(fs::read(path).unwrap(), body);
        assert!(progress.contains(&UpdateProgress::Downloading {
            received: body.len() as u64,
            total: body.len() as u64
        }));
        assert_eq!(progress.last(), Some(&UpdateProgress::Verifying));
    }

    #[test]
    fn downloads_follow_redirects_and_report_http_errors_without_response_content() {
        let body = b"zip";
        let (url, server) = server(vec![
            b"HTTP/1.1 302 Found\r\nLocation: /archive\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec(),
            b"HTTP/1.1 200 OK\r\nContent-Length: 3\r\nConnection: close\r\n\r\nzip".to_vec(),
            b"HTTP/1.1 403 Forbidden\r\nContent-Length: 7\r\nConnection: close\r\n\r\nPRIVATE".to_vec(),
        ]);
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("download.zip");
        let download = download_for(body);
        let cancelled = AtomicBool::new(false);
        download_archive(
            ureq::get(&url),
            &download,
            &path,
            &cancelled,
            &mut |_| {},
            Duration::from_secs(2),
        )
        .unwrap();
        let error = download_archive(
            ureq::get(&url),
            &download,
            &path,
            &cancelled,
            &mut |_| {},
            Duration::from_secs(2),
        )
        .unwrap_err();
        assert!(error.contains("限制请求"));
        assert!(!error.contains("PRIVATE"));
        server.join().unwrap();
    }

    #[test]
    fn stalled_downloads_time_out_and_cancellation_stops_before_network_io() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            read_request(&mut stream);
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 10\r\n\r\na")
                .unwrap();
            thread::sleep(Duration::from_millis(350));
        });
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("download.zip");
        let download = download_for(b"0123456789");
        let error = download_archive(
            ureq::get(&url),
            &download,
            &path,
            &AtomicBool::new(false),
            &mut |_| {},
            Duration::from_millis(80),
        )
        .unwrap_err();
        assert!(error.contains("超时"));
        server.join().unwrap();
        let start = Instant::now();
        let error = download_archive(
            ureq::get(&url),
            &download,
            &path,
            &AtomicBool::new(true),
            &mut |_| {},
            Duration::from_secs(2),
        )
        .unwrap_err();
        assert_eq!(error, "已取消更新");
        assert!(start.elapsed() < Duration::from_secs(1));
    }

    fn version_fixture() -> &'static Path {
        static FIXTURE: OnceLock<(TempDir, PathBuf)> = OnceLock::new();
        &FIXTURE
            .get_or_init(|| {
                let dir = tempfile::tempdir().unwrap();
                let source = dir.path().join("version.rs");
                fs::write(
                    &source,
                    r#"
fn main() {
    if std::env::args().nth(1).as_deref() == Some("--child") {
        std::thread::sleep(std::time::Duration::from_secs(30));
        return;
    }
    assert_eq!(std::env::args().nth(1).as_deref(), Some("version"));
    std::fs::write("version.pid", std::process::id().to_string()).unwrap();
    if std::path::Path::new("tree").exists() {
        let child = std::process::Command::new(std::env::current_exe().unwrap()).arg("--child").spawn().unwrap();
        std::fs::write("child.pid", child.id().to_string()).unwrap();
    }
    if std::path::Path::new("sleep").exists() {
        std::thread::sleep(std::time::Duration::from_secs(10));
    }
    if std::path::Path::new("failure").exists() {
        eprintln!("PRIVATE_CORE_OUTPUT");
        std::process::exit(23);
    }
    println!("sing-box version 1.2.3-beta.1");
}
"#,
                )
                .unwrap();
                let exe = dir.path().join("fixture.exe");
                let status = Command::new("rustc")
                    .arg(&source)
                    .arg("-o")
                    .arg(&exe)
                    .creation_flags(CREATE_NO_WINDOW)
                    .status()
                    .unwrap();
                assert!(status.success());
                (dir, exe)
            })
            .1
    }

    #[test]
    fn version_commands_have_a_deadline_and_never_expose_raw_output() {
        let root = tempfile::tempdir().unwrap();
        assert_eq!(local_version(Core::SingBox, root.path()).unwrap(), None);
        let target = Core::SingBox.core_dir(root.path());
        fs::create_dir(&target).unwrap();
        let exe = Core::SingBox.executable(root.path());
        fs::copy(version_fixture(), &exe).unwrap();
        assert_eq!(
            local_version(Core::SingBox, root.path()).unwrap(),
            Some("1.2.3-beta.1".into())
        );
        fs::write(target.join("failure"), b"").unwrap();
        let error = local_version(Core::SingBox, root.path()).unwrap_err();
        assert!(!error.contains("PRIVATE_CORE_OUTPUT"));
        fs::remove_file(target.join("failure")).unwrap();
        fs::write(target.join("sleep"), b"").unwrap();
        let start = Instant::now();
        assert!(
            read_version(Core::SingBox, &exe, Duration::from_millis(80))
                .unwrap_err()
                .contains("超时")
        );
        assert!(start.elapsed() < Duration::from_secs(2));
        fs::remove_file(exe).expect("timed out process must have exited");
    }

    #[test]
    fn malformed_executables_fail_before_entering_the_windows_loader() {
        let root = tempfile::tempdir().unwrap();
        let executable = root.path().join("invalid.exe");
        let mut offset_outside_file = vec![0; 64];
        offset_outside_file[..2].copy_from_slice(b"MZ");
        offset_outside_file[60..64].copy_from_slice(&u32::MAX.to_le_bytes());
        for bytes in [
            b"invalid replacement".to_vec(),
            b"MZ".to_vec(),
            offset_outside_file,
        ] {
            fs::write(&executable, bytes).unwrap();
            let start = Instant::now();
            let error = read_version(Core::SingBox, &executable, VERSION_TIMEOUT).unwrap_err();
            assert!(error.contains("不是有效的 Windows 程序"));
            assert!(start.elapsed() < Duration::from_secs(1));
        }
    }

    fn probe_handle(path: &Path) -> OwnedHandle {
        let deadline = Instant::now() + Duration::from_secs(5);
        let pid = loop {
            if let Ok(text) = fs::read_to_string(path)
                && let Ok(pid) = text.parse::<u32>()
            {
                break pid;
            }
            assert!(
                Instant::now() < deadline,
                "version fixture never wrote its PID"
            );
            thread::sleep(Duration::from_millis(10));
        };
        let handle = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, pid) };
        assert!(!handle.is_null(), "version process must still be running");
        unsafe { OwnedHandle::from_raw_handle(handle) }
    }

    struct ProbeHost(Child);

    impl Drop for ProbeHost {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    #[test]
    fn version_process_trees_end_when_their_host_is_forced_to_exit() {
        if let Some(root) = std::env::var_os("LADDER_VERSION_PROBE_TEST") {
            let executable = Core::SingBox.executable(Path::new(&root));
            let _ = read_version(Core::SingBox, &executable, Duration::from_secs(30));
            return;
        }
        let root = tempfile::tempdir().unwrap();
        let target = Core::SingBox.core_dir(root.path());
        fs::create_dir(&target).unwrap();
        fs::copy(version_fixture(), Core::SingBox.executable(root.path())).unwrap();
        fs::write(target.join("sleep"), b"").unwrap();
        fs::write(target.join("tree"), b"").unwrap();
        let mut host = ProbeHost(
            Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "update::tests::version_process_trees_end_when_their_host_is_forced_to_exit",
                ])
                .env("LADDER_VERSION_PROBE_TEST", root.path())
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .creation_flags(CREATE_NO_WINDOW)
                .spawn()
                .unwrap(),
        );
        let version = probe_handle(&target.join("version.pid"));
        let descendant = probe_handle(&target.join("child.pid"));
        for process in [&version, &descendant] {
            assert_eq!(
                unsafe { WaitForSingleObject(process.as_raw_handle(), 0) },
                WAIT_TIMEOUT
            );
        }
        host.0.kill().unwrap();
        host.0.wait().unwrap();
        for process in [&version, &descendant] {
            assert_eq!(
                unsafe { WaitForSingleObject(process.as_raw_handle(), 3000) },
                WAIT_OBJECT_0
            );
        }
    }

    #[test]
    fn successful_version_commands_also_clean_up_their_descendants() {
        let root = tempfile::tempdir().unwrap();
        let target = Core::SingBox.core_dir(root.path());
        fs::create_dir(&target).unwrap();
        let executable = Core::SingBox.executable(root.path());
        fs::copy(version_fixture(), &executable).unwrap();
        fs::write(target.join("tree"), b"").unwrap();
        assert_eq!(
            read_version(Core::SingBox, &executable, VERSION_TIMEOUT).unwrap(),
            "1.2.3-beta.1"
        );
        let pid = fs::read_to_string(target.join("child.pid"))
            .unwrap()
            .parse()
            .unwrap();
        let handle = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, pid) };
        if !handle.is_null() {
            let handle = unsafe { OwnedHandle::from_raw_handle(handle) };
            assert_eq!(
                unsafe { WaitForSingleObject(handle.as_raw_handle(), 3000) },
                WAIT_OBJECT_0
            );
        }
        fs::remove_file(executable).expect("the fixture executable must no longer be mapped");
    }

    #[test]
    #[ignore = "downloads current official releases; run explicitly for release acceptance"]
    fn official_stable_install() {
        official_install(UpdateChannel::Stable);
    }

    #[test]
    #[ignore = "downloads current official releases; run explicitly for release acceptance"]
    fn official_preview_install() {
        official_install(UpdateChannel::Preview);
    }

    fn official_install(channel: UpdateChannel) {
        let root = tempfile::tempdir().unwrap();
        for core in Core::ALL {
            let cancelled = AtomicBool::new(false);
            let mut prepared = prepare(core, channel, root.path(), None, &cancelled, |_| {})
                .unwrap_or_else(|error| panic!("{}: {error}", core.label()))
                .expect("a clean directory requires installation");
            let version = prepared.version.clone();
            assert!(!core.executable(root.path()).exists());
            prepared.install().unwrap();
            prepared.commit().unwrap();
            assert_eq!(
                local_version(core, root.path()).unwrap(),
                Some(version.clone())
            );
            assert!(
                prepare(
                    core,
                    channel,
                    root.path(),
                    Some(&version),
                    &cancelled,
                    |_| {}
                )
                .unwrap()
                .is_none()
            );
            eprintln!(
                "{} {channel:?} {version}: official installation, version probe and repeat check passed",
                core.label()
            );
        }
    }
}
