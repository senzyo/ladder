use std::env;
use std::ffi::OsStr;
use std::fs;
use std::path::PathBuf;
use std::process;
use std::thread;
use std::time::{Duration, Instant};

fn main() {
    if matches!(
        env::args_os().nth(1).as_deref(),
        Some(arg) if arg == OsStr::new("version") || arg == OsStr::new("-v")
    ) {
        let exe = env::current_exe().unwrap();
        if exe.file_stem().unwrap() == "xray" {
            println!("Xray 1.0.0 (Xray, Penetrates Everything.) fixture");
        } else if exe.file_stem().unwrap() == "mihomo" {
            println!("Mihomo Meta v1.0.0 windows fixture");
        } else {
            println!("sing-box version 1.0.0");
        }
        return;
    }
    let file = PathBuf::from(env::args_os().last().unwrap());
    if env::args_os().nth(1).as_deref() == Some(OsStr::new("--child")) {
        fs::write(file, process::id().to_string()).unwrap();
        thread::sleep(Duration::from_secs(90));
        return;
    }
    let config = fs::read_to_string(&file).unwrap();
    if env::args().any(|arg| matches!(arg.as_str(), "check" | "-test" | "-t")) {
        if config.contains("fixture_validation_wait") {
            fs::write(file.with_extension("started"), process::id().to_string()).unwrap();
            let deadline = Instant::now() + Duration::from_secs(15);
            while !file.with_extension("continue").is_file() {
                assert!(
                    Instant::now() < deadline,
                    "validation fixture was not released"
                );
                thread::sleep(Duration::from_millis(10));
            }
        }
        if config.contains("fixture_failure") {
            eprintln!("fixture configuration rejected");
            process::exit(42);
        }
        return;
    }
    if config.contains("fixture_tun") {
        let exe = env::current_exe().unwrap();
        let root = file.ancestors().nth(2).unwrap();
        let label = exe.file_stem().unwrap().to_str().unwrap();
        fs::write(
            root.join(format!("{label}.config-path")),
            file.to_str().unwrap(),
        )
        .unwrap();
    }
    if config.contains("fixture_start_failure") {
        process::exit(42);
    }
    if config.contains("fixture_native_exit") {
        let exe = env::current_exe().unwrap();
        let label = exe.file_stem().unwrap().to_str().unwrap();
        let prefix = match label {
            "mihomo" => "time=\"fixture\" level=fatal msg=",
            "xray" => "Failed to start: main:",
            _ => "FATAL[0000]",
        };
        eprintln!("{prefix} fixture-native-exit-{label}: cannot find the file specified");
        process::exit(42);
    }
    if config.contains("fixture_live_error") {
        let exe = env::current_exe().unwrap();
        let label = exe.file_stem().unwrap().to_str().unwrap();
        let prefix = match label {
            "mihomo" => "time=\"fixture\" level=error msg=",
            "xray" => "[Error]",
            _ => "ERROR[0000]",
        };
        eprintln!("{prefix} fixture-live-output-{label}: cannot find the file specified");
    }
    if config.contains("fixture_tree") {
        let exe = env::current_exe().unwrap();
        let assets = exe.parent().unwrap().canonicalize().unwrap();
        assert_eq!(
            env::current_dir().unwrap().canonicalize().unwrap(),
            file.ancestors().nth(2).unwrap().canonicalize().unwrap()
        );
        let (resource_path, expected) = match exe.file_stem().unwrap().to_str().unwrap() {
            "xray" => (env::var_os("XRAY_LOCATION_ASSET").unwrap(), assets),
            "mihomo" => {
                assert_eq!(env::args_os().nth(1).unwrap(), "-d");
                assert_eq!(env::args_os().nth(3).unwrap(), "-f");
                let config_dir = file.parent().unwrap().canonicalize().unwrap();
                assert_eq!(
                    fs::read(config_dir.join("rules/relative.yaml")).unwrap(),
                    b"fixture rule"
                );
                (env::args_os().nth(2).unwrap(), config_dir)
            }
            _ => {
                assert_eq!(env::args_os().nth(2).unwrap(), "-D");
                (env::args_os().nth(3).unwrap(), assets)
            }
        };
        assert_eq!(
            PathBuf::from(resource_path).canonicalize().unwrap(),
            expected
        );
        // 刻意留下持有输出管道的后代, 测试其随核心或宿主强制退出。
        process::Command::new(exe)
            .arg("--child")
            .arg(file.with_extension("child"))
            .spawn()
            .unwrap();
    }
    fs::write(file.with_extension("pid"), process::id().to_string()).unwrap();
    let deadline = Instant::now() + Duration::from_secs(90);
    while Instant::now() < deadline {
        if file.with_extension("exit").is_file() {
            process::exit(23);
        }
        thread::sleep(Duration::from_millis(50));
    }
}
