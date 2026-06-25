use serde_json::Value;
use sha2::{Digest, Sha256};
use std::fs;
use std::io::{self, BufReader};
use std::path::Path;
use std::process::Command;

const GH_PROXY: &str = "https://gh-proxy.com/";
const USER_AGENT: &str = "sing-box-with-xray";
const MAX_RETRIES: u32 = 3;

pub fn update_cores(
    work_dir: &Path,
    sing_ver: Option<&str>,
    xray_ver: Option<&str>,
) -> Result<(), String> {
    let work_dir = work_dir.to_path_buf();
    update_sing_box(&work_dir, sing_ver)?;
    update_xray(&work_dir, xray_ver)
}

pub fn update_sing_box(work_dir: &Path, local_version: Option<&str>) -> Result<(), String> {
    let exe_path = work_dir.join("sing-box.exe");
    let api_url = "https://api.github.com/repos/SagerNet/sing-box/releases/latest";

    let local = match local_version {
        Some(v) if v != "0.0.0" => v.to_string(),
        _ => get_local_version(&exe_path, "version"),
    };
    let (remote_ver, assets) = fetch_release(api_url)?;

    if !is_newer(&local, &remote_ver) {
        crate::toast::show_toast("sing-box", "当前已是最新版本");
        return Ok(());
    }

    let zip_name = format!("sing-box-{}-windows-amd64.zip", remote_ver);
    let zip_path = work_dir.join(&zip_name);

    let download_url = match find_asset_url(&assets, &zip_name) {
        Some(url) => url,
        None => return Err(format!("未找到发布文件: {zip_name}")),
    };
    let expected_hash = find_asset_digest(&assets, &zip_name);

    crate::toast::show_toast("sing-box", &format!("检测到新版本 v{remote_ver}"));

    let tag = "update-sing-box";
    let title = format!("sing-box v{remote_ver}");
    crate::toast::show_progress_toast(&title, tag);

    if download_with_retry(
        &download_url,
        &zip_path,
        expected_hash.as_deref(),
    )
    .is_err()
    {
        crate::toast::show_toast_tagged("sing-box", "下载失败，请稍后重试", tag);
        return Ok(());
    }

    let exe_in_zip = format!("sing-box-{}-windows-amd64/sing-box.exe", remote_ver);
    replace_exe_from_zip(&zip_path, &exe_in_zip, &exe_path)?;

    let _ = fs::remove_file(&zip_path);
    crate::toast::show_toast_tagged("sing-box", "更新完成", tag);
    Ok(())
}

pub fn update_xray(work_dir: &Path, local_version: Option<&str>) -> Result<(), String> {
    let exe_path = work_dir.join("xray.exe");
    let api_url = "https://api.github.com/repos/XTLS/Xray-core/releases/latest";

    let local = match local_version {
        Some(v) if v != "0.0.0" => v.to_string(),
        _ => get_local_version(&exe_path, "version"),
    };
    let (remote_ver, assets) = fetch_release(api_url)?;

    if !is_newer(&local, &remote_ver) {
        crate::toast::show_toast("xray", "当前已是最新版本");
        return Ok(());
    }

    let zip_name = "Xray-windows-64.zip";
    let zip_path = work_dir.join(zip_name);

    let download_url = match find_asset_url(&assets, zip_name) {
        Some(url) => url,
        None => return Err(format!("未找到发布文件: {zip_name}")),
    };
    let expected_hash = find_asset_digest(&assets, zip_name);

    crate::toast::show_toast("xray", &format!("检测到新版本 v{remote_ver}"));

    let tag = "update-xray";
    let title = format!("xray v{remote_ver}");
    crate::toast::show_progress_toast(&title, tag);

    if download_with_retry(
        &download_url,
        &zip_path,
        expected_hash.as_deref(),
    )
    .is_err()
    {
        crate::toast::show_toast_tagged("xray", "下载失败，请稍后重试", tag);
        return Ok(());
    }

    replace_exe_from_zip(&zip_path, "xray.exe", &exe_path)?;

    let _ = fs::remove_file(&zip_path);
    crate::toast::show_toast_tagged("xray", "更新完成", tag);
    Ok(())
}

pub(crate) fn get_local_version(exe_path: &Path, version_arg: &str) -> String {
    let output = match Command::new(exe_path).arg(version_arg).output() {
        Ok(out) => out,
        Err(_) => return "0.0.0".to_string(),
    };

    let text = String::from_utf8_lossy(&output.stdout);
    extract_version(&text).unwrap_or_else(|| "0.0.0".to_string())
}

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

fn fetch_release(api_url: &str) -> Result<(String, Vec<Value>), String> {
    let resp = ureq::get(api_url)
        .set("User-Agent", USER_AGENT)
        .call()
        .map_err(|e| format!("请求 GitHub API 失败: {e}"))?;

    let body = resp
        .into_string()
        .map_err(|e| format!("读取 GitHub API 响应失败: {e}"))?;

    let json: Value =
        serde_json::from_str(&body).map_err(|e| format!("解析 GitHub API 响应失败: {e}"))?;

    let tag = json["tag_name"]
        .as_str()
        .ok_or("GitHub API 响应缺少 tag_name")?
        .to_string();

    let version = tag.trim_start_matches('v').to_string();

    let assets: Vec<Value> = json["assets"]
        .as_array()
        .cloned()
        .unwrap_or_default();

    Ok((version, assets))
}

fn find_asset_url(assets: &[Value], file_name: &str) -> Option<String> {
    assets
        .iter()
        .find(|a| a["name"].as_str() == Some(file_name))
        .and_then(|a| a["browser_download_url"].as_str())
        .map(|s| s.to_string())
}

fn find_asset_digest(assets: &[Value], file_name: &str) -> Option<String> {
    assets
        .iter()
        .find(|a| a["name"].as_str() == Some(file_name))
        .and_then(|a| a["digest"].as_str())
        .and_then(|d| d.split(':').next_back())
        .map(|s| s.to_string())
}

fn download_with_retry(
    download_url: &str,
    dest: &Path,
    expected_hash: Option<&str>,
) -> Result<(), String> {
    let proxy_url = format!("{GH_PROXY}{download_url}");

    for attempt in 1..=MAX_RETRIES {
        if attempt > 1 {
            std::thread::sleep(std::time::Duration::from_secs(1));
        }

        download_file(&proxy_url, dest)?;

        match expected_hash {
            Some(expected) => {
                let actual = sha256_file(dest)?;
                if actual.eq_ignore_ascii_case(expected) {
                    return Ok(());
                }
                let _ = fs::remove_file(dest);
            }
            None => return Ok(()),
        }
    }

    Err("下载文件校验失败，已达到最大重试次数".to_string())
}

fn download_file(url: &str, dest: &Path) -> Result<(), String> {
    if dest.exists() {
        let _ = fs::remove_file(dest);
    }

    let resp = ureq::get(url)
        .set("User-Agent", USER_AGENT)
        .call()
        .map_err(|e| format!("下载失败: {e}"))?;

    let mut reader = resp.into_reader();
    let mut file = fs::File::create(dest).map_err(|e| format!("创建文件失败: {e}"))?;

    io::copy(&mut reader, &mut file).map_err(|e| format!("写入文件失败: {e}"))?;

    Ok(())
}

fn sha256_file(path: &Path) -> Result<String, String> {
    use std::io::Read;
    let mut file =
        fs::File::open(path).map_err(|e| format!("打开文件计算 SHA256 失败: {e}"))?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 8192];
    loop {
        let n = file.read(&mut buf).map_err(|e| format!("读取文件计算 SHA256 失败: {e}"))?;
        if n == 0 {
            break;
        }
        Digest::update(&mut hasher, &buf[..n]);
    }
    let hash = hasher.finalize();
    Ok(hash.iter().map(|b| format!("{b:02x}")).collect())
}

fn replace_exe_from_zip(
    zip_path: &Path,
    exe_name_in_zip: &str,
    exe_dest: &Path,
) -> Result<(), String> {
    let file = fs::File::open(zip_path)
        .map_err(|e| format!("打开 zip 文件失败: {e}"))?;
    let reader = BufReader::new(file);
    let mut archive =
        zip::ZipArchive::new(reader).map_err(|e| format!("解析 zip 文件失败: {e}"))?;

    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
            .map_err(|e| format!("读取 zip 条目失败: {e}"))?;

        let name = entry.name().to_string();
        if name.ends_with(exe_name_in_zip) && !name.contains('/') {
            let mut out = fs::File::create(exe_dest)
                .map_err(|e| format!("创建目标 exe 文件失败: {e}"))?;
            io::copy(&mut entry, &mut out)
                .map_err(|e| format!("解压 exe 文件失败: {e}"))?;
            return Ok(());
        }
    }

    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
            .map_err(|e| format!("读取 zip 条目失败: {e}"))?;

        if entry.name().ends_with(exe_name_in_zip) {
            let mut out = fs::File::create(exe_dest)
                .map_err(|e| format!("创建目标 exe 文件失败: {e}"))?;
            io::copy(&mut entry, &mut out)
                .map_err(|e| format!("解压 exe 文件失败: {e}"))?;
            return Ok(());
        }
    }

    Err(format!("在 zip 中未找到: {exe_name_in_zip}"))
}
