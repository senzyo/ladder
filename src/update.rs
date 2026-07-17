//! 核心更新逻辑。
//!
//! 通过 GitHub Releases API 检查 sing-box / xray 的最新版本，
//! 与本地版本比较后决定是否下载更新。支持 CDN 代理、SHA256 校验、
//! 自动重试，下载完成后从 zip 中提取 exe 并替换。

use serde_json::Value;
use sha2::{Digest, Sha256};
use std::fs;
use std::io::{self, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::Command;
use tracing::{debug, error, info, warn};

use crate::error::AppError;

/// GitHub API 要求的 User-Agent 头，缺少会返回 403。
const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/149.0.0.0 Safari/537.36 Edg/149.0.0.0";

// 编译时根据目标架构确定下载文件名。
// amd64 编译产物只下载 amd64 核心，arm64 编译产物只下载 arm64 核心。
const SINGBOX_ARCH_SUFFIX: &str = if cfg!(target_arch = "aarch64") {
    "arm64"
} else {
    "amd64"
};

const XRAY_ZIP_NAME: &str = if cfg!(target_arch = "aarch64") {
    "Xray-windows-arm64-v8a.zip"
} else {
    "Xray-windows-64.zip"
};

/// 依次更新 sing-box 和 xray。
pub fn update_cores(
    exe_dir: &Path,
    gh_proxy_url: &str,
    max_retries: u32,
    delay_secs: u64,
) -> Result<(), AppError> {
    let exe_dir = exe_dir.to_path_buf();
    update_sing_box(&exe_dir, gh_proxy_url, max_retries, delay_secs)?;
    update_xray(&exe_dir, gh_proxy_url, max_retries, delay_secs)
}

/// 检查并更新 sing-box。
pub fn update_sing_box(
    exe_dir: &Path,
    gh_proxy_url: &str,
    max_retries: u32,
    delay_secs: u64,
) -> Result<(), AppError> {
    let exe_path = exe_dir.join("sing-box_core").join("sing-box.exe");
    let api_url = "https://api.github.com/repos/SagerNet/sing-box/releases/latest";

    let local = get_local_version(&exe_path, "version");
    let (remote_ver, assets) = fetch_release(api_url)?;
    debug!("[sing-box] 版本比较: local={local}, remote={remote_ver}");

    if !is_newer(&local, &remote_ver) {
        info!("[sing-box] 已是最新版本，跳过更新");
        crate::toast::show_toast("sing-box", "当前已是最新版本");
        return Ok(());
    }

    let zip_name = format!("sing-box-{}-windows-{}.zip", remote_ver, SINGBOX_ARCH_SUFFIX);
    let zip_path = exe_dir.join(&zip_name);

    let (download_url, expected_hash) = find_asset(&assets, &zip_name);
    let download_url = match download_url {
        Some(url) => url,
        None => {
            error!("[sing-box] 未找到发布文件: {zip_name}");
            return Err(AppError::Msg(format!("未找到发布文件: {zip_name}")));
        }
    };
    debug!(
        "[sing-box] 下载链接: {download_url}, SHA256: {}",
        expected_hash.as_deref().unwrap_or("无")
    );

    crate::toast::show_toast("sing-box", &format!("检测到新版本 v{remote_ver}"));

    let tag = "update-sing-box";
    let title = format!("sing-box v{remote_ver}");
    crate::toast::show_progress_toast(&title, tag);

    if let Err(e) = download_with_retry(
        &download_url,
        &zip_path,
        expected_hash.as_deref(),
        gh_proxy_url,
        max_retries,
        delay_secs,
    ) {
        error!("[sing-box] 下载失败: {e}");
        crate::toast::show_toast_tagged("sing-box", "下载失败，请稍后重试", tag);
        return Ok(());
    }

    let core_dir = exe_dir.join("sing-box_core");
    let nested_prefix = format!("sing-box-{}-windows-{}/", remote_ver, SINGBOX_ARCH_SUFFIX);
    backup_and_extract(&zip_path, &core_dir, Some(&nested_prefix))?;

    let _ = fs::remove_file(&zip_path);
    info!("[sing-box] 更新完成 -> v{remote_ver}");
    crate::toast::show_toast_tagged("sing-box", "更新完成", tag);
    Ok(())
}

/// 检查并更新 xray。
pub fn update_xray(
    exe_dir: &Path,
    gh_proxy_url: &str,
    max_retries: u32,
    delay_secs: u64,
) -> Result<(), AppError> {
    let exe_path = exe_dir.join("xray_core").join("xray.exe");
    let api_url = "https://api.github.com/repos/XTLS/Xray-core/releases/latest";

    let local = get_local_version(&exe_path, "version");
    let (remote_ver, assets) = fetch_release(api_url)?;
    debug!("[xray] 版本比较: local={local}, remote={remote_ver}");

    if !is_newer(&local, &remote_ver) {
        info!("[xray] 已是最新版本，跳过更新");
        crate::toast::show_toast("xray", "当前已是最新版本");
        return Ok(());
    }

    let zip_name = XRAY_ZIP_NAME;
    let zip_path = exe_dir.join(zip_name);

    let (download_url, expected_hash) = find_asset(&assets, zip_name);
    let download_url = match download_url {
        Some(url) => url,
        None => {
            error!("[xray] 未找到发布文件: {zip_name}");
            return Err(AppError::Msg(format!("未找到发布文件: {zip_name}")));
        }
    };
    debug!(
        "[xray] 下载链接: {download_url}, SHA256: {}",
        expected_hash.as_deref().unwrap_or("无")
    );

    crate::toast::show_toast("xray", &format!("检测到新版本 v{remote_ver}"));

    let tag = "update-xray";
    let title = format!("xray v{remote_ver}");
    crate::toast::show_progress_toast(&title, tag);

    if let Err(e) = download_with_retry(
        &download_url,
        &zip_path,
        expected_hash.as_deref(),
        gh_proxy_url,
        max_retries,
        delay_secs,
    ) {
        error!("[xray] 下载失败: {e}");
        crate::toast::show_toast_tagged("xray", "下载失败，请稍后重试", tag);
        return Ok(());
    }

    let core_dir = exe_dir.join("xray_core");
    backup_and_extract(&zip_path, &core_dir, None)?;

    let _ = fs::remove_file(&zip_path);
    info!("[xray] 更新完成 -> v{remote_ver}");
    crate::toast::show_toast_tagged("xray", "更新完成", tag);
    Ok(())
}

/// 运行可执行文件的版本命令并从 stdout 提取版本号，失败返回 "0.0.0"。
pub(crate) fn get_local_version(exe_path: &Path, version_arg: &str) -> String {
    let output = match Command::new(exe_path).arg(version_arg).output() {
        Ok(out) => out,
        Err(e) => {
            debug!("获取版本失败 ({}): {e}", exe_path.display());
            return "0.0.0".to_string();
        }
    };

    let text = String::from_utf8_lossy(&output.stdout);
    let version = extract_version(&text).unwrap_or_else(|| "0.0.0".to_string());
    debug!("本地版本: {} -> {version}", exe_path.display());
    version
}

/// 从命令输出中提取版本号（如 "sing-box version 1.13.13" → "1.13.13"）。
///
/// 逐字节扫描，累积数字和点号，遇到连字符停止（跳过 "-beta" 等后缀），
/// 遇到其他非数字字符终止。要求结果至少包含一个点号。
fn extract_version(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut start = None;
    let mut end = None;

    for (i, &b) in bytes.iter().enumerate() {
        if b.is_ascii_digit() || b == b'.' {
            if start.is_none() {
                start = Some(i);
            }
            end = Some(i + 1);
        } else if start.is_some() && b != b'-' {
            break;
        }
    }

    match (start, end) {
        (Some(s), Some(e)) => {
            let version = &text[s..e];
            if version.contains('.') {
                Some(version.to_string())
            } else {
                None
            }
        }
        _ => None,
    }
}

/// 比较两个版本号，remote > local 时返回 true。
/// 缺失的段按 0 处理，因此 "1.0" 等价于 "1.0.0"。
fn is_newer(local: &str, remote: &str) -> bool {
    let local: Vec<u32> = local.split('.').filter_map(|s| s.parse().ok()).collect();
    let remote: Vec<u32> = remote.split('.').filter_map(|s| s.parse().ok()).collect();

    for i in 0..local.len().max(remote.len()) {
        let l = local.get(i).copied().unwrap_or(0);
        let r = remote.get(i).copied().unwrap_or(0);
        if r > l {
            return true;
        }
        if r < l {
            return false;
        }
    }
    false
}

/// 调用 GitHub Releases API 获取最新版本号和 assets 列表。
fn fetch_release(api_url: &str) -> Result<(String, Vec<Value>), AppError> {
    debug!("请求 GitHub API: {api_url}");
    let resp = ureq::get(api_url)
        .header("User-Agent", USER_AGENT)
        .call()
        .map_err(|e| AppError::Msg(format!("请求 GitHub API 失败: {e}")))?;

    let body = resp
        .into_body()
        .read_to_string()
        .map_err(|e| AppError::Msg(format!("读取 GitHub API 响应失败: {e}")))?;

    let json: Value =
        serde_json::from_str(&body).map_err(|e| AppError::Msg(format!("解析 GitHub API 响应失败: {e}")))?;

    let tag = json["tag_name"]
        .as_str()
        .ok_or(AppError::Msg("GitHub API 响应缺少 tag_name".into()))?
        .to_string();

    let version = tag.trim_start_matches('v').to_string();
    let assets: Vec<Value> = json["assets"].as_array().cloned().unwrap_or_default();

    debug!("GitHub API 响应: tag={tag}, assets 数量={}", assets.len());
    Ok((version, assets))
}

/// 从 release assets 中查找指定文件名的下载 URL 和 digest 哈希值。
fn find_asset(assets: &[Value], file_name: &str) -> (Option<String>, Option<String>) {
    let asset = assets.iter().find(|a| a["name"].as_str() == Some(file_name));
    let url = asset
        .and_then(|a| a["browser_download_url"].as_str())
        .map(|s| s.to_string());
    let hash = asset
        .and_then(|a| a["digest"].as_str())
        .and_then(|d| d.split(':').next_back())
        .map(|s| s.to_string());
    (url, hash)
}

/// 带重试的下载。启用代理时将代理 URL 前缀拼接到下载链接。
/// 若提供 expected_hash，下载后校验 SHA256，不匹配则删除文件并重试。
fn download_with_retry(
    download_url: &str,
    dest: &Path,
    expected_hash: Option<&str>,
    gh_proxy_url: &str,
    max_retries: u32,
    delay_secs: u64,
) -> Result<(), AppError> {
    let url = if gh_proxy_url.is_empty() {
        download_url.to_string()
    } else {
        format!("{gh_proxy_url}{download_url}")
    };
    debug!(
        "下载准备: url={url}, 代理={}, hash={}",
        if gh_proxy_url.is_empty() { "禁用" } else { "启用" },
        expected_hash.unwrap_or("无")
    );

    for attempt in 1..=max_retries {
        if attempt > 1 {
            debug!("第 {attempt}/{max_retries} 次重试, 等待 {delay_secs}s...");
            std::thread::sleep(std::time::Duration::from_secs(delay_secs));
        } else {
            debug!("第 1/{max_retries} 次尝试下载...");
        }

        download_file(&url, dest)?;

        match expected_hash {
            Some(expected) => {
                let actual = sha256_file(dest)?;
                if actual.eq_ignore_ascii_case(expected) {
                    debug!("SHA256 校验通过: {actual}");
                    return Ok(());
                }
                warn!("SHA256 校验失败: expected={expected}, actual={actual}");
                let _ = fs::remove_file(dest);
            }
            None => return Ok(()),
        }
    }

    Err(AppError::Msg("下载文件校验失败，已达到最大重试次数".into()))
}

/// 下载单个文件到指定路径，已存在时先删除。
fn download_file(url: &str, dest: &Path) -> Result<(), AppError> {
    if dest.exists() {
        let _ = fs::remove_file(dest);
    }

    let resp = ureq::get(url)
        .header("User-Agent", USER_AGENT)
        .call()
        .map_err(|e| AppError::Msg(format!("下载失败: {e}")))?;

    let mut reader = resp.into_body().into_reader();
    let mut file = fs::File::create(dest).map_err(|e| AppError::Msg(format!("创建文件失败: {e}")))?;

    let bytes = io::copy(&mut reader, &mut file).map_err(|e| AppError::Msg(format!("写入文件失败: {e}")))?;
    debug!("下载完成: {} ({:.1} MB)", dest.display(), bytes as f64 / 1_048_576.0);

    Ok(())
}

/// 计算文件的 SHA256 哈希值，返回小写十六进制字符串。
fn sha256_file(path: &Path) -> Result<String, AppError> {
    let mut file = fs::File::open(path).map_err(|e| AppError::Msg(format!("打开文件计算 SHA256 失败: {e}")))?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 8192];
    loop {
        let n = file
            .read(&mut buf)
            .map_err(|e| AppError::Msg(format!("读取文件计算 SHA256 失败: {e}")))?;
        if n == 0 {
            break;
        }
        Digest::update(&mut hasher, &buf[..n]);
    }
    Ok(hasher.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

/// 备份核心目录并从 zip 解压全部内容。
///
/// 1. 删除 `{core_dir}_backup`（如存在）
/// 2. 重命名 `{core_dir}` → `{core_dir}_backup`
/// 3. 解压 zip 全部内容到 `{core_dir}`
///
/// `strip_prefix` 为 Some 时，跳过 zip 中以此开头的顶层目录（用于 sing-box 的嵌套目录结构）。
fn backup_and_extract(zip_path: &Path, core_dir: &Path, strip_prefix: Option<&str>) -> Result<(), AppError> {
    let backup_dir = PathBuf::from(format!("{}_backup", core_dir.display()));

    if backup_dir.exists() {
        debug!("删除旧备份: {}", backup_dir.display());
        fs::remove_dir_all(&backup_dir).map_err(|e| AppError::Msg(format!("删除旧备份失败: {e}")))?;
    }

    if core_dir.exists() {
        debug!("备份: {} -> {}", core_dir.display(), backup_dir.display());
        fs::rename(core_dir, &backup_dir).map_err(|e| AppError::Msg(format!("备份目录失败: {e}")))?;
    }

    fs::create_dir_all(core_dir).map_err(|e| AppError::Msg(format!("创建核心目录失败: {e}")))?;

    let file = fs::File::open(zip_path).map_err(|e| AppError::Msg(format!("打开 zip 文件失败: {e}")))?;
    let reader = BufReader::new(file);
    let mut archive = zip::ZipArchive::new(reader).map_err(|e| AppError::Msg(format!("解析 zip 文件失败: {e}")))?;

    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
            .map_err(|e| AppError::Msg(format!("读取 zip 条目失败: {e}")))?;

        let raw_name = entry.name().to_string();

        let rel_path = match strip_prefix {
            Some(prefix) => {
                if let Some(rest) = raw_name.strip_prefix(prefix) {
                    rest.to_string()
                } else {
                    continue;
                }
            }
            None => raw_name,
        };

        if rel_path.is_empty() {
            continue;
        }

        let out_path = core_dir.join(&rel_path);

        if !out_path.starts_with(core_dir) {
            warn!("zip 条目路径越界，跳过: {}", rel_path);
            continue;
        }

        if entry.is_dir() {
            fs::create_dir_all(&out_path).map_err(|e| AppError::Msg(format!("创建目录失败: {e}")))?;
        } else {
            if let Some(parent) = out_path.parent() {
                fs::create_dir_all(parent).map_err(|e| AppError::Msg(format!("创建父目录失败: {e}")))?;
            }
            let mut out = fs::File::create(&out_path).map_err(|e| AppError::Msg(format!("创建文件失败: {e}")))?;
            io::copy(&mut entry, &mut out).map_err(|e| AppError::Msg(format!("解压文件失败: {e}")))?;
        }
    }

    debug!("解压完成: {} -> {}", zip_path.display(), core_dir.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_version_simple() {
        assert_eq!(extract_version("sing-box version 1.13.13"), Some("1.13.13".into()));
        assert_eq!(extract_version("Xray 24.12.18"), Some("24.12.18".into()));
        assert_eq!(extract_version("v1.0.0"), Some("1.0.0".into()));
    }

    #[test]
    fn test_extract_version_edge_cases() {
        assert_eq!(extract_version("no version here"), None);
        assert_eq!(extract_version("123"), None);
        assert_eq!(extract_version("version 1.2.3-beta"), Some("1.2.3".into()));
        assert_eq!(extract_version("1.2.3.4"), Some("1.2.3.4".into()));
    }

    #[test]
    fn test_is_newer_true() {
        assert!(is_newer("1.0.0", "1.0.1"));
        assert!(is_newer("1.0.0", "1.1.0"));
        assert!(is_newer("1.0.0", "2.0.0"));
        assert!(is_newer("1.0.9", "1.0.10"));
        assert!(is_newer("0.0.0", "1.0.0"));
    }

    #[test]
    fn test_is_newer_false_equal() {
        assert!(!is_newer("1.0.0", "1.0.0"));
        assert!(!is_newer("2.0.0", "2.0.0"));
    }

    #[test]
    fn test_is_newer_false_older() {
        assert!(!is_newer("2.0.0", "1.0.0"));
        assert!(!is_newer("1.0.1", "1.0.0"));
    }

    #[test]
    fn test_is_newer_different_lengths() {
        assert!(is_newer("1.0", "1.0.1"));
        assert!(!is_newer("1.0.1", "1.0"));
    }
}
