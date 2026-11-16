use std::collections::hash_map::RandomState;
use std::fs;
use std::hash::{BuildHasher, Hasher};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::Value;
use tempfile::NamedTempFile;

use crate::runtime::Core;

pub(crate) const TUN_PREFIX: &str = "tun-";
pub(crate) const RUNTIME_PREFIX: &str = ".ladder-tun-";

pub(crate) struct PreparedConfig {
    source: PathBuf,
    temporary: Option<NamedTempFile>,
    names: Vec<String>,
}

impl PreparedConfig {
    pub(crate) fn path(&self) -> &Path {
        self.temporary
            .as_ref()
            .map_or(&self.source, NamedTempFile::path)
    }

    pub(crate) fn names(&self) -> &[String] {
        &self.names
    }
}

pub(crate) fn prepare(core: Core, source: &Path) -> Result<PreparedConfig, String> {
    let text = fs::read(source).map_err(|e| format!("读取核心配置失败: {e}"))?;
    let text = if core == Core::Xray {
        mask_hash_comments(text)
    } else {
        text
    };
    let mut config: Value = serde_json_lenient::from_slice(&text).map_err(|e| {
        format!(
            "核心配置格式无效 (第 {} 行, 第 {} 列)",
            e.line(),
            e.column()
        )
    })?;
    let mut names = Vec::new();
    let inbounds_key = config_key(core, &config, "inbounds");
    if let Some(inbounds) = config.get_mut(&inbounds_key).and_then(Value::as_array_mut) {
        let kind = match core {
            Core::SingBox => "type",
            Core::Xray => "protocol",
        };
        for inbound in inbounds {
            let kind_key = config_key(core, inbound, kind);
            let is_tun = inbound
                .get(&kind_key)
                .and_then(Value::as_str)
                .is_some_and(|kind| match core {
                    Core::SingBox => kind == "tun",
                    Core::Xray => kind.eq_ignore_ascii_case("tun"),
                });
            if !is_tun {
                continue;
            }
            // 只有对象能通过上面的 TUN 类型检查。
            let (settings, key) = match core {
                Core::SingBox => (inbound.as_object_mut().unwrap(), "interface_name".into()),
                Core::Xray => {
                    let settings_key = config_key(core, inbound, "settings");
                    let settings = inbound
                        .as_object_mut()
                        .unwrap()
                        .entry(settings_key)
                        .or_insert_with(|| Value::Object(Default::default()));
                    let name_key = config_key(core, settings, "name");
                    let settings = settings
                        .as_object_mut()
                        .ok_or("Xray 的 TUN settings 必须是对象")?;
                    (settings, name_key)
                }
            };
            if settings.get(&key).is_some_and(|value| !value.is_string()) {
                return Err(format!("TUN 的 {key} 必须是字符串"));
            }
            let name = random_name();
            settings.insert(key, Value::String(name.clone()));
            names.push(name);
        }
    }
    let temporary = if names.is_empty() {
        None
    } else {
        let directory = source.parent().ok_or("无法定位核心配置目录")?;
        let mut file = tempfile::Builder::new()
            .prefix(RUNTIME_PREFIX)
            .suffix(".json")
            .tempfile_in(directory)
            .map_err(|e| format!("创建 TUN 运行配置失败: {e}"))?;
        serde_json::to_writer(file.as_file_mut(), &config)
            .map_err(|_| "写入 TUN 运行配置失败".to_owned())?;
        file.flush()
            .map_err(|e| format!("写入 TUN 运行配置失败: {e}"))?;
        Some(file)
    };
    Ok(PreparedConfig {
        source: source.to_owned(),
        temporary,
        names,
    })
}

fn config_key(core: Core, object: &Value, key: &str) -> String {
    // 保留 Xray 的原字段拼写，避免另插入 settings 或 name 而覆盖原设置。
    if core == Core::Xray
        && object.get(key).is_none()
        && let Some(actual) = object.as_object().and_then(|object| {
            object
                .keys()
                .find(|actual| actual.eq_ignore_ascii_case(key))
        })
    {
        return actual.clone();
    }
    key.to_owned()
}

fn random_name() -> String {
    let seed = RandomState::new();
    let mut hasher = seed.build_hasher();
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    hasher.write_u128(nanos);
    format!("{TUN_PREFIX}{:06x}", hasher.finish() & 0xff_ffff)
}

// Xray 还允许 # 行注释。只遮盖字符串和已有注释以外的 #，保留错误行列位置。
fn mask_hash_comments(mut text: Vec<u8>) -> Vec<u8> {
    let mut index = 0;
    while index < text.len() {
        match text[index] {
            b'"' => {
                index += 1;
                while index < text.len() {
                    match text[index] {
                        b'\\' => index += 2,
                        b'"' => {
                            index += 1;
                            break;
                        }
                        _ => index += 1,
                    }
                }
            }
            b'/' if text.get(index + 1) == Some(&b'*') => {
                index += 2;
                while index < text.len() {
                    if text[index] == b'*' && text.get(index + 1) == Some(&b'/') {
                        index += 2;
                        break;
                    }
                    index += 1;
                }
            }
            b'/' if text.get(index + 1) == Some(&b'/') => {
                while index < text.len() && !matches!(text[index], b'\r' | b'\n') {
                    index += 1;
                }
            }
            b'#' => {
                while index < text.len() && !matches!(text[index], b'\r' | b'\n') {
                    text[index] = b' ';
                    index += 1;
                }
            }
            _ => index += 1,
        }
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(path: &Path) -> Value {
        serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
    }

    #[test]
    fn sing_box_randomizes_all_tun_inbounds_without_changing_source_or_other_fields() {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("配置.json");
        let original = br#"{
            // Existing names outside TUN must not be replaced.
            "outbounds": [{"interface_name": "shared", "tag": "direct"}],
            "inbounds": [
                {"type":"mixed", "interface_name":"shared"},
                {"type":"tun", "interface_name":"shared", "mtu":1500},
                {"type":"tun", "tag":"second"},
            ],
        }"#;
        fs::write(&source, original).unwrap();
        let first = prepare(Core::SingBox, &source).unwrap();
        let second = prepare(Core::SingBox, &source).unwrap();
        assert_eq!(fs::read(&source).unwrap(), original);
        assert_eq!(first.path().parent(), source.parent());
        assert_ne!(first.path(), source);
        assert_eq!(first.names().len(), 2);
        assert_ne!(first.names(), second.names());
        assert_ne!(first.names()[0], first.names()[1]);
        assert!(first.names().iter().all(|name| {
            name.starts_with(TUN_PREFIX)
                && name.len() == TUN_PREFIX.len() + 6
                && name[TUN_PREFIX.len()..]
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit())
        }));
        let actual = read(first.path());
        assert_eq!(actual["outbounds"][0]["interface_name"], "shared");
        assert_eq!(actual["inbounds"][0]["interface_name"], "shared");
        assert_eq!(actual["inbounds"][1]["mtu"], 1500);
        assert_eq!(actual["inbounds"][1]["interface_name"], first.names()[0]);
        assert_eq!(actual["inbounds"][2]["interface_name"], first.names()[1]);
        assert_eq!(actual["inbounds"][2]["tag"], "second");
        let generated = first.path().to_owned();
        drop(first);
        assert!(!generated.exists());
        assert!(second.path().exists());
    }

    #[test]
    fn xray_adds_missing_settings_and_names_without_touching_other_inbounds() {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("xray.json");
        let original = r##"# "Leading # comment"
        {
            /* "#" block */
            "inbounds": [
                {"protocol":"socks", "settings":{"name":"shared"}},
                {"protocol":"tun", "settings":{"name":"shared", "MTU":1500}},
                {"protocol":"tun", "settings":{}},
                {"protocol":"tun"}
            ],
            // "#" line comment
            "log":{"value":"https://example.invalid/#\"fragment", "other":"\\#"} # tail
        }"##;
        fs::write(&source, original).unwrap();
        let prepared = prepare(Core::Xray, &source).unwrap();
        assert_eq!(fs::read_to_string(&source).unwrap(), original);
        let actual = read(prepared.path());
        assert_eq!(prepared.names().len(), 3);
        assert_eq!(actual["inbounds"][0]["settings"]["name"], "shared");
        assert_eq!(actual["inbounds"][1]["settings"]["MTU"], 1500);
        for (index, name) in prepared.names().iter().enumerate() {
            assert_eq!(actual["inbounds"][index + 1]["settings"]["name"], *name);
        }
        assert_eq!(
            actual["log"]["value"],
            "https://example.invalid/#\"fragment"
        );
        assert_eq!(actual["log"]["other"], "\\#");
    }

    #[test]
    fn xray_randomizes_tun_with_case_insensitive_fields_and_protocol() {
        for (inbounds, protocol, settings, name, kind) in [
            ("inbounds", "protocol", "settings", "name", "TUN"),
            ("Inbounds", "protocol", "settings", "name", "tun"),
            ("inbounds", "Protocol", "settings", "name", "tun"),
            ("inbounds", "protocol", "Settings", "name", "tun"),
            ("inbounds", "protocol", "settings", "Name", "tun"),
            ("iNbOuNdS", "pRoToCoL", "sEtTiNgS", "nAmE", "TuN"),
        ] {
            let directory = tempfile::tempdir().unwrap();
            let source = directory.path().join("xray.json");
            let mut expected = serde_json::json!({
                inbounds: [
                    { protocol: kind, settings: { name: "original", "MTU": 1400 } },
                    { "protocol": "socks", "settings": { "Name": "untouched" } },
                ],
                "outbounds": [{ "protocol": "freedom", "tag": "direct" }],
            });
            let original = serde_json::to_vec(&expected).unwrap();
            fs::write(&source, &original).unwrap();
            let prepared = prepare(Core::Xray, &source).unwrap();
            assert_eq!(prepared.names().len(), 1, "{inbounds}/{protocol}/{kind}");
            assert_ne!(prepared.path(), source);
            assert!(prepared.names()[0].starts_with(TUN_PREFIX));
            expected[inbounds][0][settings][name] = prepared.names()[0].clone().into();
            assert_eq!(read(prepared.path()), expected);
            assert_eq!(fs::read(&source).unwrap(), original);
        }
    }

    #[test]
    fn non_tun_configs_keep_the_original_path_and_bytes() {
        for (core, text) in [
            (Core::SingBox, "{ /* comment */ \"log\": {}, }"),
            (Core::Xray, "# comment\n{\"log\":{}, \"inbounds\":[]}"),
        ] {
            let directory = tempfile::tempdir().unwrap();
            let source = directory.path().join("core.json");
            fs::write(&source, text).unwrap();
            let prepared = prepare(core, &source).unwrap();
            assert_eq!(prepared.path(), source);
            assert!(prepared.names().is_empty());
            drop(prepared);
            assert_eq!(fs::read_to_string(&source).unwrap(), text);
            assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
        }
    }

    #[test]
    fn invalid_tun_fields_and_invalid_json_fail_without_source_changes_or_secret_details() {
        for (core, text) in [
            (
                Core::SingBox,
                r#"{"inbounds":[{"type":"tun","interface_name":5}]}"#,
            ),
            (
                Core::Xray,
                r#"{"inbounds":[{"protocol":"tun","settings":[]}] }"#,
            ),
            (
                Core::Xray,
                r#"{"inbounds":[{"protocol":"tun","settings":{"name":false}}]}"#,
            ),
            (
                Core::Xray,
                r#"{"inbounds":[{"protocol":"tun"}],"token":PRIVATE_VALUE}"#,
            ),
        ] {
            let directory = tempfile::tempdir().unwrap();
            let source = directory.path().join("core.json");
            fs::write(&source, text).unwrap();
            let error = prepare(core, &source).err().unwrap();
            assert!(!error.contains("PRIVATE_VALUE"));
            assert_eq!(fs::read_to_string(&source).unwrap(), text);
            assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
        }
    }
}
