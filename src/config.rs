use std::{fs, path::Path};

use json_comments::StripComments;

pub(crate) fn xray_uses_tun(path: &Path) -> Result<bool, String> {
    let bytes = fs::read(path).map_err(|error| format!("读取配置文件失败: {error}"))?;
    let input = bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(&bytes);
    // 这里只识别需要接管 DNS 的 TUN 入站，配置有效性由 Xray 自己判断。
    let config: serde_json::Value = match serde_json::from_reader(StripComments::new(input)) {
        Ok(config) => config,
        Err(_) => return Ok(false),
    };
    Ok(config["inbounds"].as_array().is_some_and(|inbounds| {
        inbounds
            .iter()
            .any(|inbound| inbound["protocol"].as_str() == Some("tun"))
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_tun_with_xray_comments_without_rewriting_config() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("注释配置.json");
        let input = concat!(
            "\u{feff}{\n",
            "  // 行注释\n",
            "  \"inbounds\": [\n",
            "    /* 块注释 */ {\"protocol\": \"tun\"} # 行尾注释\n",
            "  ]\n",
            "}\n"
        );
        fs::write(&path, input).unwrap();
        assert!(xray_uses_tun(&path).unwrap());
        assert_eq!(fs::read_to_string(&path).unwrap(), input);
    }

    #[test]
    fn ignores_tun_in_strings_comments_and_non_inbound_fields() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("ordinary.json");
        let input = r##"{
            "remarks": "\"inbounds\": [{\"protocol\": \"tun\"}] // # /* */ \\",
            "address": "https://example.invalid/#fragment",
            // "inbounds": [{"protocol": "tun"}],
            /* "inbounds": [{"protocol": "tun"}], */
            # "inbounds": [{"protocol": "tun"}],
            "outbounds": [{"protocol": "tun"}],
            "inbounds": [{"protocol": "socks"}]
        }"##;
        fs::write(&path, input).unwrap();
        assert!(!xray_uses_tun(&path).unwrap());
    }

    #[test]
    fn leaves_syntax_errors_to_the_core_and_reports_read_errors() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("invalid.json");
        for input in [
            r#"{"inbounds": [{"protocol": "tun"}],"#,
            r#"{"inbounds": [{"protocol": "tun"}],}"#,
            r#"{"inbounds": []}"#,
        ] {
            fs::write(&path, input).unwrap();
            assert!(!xray_uses_tun(&path).unwrap());
        }
        fs::write(&path, [0xff, 0xfe]).unwrap();
        assert!(!xray_uses_tun(&path).unwrap());
        assert!(xray_uses_tun(&directory.path().join("missing.json")).is_err());
    }
}
