use std::env;
use std::ffi::OsStr;
use std::fs;
use std::path::PathBuf;
use std::process;
use std::thread;
use std::time::{Duration, Instant};

fn main() {
    let file = PathBuf::from(env::args_os().last().unwrap());
    if env::args_os().nth(1).as_deref() == Some(OsStr::new("--child")) {
        fs::write(file, process::id().to_string()).unwrap();
        thread::sleep(Duration::from_secs(90));
        return;
    }
    let config = fs::read_to_string(&file).unwrap();
    if config.contains("fixture_failure") {
        eprintln!("access is denied: PRIVATE_FIXTURE_VALUE");
        process::exit(42);
    }
    if config.contains("fixture_tree") {
        let exe = env::current_exe().unwrap();
        let assets = exe.parent().unwrap().canonicalize().unwrap();
        assert_eq!(
            env::current_dir().unwrap().canonicalize().unwrap(),
            assets.parent().unwrap()
        );
        let resource_path = if exe.file_stem().unwrap() == "xray" {
            env::var_os("XRAY_LOCATION_ASSET").unwrap()
        } else {
            assert_eq!(env::args_os().nth(2).unwrap(), "-D");
            env::args_os().nth(3).unwrap()
        };
        assert_eq!(PathBuf::from(resource_path).canonicalize().unwrap(), assets);
        // 刻意留下持有输出管道的后代，测试其随核心或宿主强制退出。
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
