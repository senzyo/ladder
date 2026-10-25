//! 候选配置选择，以及配置文件的发现、排序与去重。

use std::fs;
use std::path::PathBuf;

use crate::core::Core;

#[derive(Clone)]
pub struct ConfigSelection {
    pub core: Core,
    pub path: PathBuf,
}

/// 从多个目录中收集所有 .json 文件, 按文件名排序并去重。
pub fn find_json_configs(dirs: &[PathBuf]) -> Vec<PathBuf> {
    let mut paths = Vec::new();

    for dir in dirs {
        let Ok(entries) = fs::read_dir(dir) else {
            continue;
        };

        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) == Some("json") {
                paths.push(path);
            }
        }
    }

    paths.sort_by(|a, b| a.file_name().cmp(&b.file_name()));
    // 按文件名去重而不是按完整路径: 菜单项显示的是文件名, 不同目录下的
    // 同名配置在菜单里根本无法区分。依赖上面已按文件名排序。
    paths.dedup_by(|a, b| a.file_name() == b.file_name());
    paths
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_find_json_configs_sorts_and_filters() {
        let dir = tempfile::tempdir().expect("应能创建隔离配置测试目录");
        fs::write(dir.path().join("b.json"), "{}").expect("应能写入配置扫描测试样本");
        fs::write(dir.path().join("a.json"), "{}").expect("应能写入配置扫描测试样本");
        fs::write(dir.path().join("c.txt"), "x").expect("应能写入配置扫描测试样本");

        let found = find_json_configs(&[dir.path().to_path_buf()]);

        let names: Vec<_> = found
            .iter()
            .map(|p| p.file_name().expect("扫描结果应包含文件名").to_owned())
            .collect();
        assert_eq!(names, ["a.json", "b.json"], "只收 .json 且按文件名排序");
    }

    /// 不同目录下的同名配置在菜单里显示成同一个名字, 必须去重。
    #[test]
    fn test_find_json_configs_dedups_by_file_name() {
        let dir = tempfile::tempdir().expect("应能创建隔离配置测试目录");
        let first = dir.path().join("first");
        let second = dir.path().join("second");
        fs::create_dir_all(&first).expect("应能创建配置扫描测试子目录");
        fs::create_dir_all(&second).expect("应能创建配置扫描测试子目录");
        fs::write(first.join("same.json"), "{}").expect("应能写入配置扫描测试样本");
        fs::write(second.join("same.json"), "{}").expect("应能写入配置扫描测试样本");
        fs::write(second.join("other.json"), "{}").expect("应能写入配置扫描测试样本");

        let found = find_json_configs(&[first, second]);

        let names: Vec<_> = found
            .iter()
            .map(|p| p.file_name().expect("扫描结果应包含文件名").to_owned())
            .collect();
        assert_eq!(names, ["other.json", "same.json"]);
    }

    #[test]
    fn test_find_json_configs_skips_missing_dir() {
        let dir = tempfile::tempdir().expect("应能创建隔离配置测试目录");
        let found = find_json_configs(&[dir.path().join("不存在")]);
        assert!(found.is_empty());
    }
}
