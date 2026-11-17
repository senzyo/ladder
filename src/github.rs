use ureq::http::Uri;

pub(crate) const DEFAULT_PROXY: &str = "https://axisnow.gh-proxy.org/";

pub(crate) fn normalized_prefix(prefix: &str) -> Result<String, String> {
    let prefix = prefix.trim();
    if prefix.is_empty() {
        return Ok(String::new());
    }
    let invalid =
        || "GitHub 加速链接前缀须为 HTTPS 地址, 不能包含账号、密码、查询参数或片段".to_owned();
    let uri: Uri = prefix.parse().map_err(|_| invalid())?;
    if uri.scheme_str() != Some("https")
        || uri.host().is_none_or(str::is_empty)
        || uri
            .authority()
            .is_none_or(|value| value.as_str().contains('@'))
        || uri.query().is_some()
        || prefix.contains(['#', '\\'])
        || prefix.chars().any(char::is_whitespace)
    {
        return Err(invalid());
    }
    Ok(format!("{}/", prefix.trim_end_matches('/')))
}

pub(crate) fn url(prefix: &str, official_url: &str) -> Result<String, String> {
    let uri: Uri = official_url
        .parse()
        .map_err(|_| "GitHub 资源地址无效".to_owned())?;
    if uri.scheme_str() != Some("https")
        || !matches!(uri.host(), Some("api.github.com" | "github.com"))
        || uri
            .authority()
            .is_none_or(|value| value.as_str().contains('@'))
        || uri.port().is_some()
    {
        return Err("GitHub 资源地址无效".into());
    }
    Ok(format!("{}{official_url}", normalized_prefix(prefix)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefixes_apply_to_both_api_and_assets() {
        let api = "https://api.github.com/repos/SagerNet/sing-box/releases?page=2";
        let asset = "https://github.com/XTLS/Xray-core/releases/download/v1.0.0/core.zip";
        assert_eq!(url("", api).unwrap(), api);
        assert_eq!(
            url("  https://gh-proxy.org  ", api).unwrap(),
            format!("https://gh-proxy.org/{api}")
        );
        assert_eq!(
            url("https://example.com/proxy/", asset).unwrap(),
            format!("https://example.com/proxy/{asset}")
        );
    }

    #[test]
    fn invalid_prefixes_do_not_echo_credentials() {
        for prefix in [
            "http://example.com",
            "https://user:password@example.com",
            "https://example.com/?token=secret",
            "https://example.com/#secret",
            "https://",
            "https://example.com/ bad",
            "https://example.com\\bad",
        ] {
            let error = normalized_prefix(prefix).unwrap_err();
            assert!(!error.contains("secret"));
            assert!(!error.contains("password"));
        }
        assert!(url("", "https://github.com.evil.example/file").is_err());
        assert!(url("", "https://user@github.com/file").is_err());
    }
}
