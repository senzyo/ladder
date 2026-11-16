use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::windows::fs::MetadataExt;
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
pub struct CoreVersion {
    pub version: String,
    pub channel: UpdateChannel,
    pub installed: bool,
    pub release_tag: Option<String>,
}

pub struct ReleasePage {
    pub versions: Vec<CoreVersion>,
    pub has_more: bool,
}

#[derive(Deserialize, Serialize)]
struct VersionRecord {
    channel: UpdateChannel,
    release_tag: Option<String>,
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
    channel: UpdateChannel,
    tag: String,
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

pub(crate) fn compare_versions(left: &str, right: &str) -> Option<std::cmp::Ordering> {
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

fn parse_release(core: Core, data: &[u8]) -> Result<Download, String> {
    let release: Release =
        serde_json::from_slice(data).map_err(|_| "官方发布信息格式无效，请稍后重试".to_string())?;
    release_download(core, release)
}

fn release_download(core: Core, release: Release) -> Result<Download, String> {
    let version = release
        .tag_name
        .strip_prefix('v')
        .unwrap_or(&release.tag_name);
    let Some((_, prerelease)) = version_parts(version) else {
        return Err("官方响应不是可安装的版本".into());
    };
    if release.draft {
        return Err("官方响应不是已发布的可安装版本".into());
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
        channel: if release.prerelease || prerelease.is_some() {
            UpdateChannel::Preview
        } else {
            UpdateChannel::Stable
        },
        tag: release.tag_name,
        url: asset.url,
        size: asset.size,
        sha256,
    })
}

fn parse_releases(core: Core, data: &[u8]) -> Result<ReleasePage, String> {
    let releases: Vec<Release> =
        serde_json::from_slice(data).map_err(|_| "官方发布列表格式无效，请稍后重试".to_string())?;
    let has_more = releases.len() == 30;
    let mut versions = Vec::new();
    for release in releases {
        let version = release
            .tag_name
            .strip_prefix('v')
            .unwrap_or(&release.tag_name);
        if release.draft
            || version_parts(version).is_none()
            || !release
                .assets
                .iter()
                .any(|asset| asset.name == asset_name(core, version))
        {
            continue;
        }
        if versions
            .iter()
            .any(|item: &CoreVersion| item.version == version)
        {
            continue;
        }
        versions.push(CoreVersion {
            version: version.into(),
            channel: if release.prerelease || version_parts(version).unwrap().1.is_some() {
                UpdateChannel::Preview
            } else {
                UpdateChannel::Stable
            },
            installed: false,
            release_tag: Some(release.tag_name),
        });
    }
    versions.sort_by(|left, right| compare_versions(&right.version, &left.version).unwrap());
    Ok(ReleasePage { versions, has_more })
}

fn release_data(core: Core, endpoint: &str, cancelled: &AtomicBool) -> Result<Vec<u8>, String> {
    check_cancelled(cancelled)?;
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
    Ok(data)
}

pub fn fetch_releases(
    core: Core,
    page: u32,
    cancelled: &AtomicBool,
) -> Result<ReleasePage, String> {
    if page == 0 {
        return Err("版本列表页码无效".into());
    }
    let data = release_data(
        core,
        &format!("releases?per_page=30&page={page}"),
        cancelled,
    )?;
    parse_releases(core, &data)
}

pub fn version_directory(core: Core, root: &Path, version: &str) -> Result<PathBuf, String> {
    if version_parts(version).is_none() {
        return Err("核心版本号无效".into());
    }
    let directory = versions_directory(core, root)?.join(version);
    check_directory(&directory)?;
    Ok(directory)
}

fn versions_directory(core: Core, root: &Path) -> Result<PathBuf, String> {
    let core = core.core_dir(root);
    check_directory(&core)?;
    let directory = core.join("versions");
    check_directory(&directory)?;
    Ok(directory)
}

fn check_directory(directory: &Path) -> Result<(), String> {
    match fs::symlink_metadata(directory) {
        Ok(metadata) if !metadata.is_dir() || is_reparse_point(&metadata) => {
            Err("核心版本路径不是普通目录，无法执行操作".into())
        }
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("无法检查核心版本目录: {error}")),
    }
}

fn is_reparse_point(metadata: &fs::Metadata) -> bool {
    metadata.file_attributes() & 0x400 != 0
}

pub fn installed_versions(core: Core, root: &Path) -> Result<Vec<CoreVersion>, String> {
    let directory = versions_directory(core, root)?;
    let entries = match fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(format!("无法读取本地核心版本: {error}")),
    };
    let mut versions = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|error| format!("无法读取本地核心版本: {error}"))?;
        let Some(version) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        if version_parts(&version).is_none() {
            continue;
        }
        if entry
            .file_type()
            .map_err(|error| format!("无法检查本地核心版本: {error}"))?
            .is_file()
        {
            continue;
        }
        let path = version_directory(core, root, &version)?;
        let record = match fs::read(path.join(".ladder-version.json")) {
            Ok(data) => serde_json::from_slice::<VersionRecord>(&data).ok(),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(format!("无法读取核心版本记录: {error}")),
        };
        let (channel, release_tag) = match record {
            Some(record) => (record.channel, record.release_tag),
            None => (
                if version_parts(&version).unwrap().1.is_some() {
                    UpdateChannel::Preview
                } else {
                    UpdateChannel::Stable
                },
                None,
            ),
        };
        versions.push(CoreVersion {
            version,
            channel,
            installed: true,
            release_tag,
        });
    }
    versions.sort_by(|left, right| compare_versions(&right.version, &left.version).unwrap());
    Ok(versions)
}

pub fn delete_version(core: Core, root: &Path, version: &str) -> Result<(), String> {
    let directory = version_directory(core, root, version)?;
    let parent = fs::canonicalize(directory.parent().unwrap())
        .map_err(|error| format!("无法定位核心版本目录: {error}"))?;
    let target =
        fs::canonicalize(&directory).map_err(|error| format!("无法定位待删除版本: {error}"))?;
    if target.parent() != Some(parent.as_path()) {
        return Err("待删除版本不在核心版本目录内".into());
    }
    fs::remove_dir_all(&target)
        .map_err(|error| format!("删除核心 {version} 失败，请关闭占用该版本的程序后重试: {error}"))
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

pub fn prepare_version(
    core: Core,
    root: &Path,
    release: &CoreVersion,
    cancelled: &AtomicBool,
    mut progress: impl FnMut(UpdateProgress),
) -> Result<PreparedUpdate, String> {
    check_cancelled(cancelled)?;
    let target = version_directory(core, root, &release.version)?;
    if target.exists() {
        return Err("该版本已存在于本地，请先删除后再下载".into());
    }
    let tag = release
        .release_tag
        .as_deref()
        .ok_or("请先刷新官方版本列表")?;
    if tag.strip_prefix('v').unwrap_or(tag) != release.version {
        return Err("官方版本标签与版本号不符，请刷新列表".into());
    }
    progress(UpdateProgress::Checking);
    let data = release_data(core, &format!("releases/tags/{tag}"), cancelled)?;
    let download = parse_release(core, &data)?;
    if download.version != release.version || download.tag != tag {
        return Err("官方发布响应与所选版本不符，请刷新列表".into());
    }
    let mut prepared = PreparedUpdate::new(core, root, download.version.clone())?;
    prepared.channel = download.channel;
    prepared.release_tag = Some(download.tag.clone());
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
    Ok(prepared)
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

pub struct PreparedUpdate {
    pub version: String,
    pub channel: UpdateChannel,
    release_tag: Option<String>,
    core: Core,
    root: PathBuf,
    directory: TempDir,
    files: Vec<&'static str>,
    installed: bool,
}

impl PreparedUpdate {
    fn new(core: Core, root: &Path, version: String) -> Result<Self, String> {
        version_directory(core, root, &version)?;
        let directory = tempfile::Builder::new()
            .prefix(&format!(".ladder-download-{}-", core.label()))
            .tempdir_in(root)
            .map_err(|e| format!("创建下载暂存目录失败: {e}"))?;
        fs::create_dir(directory.path().join("new"))
            .map_err(|e| format!("创建下载暂存目录失败: {e}"))?;
        let channel = if version_parts(&version).unwrap().1.is_some() {
            UpdateChannel::Preview
        } else {
            UpdateChannel::Stable
        };
        Ok(Self {
            version,
            channel,
            release_tag: None,
            core,
            root: root.into(),
            directory,
            files: Vec::new(),
            installed: false,
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
            if self.files.contains(&name) {
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
            self.files.push(name);
        }
        let executable = artifacts(core)[0];
        if !self.files.contains(&executable) {
            return Err(format!("安装包缺少 {executable}"));
        }
        Ok(())
    }

    pub fn install(&mut self) -> Result<(), String> {
        if self.installed {
            return Err("该版本已完成下载".into());
        }
        let target = version_directory(self.core, &self.root, &self.version)?;
        if target.exists() {
            return Err("该版本已存在于本地，请先删除后再下载".into());
        }
        let staged = self.directory.path().join("new");
        let record = VersionRecord {
            channel: self.channel,
            release_tag: self.release_tag.clone(),
        };
        let mut file = File::create(staged.join(".ladder-version.json"))
            .map_err(|error| format!("创建版本记录失败: {error}"))?;
        serde_json::to_writer(&mut file, &record)
            .map_err(|error| format!("保存版本记录失败: {error}"))?;
        file.sync_all()
            .map_err(|error| format!("保存版本记录失败: {error}"))?;
        drop(file);
        fs::create_dir_all(target.parent().unwrap())
            .map_err(|error| format!("创建核心版本目录失败: {error}"))?;
        version_directory(self.core, &self.root, &self.version)?;
        fs::rename(&staged, &target)
            .map_err(|error| format!("保存核心版本失败，现有版本未修改: {error}"))?;
        self.installed = true;
        Ok(())
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
            prepared.files.push(name);
        }
        prepared
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

pub fn read_version_for(core: Core, executable: &Path) -> Result<String, String> {
    read_version(core, executable, VERSION_TIMEOUT)
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
    fn release_lists_preserve_stable_and_preview_versions_and_sort_numerically() {
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
                release_version(core, "1.3.0-alpha.20", false),
                release_version(core, "26.11.10", true),
                release_version(core, "26.11.9", false),
                release_version(core, "1.2.3", false)
            ]);
            let parsed = parse_releases(core, &serde_json::to_vec(&releases).unwrap()).unwrap();
            assert!(!parsed.has_more);
            let versions: Vec<_> = parsed
                .versions
                .iter()
                .map(|item| item.version.as_str())
                .collect();
            assert_eq!(
                versions,
                [
                    "26.11.10",
                    "26.11.9",
                    "1.3.0-beta.10",
                    "1.3.0-beta.2",
                    "1.3.0-alpha.20",
                    "1.2.3"
                ]
            );
            assert_eq!(parsed.versions[0].channel, UpdateChannel::Preview);
            assert_eq!(parsed.versions[1].channel, UpdateChannel::Stable);
            assert_eq!(parsed.versions[4].channel, UpdateChannel::Preview);
            assert!(
                parsed
                    .versions
                    .iter()
                    .all(|item| !item.installed && item.release_tag.is_some())
            );
            let mut unavailable = release(core);
            unavailable["assets"][0]["digest"] = serde_json::Value::Null;
            let page = vec![unavailable; 30];
            let parsed = parse_releases(core, &serde_json::to_vec(&page).unwrap()).unwrap();
            assert!(parsed.has_more);
            assert_eq!(parsed.versions.len(), 1);
        }
    }

    #[test]
    fn downloads_require_official_assets_and_complete_sha256() {
        for core in Core::ALL {
            let good = release(core);
            let parsed = parse_release(core, &serde_json::to_vec(&good).unwrap()).unwrap();
            assert_eq!(parsed.version, "1.2.3");
            assert_eq!(parsed.sha256, [0xab; 32]);
            let mut bad = good.clone();
            bad["draft"] = true.into();
            assert!(parse_release(core, &serde_json::to_vec(&bad).unwrap()).is_err());
            for digest in [
                "sha256:abcd",
                "md5:abcd",
                "",
                &format!("sha256:{}", "zz".repeat(32)),
            ] {
                let mut bad = good.clone();
                bad["assets"][0]["digest"] = digest.into();
                assert!(parse_release(core, &serde_json::to_vec(&bad).unwrap()).is_err());
            }
            for url in [
                "https://example.invalid/core.zip",
                "http://github.com/core.zip",
            ] {
                let mut bad = good.clone();
                bad["assets"][0]["browser_download_url"] = url.into();
                assert!(parse_release(core, &serde_json::to_vec(&bad).unwrap()).is_err());
            }
            let mut bad = good.clone();
            bad["assets"] = serde_json::json!([good["assets"][0], good["assets"][0]]);
            assert!(parse_release(core, &serde_json::to_vec(&bad).unwrap()).is_err());
            let mut bad = good.clone();
            bad["assets"][0]["digest"] = serde_json::Value::Null;
            assert!(parse_release(core, &serde_json::to_vec(&bad).unwrap()).is_err());
            for (version, prerelease) in [("1.3.0-beta.1", false), ("26.11.10", true)] {
                let data = serde_json::to_vec(&release_version(core, version, prerelease)).unwrap();
                assert_eq!(
                    parse_release(core, &data).unwrap().channel,
                    UpdateChannel::Preview
                );
            }
        }
    }

    #[test]
    fn version_parsing_and_ordering_preserve_prereleases() {
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
        for (older, newer) in [
            ("1.2.3-alpha.9", "1.2.3-beta.1"),
            ("1.2.3-beta.2", "1.2.3-beta.10"),
            ("1.2.3-beta.10", "1.2.3-rc.1"),
            ("1.2.3-rc.1", "1.2.3"),
            ("1.2.3", "1.3.0-alpha.1"),
            ("26.9.30", "26.11.9"),
        ] {
            assert_eq!(
                compare_versions(older, newer),
                Some(std::cmp::Ordering::Less)
            );
            assert_eq!(
                compare_versions(newer, older),
                Some(std::cmp::Ordering::Greater)
            );
        }
        assert_eq!(
            compare_versions("1.2.3-beta.1+build", "1.2.3-beta.1"),
            Some(std::cmp::Ordering::Equal)
        );
        assert_eq!(
            compare_versions("1.2.3.0", "1.2.3"),
            Some(std::cmp::Ordering::Equal)
        );
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
    fn archives_are_staged_then_committed_as_separate_versions() {
        let root = tempfile::tempdir().unwrap();
        let core = Core::SingBox;
        let first = version_directory(core, root.path(), "1.0.0").unwrap();
        fs::create_dir_all(&first).unwrap();
        fs::write(first.join("sing-box.exe"), b"old").unwrap();
        let configs = root.path().join("configs");
        fs::create_dir(&configs).unwrap();
        fs::write(configs.join("user.json"), b"private config").unwrap();
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
        let target = version_directory(core, root.path(), "1.2.3").unwrap();
        let mut prepared = PreparedUpdate::new(core, root.path(), "1.2.3".into()).unwrap();
        prepared
            .extract(core, &archive, &AtomicBool::new(false))
            .unwrap();
        assert!(!target.exists());
        prepared.install().unwrap();
        drop(prepared);
        assert_eq!(fs::read(target.join("sing-box.exe")).unwrap(), b"new");
        assert_eq!(fs::read(target.join("libcronet.dll")).unwrap(), b"dll");
        assert!(!target.join("config.json").exists());
        assert_eq!(fs::read(first.join("sing-box.exe")).unwrap(), b"old");
        assert_eq!(
            fs::read(configs.join("user.json")).unwrap(),
            b"private config"
        );
        assert_eq!(installed_versions(core, root.path()).unwrap().len(), 2);
    }

    #[test]
    fn versions_coexist_and_deletion_preserves_other_versions_and_cores() {
        let root = tempfile::tempdir().unwrap();
        for core in Core::ALL {
            for version in ["1.0.0", "2.0.0", "3.0.0-beta.1"] {
                let mut prepared = PreparedUpdate::for_test(
                    core,
                    root.path(),
                    version,
                    &[(artifacts(core)[0], version.as_bytes())],
                );
                if version == "2.0.0" {
                    prepared.channel = UpdateChannel::Preview;
                    prepared.release_tag = Some("v2.0.0".into());
                }
                prepared.install().unwrap();
            }
            let versions = installed_versions(core, root.path()).unwrap();
            assert_eq!(versions.len(), 3);
            assert_eq!(versions[0].version, "3.0.0-beta.1");
            assert_eq!(versions[1].channel, UpdateChannel::Preview);
            assert_eq!(versions[1].release_tag.as_deref(), Some("v2.0.0"));
        }
        delete_version(Core::SingBox, root.path(), "2.0.0").unwrap();
        let versions = installed_versions(Core::SingBox, root.path()).unwrap();
        assert_eq!(versions.len(), 2);
        assert!(versions.iter().all(|item| item.version != "2.0.0"));
        assert_eq!(
            installed_versions(Core::Xray, root.path()).unwrap().len(),
            3
        );
        assert_eq!(
            fs::read(
                version_directory(Core::SingBox, root.path(), "1.0.0")
                    .unwrap()
                    .join("sing-box.exe")
            )
            .unwrap(),
            b"1.0.0"
        );
    }

    #[test]
    fn duplicate_installs_and_abandoned_downloads_never_replace_existing_files() {
        let root = tempfile::tempdir().unwrap();
        let core = Core::Xray;
        let mut first =
            PreparedUpdate::for_test(core, root.path(), "1.0.0", &[("xray.exe", b"original")]);
        first.install().unwrap();
        assert!(first.install().is_err());
        let mut duplicate =
            PreparedUpdate::for_test(core, root.path(), "1.0.0", &[("xray.exe", b"replacement")]);
        assert!(duplicate.install().unwrap_err().contains("已存在"));
        let abandoned =
            PreparedUpdate::for_test(core, root.path(), "2.0.0", &[("xray.exe", b"abandoned")]);
        let staging = abandoned.directory.path().to_owned();
        drop(abandoned);
        assert!(!staging.exists());
        assert!(
            !version_directory(core, root.path(), "2.0.0")
                .unwrap()
                .exists()
        );
        assert_eq!(
            fs::read(
                version_directory(core, root.path(), "1.0.0")
                    .unwrap()
                    .join("xray.exe")
            )
            .unwrap(),
            b"original"
        );
    }

    #[test]
    fn archives_without_optional_libraries_and_damaged_metadata_remain_manageable() {
        let root = tempfile::tempdir().unwrap();
        for core in Core::ALL {
            let archive = zip_file(
                root.path(),
                &[(artifacts(core)[0], b"historical executable")],
            );
            let mut prepared = PreparedUpdate::new(core, root.path(), "1.0.0".into()).unwrap();
            prepared
                .extract(core, &archive, &AtomicBool::new(false))
                .unwrap();
            prepared.install().unwrap();
            let target = version_directory(core, root.path(), "1.0.0").unwrap();
            fs::write(target.join(".ladder-version.json"), b"broken json").unwrap();
            let versions = installed_versions(core, root.path()).unwrap();
            assert_eq!(versions.len(), 1);
            assert!(versions[0].installed);
            assert_eq!(versions[0].channel, UpdateChannel::Stable);
            assert_eq!(versions[0].release_tag, None);
            fs::remove_file(target.join(artifacts(core)[0])).unwrap();
            let incomplete = installed_versions(core, root.path()).unwrap();
            assert_eq!(incomplete.len(), 1);
            assert!(incomplete[0].installed);
            delete_version(core, root.path(), "1.0.0").unwrap();
            assert!(installed_versions(core, root.path()).unwrap().is_empty());
        }
    }

    #[test]
    fn version_paths_reject_traversal_and_junctions_before_deletion() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("keep.txt"), b"keep").unwrap();
        for version in [
            "..",
            "../1.0.0",
            "1.0.0/../2.0.0",
            "1.0.0:stream",
            "C:\\outside",
            "1.0.0 ",
        ] {
            assert!(version_directory(Core::Xray, root.path(), version).is_err());
            assert!(delete_version(Core::Xray, root.path(), version).is_err());
        }
        for depth in 0..3 {
            let root = tempfile::tempdir().unwrap();
            let mut path = Core::Xray.core_dir(root.path());
            if depth > 0 {
                path.push("versions");
            }
            if depth > 1 {
                path.push("1.0.0");
            }
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            let status = Command::new("cmd")
                .args(["/C", "mklink", "/J"])
                .arg(&path)
                .arg(outside.path())
                .creation_flags(CREATE_NO_WINDOW)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .unwrap();
            assert!(status.success());
            assert!(version_directory(Core::Xray, root.path(), "1.0.0").is_err());
            assert!(delete_version(Core::Xray, root.path(), "1.0.0").is_err());
            assert!(installed_versions(Core::Xray, root.path()).is_err());
            assert_eq!(fs::read(outside.path().join("keep.txt")).unwrap(), b"keep");
            fs::remove_dir(path).unwrap();
        }
    }

    #[test]
    fn missing_executables_unsafe_paths_and_duplicate_artifacts_abort_before_installing() {
        let root = tempfile::tempdir().unwrap();
        let cases: &[&[(&str, &[u8])]] = &[
            &[("libcronet.dll", b"dll")],
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
            channel: UpdateChannel::Stable,
            tag: "v1.2.3".into(),
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
        let target = version_directory(Core::SingBox, root.path(), "1.0.0").unwrap();
        fs::create_dir_all(&target).unwrap();
        let exe = Core::SingBox.executable(root.path());
        fs::copy(version_fixture(), &exe).unwrap();
        assert_eq!(
            read_version_for(Core::SingBox, &exe).unwrap(),
            "1.2.3-beta.1"
        );
        fs::write(target.join("failure"), b"").unwrap();
        let error = read_version_for(Core::SingBox, &exe).unwrap_err();
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
        let target = version_directory(Core::SingBox, root.path(), "1.0.0").unwrap();
        fs::create_dir_all(&target).unwrap();
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
        let target = version_directory(Core::SingBox, root.path(), "1.0.0").unwrap();
        fs::create_dir_all(&target).unwrap();
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
            let release = fetch_releases(core, 1, &cancelled)
                .unwrap()
                .versions
                .into_iter()
                .find(|item| item.channel == channel)
                .expect("official first page must contain requested release channel");
            let mut prepared = prepare_version(core, root.path(), &release, &cancelled, |_| {})
                .unwrap_or_else(|error| panic!("{}: {error}", core.label()));
            let target = version_directory(core, root.path(), &release.version).unwrap();
            assert!(!target.exists());
            prepared.install().unwrap();
            assert_eq!(
                read_version_for(core, &target.join(artifacts(core)[0])).unwrap(),
                release.version
            );
            assert!(prepare_version(core, root.path(), &release, &cancelled, |_| {}).is_err());
            let local = installed_versions(core, root.path()).unwrap();
            assert_eq!(local.len(), 1);
            assert_eq!(local[0].channel, channel);
            eprintln!(
                "{} {channel:?} {}: isolated installation and version probe passed",
                core.label(),
                release.version
            );
        }
    }
}
