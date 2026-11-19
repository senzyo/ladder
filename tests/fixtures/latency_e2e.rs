use super::measure;
use crate::core::Core;
use crate::core_output::ConfigFailure;
use crate::managed_child::{JobProcess, ManagedChild};
use serde_json::json;
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

const HOST: &str = "ladder-latency.invalid";
const TIMEOUT: Duration = Duration::from_secs(5);

fn wait(predicate: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(7);
    while !predicate() {
        assert!(
            Instant::now() < deadline,
            "runtime did not publish the expected state"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn accept(listener: &TcpListener) -> TcpStream {
    listener.set_nonblocking(true).unwrap();
    let deadline = Instant::now() + Duration::from_secs(15);
    let socket = loop {
        match listener.accept() {
            Ok((socket, _)) => break socket,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                assert!(
                    Instant::now() < deadline,
                    "listener did not receive a connection"
                );
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(error) => panic!("accept: {error}"),
        }
    };
    socket.set_nonblocking(false).unwrap();
    socket.set_read_timeout(Some(TIMEOUT)).unwrap();
    socket
}

// Only the local SOCKS5 node can return the synthetic target's response.
fn read_through_node(listener: &TcpListener) -> TcpStream {
    let mut socket = accept(listener);
    let mut greeting = [0; 2];
    socket.read_exact(&mut greeting).unwrap();
    assert_eq!(greeting[0], 5);
    let mut methods = vec![0; usize::from(greeting[1])];
    socket.read_exact(&mut methods).unwrap();
    assert!(methods.contains(&0));
    socket.write_all(&[5, 0]).unwrap();

    let mut connect = [0; 5];
    socket.read_exact(&mut connect).unwrap();
    assert_eq!(connect[..4], [5, 1, 0, 3]);
    let mut hostname = vec![0; usize::from(connect[4])];
    socket.read_exact(&mut hostname).unwrap();
    assert_eq!(hostname, HOST.as_bytes());
    let mut port = [0; 2];
    socket.read_exact(&mut port).unwrap();
    assert_eq!(u16::from_be_bytes(port), 80);
    socket.write_all(&[5, 0, 0, 1, 127, 0, 0, 1, 0, 0]).unwrap();

    read_request(&socket);
    socket
}

fn respond_through_node(listener: &TcpListener) -> TcpStream {
    let mut socket = read_through_node(listener);
    socket
        .write_all(b"HTTP/1.1 204 No Content\r\n\r\n")
        .unwrap();
    socket
}

fn read_request(socket: &TcpStream) {
    let mut reader = BufReader::new(socket);
    let mut line = String::new();
    reader.read_line(&mut line).unwrap();
    assert!(matches!(
        line.trim(),
        "GET /check HTTP/1.1" | "HEAD /check HTTP/1.1"
    ));
    loop {
        line.clear();
        assert!(reader.read_line(&mut line).unwrap() > 0);
        if line == "\r\n" {
            break;
        }
    }
}

fn executable(core: Core) -> PathBuf {
    let binaries = PathBuf::from(std::env::var_os("LADDER_LATENCY_TEST_CORES").expect(
        "set LADDER_LATENCY_TEST_CORES to the sandbox directory containing all three cores",
    ));
    binaries.join(format!("{}.exe", core.label()))
}

fn native_diagnostic(core: Core, message: &str) -> &str {
    // Compare the diagnostic itself; log timestamps vary between checker invocations.
    message
        .lines()
        .find_map(|line| match core {
            Core::Mihomo => line.split_once("level=error ").map(|(_, text)| text),
            Core::SingBox => line
                .strip_prefix("FATAL[")
                .and_then(|line| line.split_once("] "))
                .map(|(_, text)| text),
            Core::Xray => line.strip_prefix("Failed to start: "),
        })
        .unwrap_or_else(|| panic!("Missing native {core:?} diagnostic: {message}"))
}

fn proxy_config(core: Core, original_port: u16, port: u16) -> serde_json::Value {
    match core {
        Core::Mihomo => json!({
            "mode": "rule", "mixed-port": original_port,
            "profile": {"store-selected":true},
            "proxies": [{"name":"node", "type":"socks5", "server":"127.0.0.1", "port":port}],
            "proxy-groups": [{"name":"chosen", "type":"select", "proxies":["node", "DIRECT"]}],
            "rules": ["MATCH,chosen"]
        }),
        Core::SingBox => json!({
            "inbounds": [{"type":"mixed", "listen":"127.0.0.1", "listen_port":original_port}],
            "outbounds": [{"type":"socks", "tag":"node", "server":"127.0.0.1", "server_port":port}],
            "route": {"final":"node"}
        }),
        Core::Xray => json!({
            "inbounds":[{"protocol":"http", "listen":"127.0.0.1", "port":original_port}],
            "outbounds":[{"protocol":"socks", "tag":"node", "settings":{
                "servers":[{"address":"127.0.0.1", "port":port}]
            }}]
        }),
    }
}

fn verify_config(core: Core) {
    let executable = executable(core);
    let folder = tempfile::tempdir().unwrap();
    let directory = core.config_dir(folder.path());
    fs::create_dir_all(&directory).unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let original_listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let original_port = original_listener.local_addr().unwrap().port();
    let mut config = proxy_config(core, original_port, port);
    let controller = if core == Core::Mihomo {
        let socket = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = socket.local_addr().unwrap();
        config["external-controller"] = address.to_string().into();
        config.as_object_mut().unwrap().remove("profile"); // exercise mihomo's default
        let nodes = config.as_object_mut().unwrap().remove("proxies").unwrap();
        fs::create_dir(directory.join("providers")).unwrap();
        fs::write(
            directory.join("providers/local.yaml"),
            json!({"proxies":nodes}).to_string(),
        )
        .unwrap();
        config["proxy-providers"] = json!({"local":{"type":"file","path":"providers/local.yaml"}});
        config["proxy-groups"][0]["proxies"] = json!(["REJECT"]);
        config["proxy-groups"][0]["use"] = json!(["local"]);
        Some((socket, address))
    } else {
        None
    };
    let config_path = directory.join(if core == Core::Mihomo {
        "sample.yaml"
    } else {
        "sample.json"
    });
    let source = serde_json::to_vec(&config).unwrap();
    fs::write(&config_path, &source).unwrap();
    // Retain each connection until the HTTP client has received its response.
    let node = std::thread::spawn(move || {
        (0..3)
            .map(|_| respond_through_node(&listener))
            .collect::<Vec<_>>()
    });
    drop(original_listener);
    let controller = controller.map(|(_, address)| address);
    let running = ManagedChild::spawn(
        core,
        folder.path(),
        &executable,
        executable.parent().unwrap(),
        crate::tun_config::prepare(core, &config_path).unwrap(),
    )
    .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while TcpStream::connect(("127.0.0.1", original_port)).is_err() {
        assert!(
            running.try_wait().unwrap().is_none(),
            "original {core:?} exited"
        );
        assert!(Instant::now() < deadline, "original {core:?} did not start");
        std::thread::sleep(Duration::from_millis(25));
    }
    let cache_before = controller.map(|address| {
        let client: ureq::Agent = ureq::Agent::config_builder()
            .proxy(None)
            .timeout_global(Some(TIMEOUT))
            .build()
            .into();
        let endpoint = format!("http://{address}/proxies/chosen");
        wait(|| client.get(&endpoint).call().is_ok());
        let response = client
            .get(&endpoint)
            .call()
            .unwrap()
            .body_mut()
            .read_to_string()
            .unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&response).unwrap()["now"],
            "REJECT"
        );
        assert_eq!(
            client
                .put(&endpoint)
                .send(r#"{"name":"node"}"#)
                .unwrap()
                .status(),
            204
        );
        fs::read(directory.join("cache.db")).unwrap()
    });
    let url = format!("http://{HOST}/check");
    super::request(original_port, &url, TIMEOUT).unwrap();
    let output = tempfile::tempfile().unwrap();
    let check =
        JobProcess::spawn_validation(core, &executable, folder.path(), &config_path, &output)
            .unwrap();
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if let Some(status) = check.try_wait().unwrap() {
            assert!(
                status.success(),
                "{core:?} validation failed with a live instance"
            );
            break;
        }
        assert!(Instant::now() < deadline, "{core:?} validation timed out");
        std::thread::sleep(Duration::from_millis(20));
    }
    let probe = || {
        use crate::runtime::{ValidatedConfig, executable_stamp};
        let checked = ValidatedConfig::hold(
            config_path.clone(),
            executable_stamp(&config_path).unwrap(),
            &executable,
            executable_stamp(&executable).unwrap(),
        )
        .unwrap();
        measure(
            core,
            folder.path(),
            checked,
            &url,
            TIMEOUT,
            &AtomicBool::new(false),
            &crate::wireguard::Sessions::default(),
        )
    };
    probe().unwrap_or_else(|error| panic!("{}: {error:?}", core.label()));
    assert!(running.try_wait().unwrap().is_none());
    super::request(original_port, &url, TIMEOUT).unwrap();
    node.join().unwrap();
    assert!(probe().is_err(), "closed proxy node reported success");
    assert!(running.try_wait().unwrap().is_none());
    running.terminate().unwrap();
    if let Some(cache) = cache_before {
        assert_eq!(fs::read(directory.join("cache.db")).unwrap(), cache);
    }
    assert_eq!(fs::read(&config_path).unwrap(), source);
    assert!(directory.read_dir().unwrap().all(|entry| {
        !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".ladder-")
    }));
}

#[test]
#[ignore = "requires real cores in LADDER_LATENCY_TEST_CORES; run in Windows Sandbox or a VM"]
fn mihomo_reaches_target_through_configured_node() {
    verify_config(Core::Mihomo);
}

#[test]
#[ignore = "requires real cores in LADDER_LATENCY_TEST_CORES; run in Windows Sandbox or a VM"]
fn sing_box_reaches_target_through_configured_node() {
    verify_config(Core::SingBox);
}

#[test]
#[ignore = "requires real sing-box in Windows Sandbox or a VM"]
fn wireguard_probe_reuses_the_live_inbound_and_skips_duplicate_identities() {
    use crate::runtime::{Action, ConfigLatency, LatencyTarget, Runtime, Settings};
    use base64::Engine;
    use base64::engine::general_purpose::STANDARD;

    let folder = tempfile::tempdir().unwrap();
    let root = folder.path();
    let core = Core::SingBox;
    let live = "live.json";
    let duplicate = "duplicate.json";
    let configs = core.config_dir(root);
    fs::create_dir_all(&configs).unwrap();
    let exe = core.executable(root);
    fs::create_dir_all(exe.parent().unwrap()).unwrap();
    fs::copy(executable(core), exe).unwrap();
    let port = TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let target = TcpListener::bind("127.0.0.1:0").unwrap();
    // Only requests authenticated at the live inbound can reach the target.
    let source = json!({
        "inbounds":[{"type":"mixed","listen":"127.0.0.1","listen_port":port,
            "users":[{"username":"fixture","password":"fixture-pass"}]}],
        "endpoints":[{"type":"wireguard","tag":"wg","system":false,
            "address":["10.77.0.2/32"],"private_key":STANDARD.encode([7u8; 32]),
            "peers":[{"address":"127.0.0.1","port":9,"public_key":STANDARD.encode([19u8; 32]),
                "allowed_ips":["10.77.0.1/32"]}]}],
        "outbounds":[{"type":"direct","tag":"direct"}],
        "route":{"rules":[{"auth_user":"fixture","action":"route","outbound":"direct"},
            {"action":"reject"}]}
    });
    for name in [live, duplicate] {
        fs::write(configs.join(name), source.to_string()).unwrap();
    }
    let settings = Settings {
        latency_target: LatencyTarget::Custom,
        custom_latency_url: format!("http://{}/check", target.local_addr().unwrap()),
        ..Default::default()
    };
    fs::write(
        root.join("ladder-state.json"),
        json!({"settings":settings,"sing_box":live,"sing_box_version":"1.0.0"}).to_string(),
    )
    .unwrap();
    let runtime = Runtime::open(root.to_owned()).unwrap();
    runtime.send(Action::Start(core)).unwrap();
    wait(|| runtime.snapshot().core(core).operation.is_none());
    let started = runtime.snapshot().core(core).clone();
    assert!(started.running(), "{started:?}");
    wait(|| TcpStream::connect(("127.0.0.1", port)).is_ok());

    runtime.send(Action::TestAllLatencies(core)).unwrap();
    let listener = std::thread::spawn(move || {
        let mut socket = accept(&target);
        read_request(&socket);
        socket
            .write_all(b"HTTP/1.1 204 No Content\r\nConnection: close\r\n\r\n")
            .unwrap();
    });
    wait(|| !runtime.snapshot().core(core).latency_busy());
    let result = runtime.snapshot().core(core).clone();
    assert!(
        matches!(result.latency(live), Some(ConfigLatency::Measured(_))),
        "{result:?}"
    );
    assert!(
        matches!(result.latency(duplicate), Some(ConfigLatency::Failed(ConfigFailure::Ladder(message))) if message.contains("WireGuard")),
        "{result:?}"
    );
    assert!(result.running() && result.current.as_deref() == Some(live));
    listener.join().unwrap();

    runtime.send(Action::Stop(core)).unwrap();
    wait(|| {
        let snapshot = runtime.snapshot();
        let state = snapshot.core(core);
        !state.running() && state.operation.is_none()
    });
    let mut no_inbound = source;
    no_inbound.as_object_mut().unwrap().remove("inbounds");
    fs::write(configs.join(live), no_inbound.to_string()).unwrap();
    runtime.send(Action::Start(core)).unwrap();
    wait(|| runtime.snapshot().core(core).operation.is_none());
    assert!(runtime.snapshot().core(core).running());
    runtime
        .send(Action::TestLatency(core, live.into()))
        .unwrap();
    wait(|| !runtime.snapshot().core(core).latency_busy());
    let state = runtime.snapshot().core(core).clone();
    assert!(
        matches!(state.latency(live), Some(ConfigLatency::Failed(ConfigFailure::Ladder(message))) if message == crate::i18n::system::latency_wireguard_no_proxy()),
        "{state:?}"
    );
    assert!(state.running());
    runtime.shutdown().unwrap();
}

#[test]
#[ignore = "requires real cores in LADDER_LATENCY_TEST_CORES; run in Windows Sandbox or a VM"]
fn xray_reaches_target_through_configured_node() {
    verify_config(Core::Xray);
}

#[test]
#[ignore = "requires real cores in LADDER_LATENCY_TEST_CORES; run in Windows Sandbox or a VM"]
fn startup_and_switch_check_the_original_file_before_changing_the_live_core() {
    use crate::runtime::{Action, ConfigValidation, Runtime, Settings};

    let folder = tempfile::tempdir().unwrap();
    let root = folder.path();
    let initial = "initial.json";
    let alternate = "alternate.json";
    for core in Core::ALL {
        let directory = core.config_dir(root);
        fs::create_dir_all(&directory).unwrap();
        for name in [initial, alternate] {
            fs::write(directory.join(name), "{\"inbounds\": [\n").unwrap();
        }
        let exe = core.executable(root);
        fs::create_dir_all(exe.parent().unwrap()).unwrap();
        fs::copy(executable(core), exe).unwrap();
    }
    fs::write(
        root.join("ladder-state.json"),
        json!({
            "settings": Settings::default(),
            "mihomo": initial, "sing_box": initial, "xray": initial,
            "mihomo_version":"1.0.0", "sing_box_version":"1.0.0", "xray_version":"1.0.0"
        })
        .to_string(),
    )
    .unwrap();
    let runtime = Runtime::open(root.to_owned()).unwrap();

    for core in Core::ALL {
        let run = |action| {
            runtime.send(action).unwrap();
            wait(|| runtime.snapshot().core(core).operation.is_none());
            runtime.snapshot().core(core).clone()
        };
        let native_error = |name: &str| {
            let snapshot = runtime.snapshot();
            let state = snapshot.core(core);
            assert!(state.run_error.is_none(), "{core:?}: {state:?}");
            let Some(ConfigValidation::Invalid(ConfigFailure::Core(message))) =
                state.validation(name)
            else {
                panic!("{core:?} did not report its native syntax error for {name}: {state:?}");
            };
            let detail = native_diagnostic(core, message);
            assert!(
                !state
                    .output_error
                    .as_deref()
                    .is_some_and(|error| error.contains(detail))
            );
            detail.to_owned()
        };

        let failed = run(Action::Start(core));
        assert!(!failed.running());
        let start_error = native_error(initial);
        runtime
            .send(Action::ValidateConfig(core, initial.into()))
            .unwrap();
        wait(|| !runtime.snapshot().core(core).validation_busy);
        assert_eq!(native_error(initial), start_error);

        let directory = core.config_dir(root);
        let node = TcpListener::bind("127.0.0.1:0").unwrap();
        let node_port = node.local_addr().unwrap().port();
        let write_valid = |name| {
            let reserved = TcpListener::bind("127.0.0.1:0").unwrap();
            let port = reserved.local_addr().unwrap().port();
            fs::write(
                directory.join(name),
                proxy_config(core, port, node_port).to_string(),
            )
            .unwrap();
            port
        };
        let start_request = |port| {
            wait(|| TcpStream::connect(("127.0.0.1", port)).is_ok());
            std::thread::spawn(move || {
                super::request(
                    port,
                    &format!("http://{HOST}/check"),
                    Duration::from_secs(10),
                )
            })
        };
        let port = write_valid(initial);
        let started = run(Action::Start(core));
        assert!(
            started.running() && started.run_error.is_none(),
            "{core:?}: {started:?}"
        );
        assert_eq!(started.validation(initial), Some(&ConfigValidation::Valid));

        let pending = start_request(port);
        let mut live_connection = read_through_node(&node);
        let rejected = run(Action::SwitchConfig(core, alternate.into()));
        assert!(rejected.running());
        assert_eq!(rejected.current.as_deref(), Some(initial));
        let switch_error = native_error(alternate);
        runtime
            .send(Action::ValidateConfig(core, alternate.into()))
            .unwrap();
        wait(|| !runtime.snapshot().core(core).validation_busy);
        assert_eq!(native_error(alternate), switch_error);
        live_connection
            .write_all(b"HTTP/1.1 204 No Content\r\n\r\n")
            .unwrap();
        pending.join().unwrap().unwrap();

        let port = write_valid(alternate);
        let switched = run(Action::SwitchConfig(core, alternate.into()));
        assert!(
            switched.running() && switched.run_error.is_none(),
            "{core:?}: {switched:?}"
        );
        assert_eq!(switched.current.as_deref(), Some(alternate));
        assert_eq!(
            switched.validation(alternate),
            Some(&ConfigValidation::Valid)
        );
        let pending = start_request(port);
        let _connection = respond_through_node(&node);
        pending.join().unwrap().unwrap();
        run(Action::Stop(core));
    }
    runtime.shutdown().unwrap();
}

#[test]
#[ignore = "requires real cores in LADDER_LATENCY_TEST_CORES; run in Windows Sandbox or a VM"]
fn native_errors_reach_the_row_for_single_and_batch_actions() {
    use crate::runtime::{
        Action, ConfigLatency, ConfigValidation, LatencyTarget, Runtime, Settings,
    };

    let folder = tempfile::tempdir().unwrap();
    let root = folder.path();
    let syntax = "a-invalid-syntax.json";
    let inbound = "b-invalid-inbound.json";
    let outbound = "c-invalid-outbound.json";
    let good = "z-valid.json";
    let marker = "ladder-invalid-protocol";
    let node = TcpListener::bind("127.0.0.1:0").unwrap();
    node.set_nonblocking(true).unwrap();
    let inbound_port = TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    for core in Core::ALL {
        let bad_outbound = match core {
            Core::Mihomo => json!({
                "proxies":[{"name":"fixture","type":marker,"server":"127.0.0.1","port":1}],
                "rules":["MATCH,fixture"]
            }),
            Core::SingBox => json!({"outbounds":[{"type":marker,"tag":"fixture"}]}),
            Core::Xray => json!({"outbounds":[{"protocol":marker,"tag":"fixture"}]}),
        };
        let configs = core.config_dir(root);
        fs::create_dir_all(&configs).unwrap();
        fs::write(configs.join(syntax), "{\"inbounds\": [\n").unwrap();
        fs::write(configs.join(outbound), bad_outbound.to_string()).unwrap();
        let mut source = proxy_config(core, inbound_port, node.local_addr().unwrap().port());
        fs::write(configs.join(good), source.to_string()).unwrap();
        // The probe replaces these inbound settings, so only checking the copy would pass.
        match core {
            Core::Mihomo => source["mixed-port"] = marker.into(),
            Core::SingBox => source["inbounds"][0]["type"] = marker.into(),
            Core::Xray => source["inbounds"][0]["protocol"] = marker.into(),
        }
        fs::write(configs.join(inbound), source.to_string()).unwrap();
        let exe = core.executable(root);
        fs::create_dir_all(exe.parent().unwrap()).unwrap();
        fs::copy(executable(core), exe).unwrap();
    }
    let settings = Settings {
        latency_target: LatencyTarget::Custom,
        custom_latency_url: format!("http://{HOST}/check"),
        ..Default::default()
    };
    fs::write(
        root.join("ladder-state.json"),
        json!({
            "settings":settings, "mihomo_version":"1.0.0",
            "sing_box_version":"1.0.0", "xray_version":"1.0.0"
        })
        .to_string(),
    )
    .unwrap();
    for batch in [false, true] {
        // Start each mode without a previously cached manual validation.
        let runtime = Runtime::open(root.to_owned()).unwrap();
        for core in Core::ALL {
            let invalid = [syntax, inbound, outbound];
            if batch {
                runtime.send(Action::TestAllLatencies(core)).unwrap();
            } else {
                for name in invalid {
                    runtime
                        .send(Action::TestLatency(core, name.into()))
                        .unwrap();
                    wait(|| !runtime.snapshot().core(core).latency_busy());
                    assert_eq!(
                        node.accept().unwrap_err().kind(),
                        std::io::ErrorKind::WouldBlock
                    );
                }
                runtime
                    .send(Action::TestLatency(core, good.into()))
                    .unwrap();
            }
            let _response = respond_through_node(&node);
            wait(|| !runtime.snapshot().core(core).latency_busy());
            let measured = runtime.snapshot();
            let rows = measured.core(core);
            assert_eq!(rows.validation(good), Some(&ConfigValidation::Valid));
            assert!(matches!(
                rows.latency(good),
                Some(ConfigLatency::Measured(_))
            ));

            if batch {
                runtime.send(Action::ValidateAllConfigs(core)).unwrap();
                wait(|| !runtime.snapshot().core(core).validation_busy);
            } else {
                for name in invalid {
                    runtime
                        .send(Action::ValidateConfig(core, name.into()))
                        .unwrap();
                    wait(|| !runtime.snapshot().core(core).validation_busy);
                }
            }
            let checked = runtime.snapshot();
            for name in invalid {
                let Some(ConfigValidation::Invalid(ConfigFailure::Core(preflight))) =
                    rows.validation(name)
                else {
                    panic!("{core:?} {name} had no native preflight error")
                };
                let Some(ConfigValidation::Invalid(ConfigFailure::Core(manual))) =
                    checked.core(core).validation(name)
                else {
                    panic!("{core:?} {name} had no native manual validation error")
                };
                assert_eq!(
                    native_diagnostic(core, preflight),
                    native_diagnostic(core, manual)
                );
                assert!(!preflight.contains('\u{1b}'));
                assert!(
                    matches!(rows.latency(name), Some(ConfigLatency::Failed(ConfigFailure::Core(message))) if message == preflight)
                );
            }
        }
        runtime.shutdown().unwrap();
    }
}

#[test]
#[ignore = "requires a real Xray in LADDER_LATENCY_TEST_CORES; run in Windows Sandbox or a VM"]
fn runtime_reports_latency_expires_old_results_and_enforces_timeout() {
    use crate::runtime::{Action, ConfigLatency, LatencyTarget, Runtime, Settings};

    let folder = tempfile::tempdir().unwrap();
    let root = folder.path();
    let core = Core::Xray;
    let name = "direct.json";
    let path = core.config_dir(root).join(name);
    fs::create_dir_all(core.config_dir(root)).unwrap();
    let config = br#"{"outbounds":[{"protocol":"freedom"}]}"#;
    fs::write(&path, config).unwrap();
    for version in ["1.0.0", "2.0.0"] {
        let directory = core.core_dir(root).join(version);
        fs::create_dir_all(&directory).unwrap();
        fs::copy(executable(core), directory.join("xray.exe")).unwrap();
    }
    fs::write(
        root.join("ladder-state.json"),
        json!({
            "settings": Settings::default(), "xray_version":"1.0.0"
        })
        .to_string(),
    )
    .unwrap();
    let runtime = Runtime::open(root.to_owned()).unwrap();

    let target = TcpListener::bind("127.0.0.1:0").unwrap();
    runtime
        .send(Action::SetLatencyTarget(LatencyTarget::Custom))
        .unwrap();
    runtime
        .set_custom_latency_url(format!("http://{}/check", target.local_addr().unwrap()))
        .unwrap()
        .recv_blocking()
        .unwrap();
    let start = || {
        runtime
            .send(Action::TestLatency(core, name.to_owned()))
            .unwrap();
        let socket = accept(&target);
        read_request(&socket);
        socket
    };
    let success = || {
        start()
            .write_all(b"HTTP/1.1 204 No Content\r\nConnection: close\r\n\r\n")
            .unwrap();
        wait(|| {
            matches!(
                runtime.snapshot().xray.latency(name),
                Some(ConfigLatency::Measured(_))
            )
        });
    };
    success();
    runtime.send(Action::SetLatencyTimeout(3)).unwrap();
    wait(|| {
        let snapshot = runtime.snapshot();
        snapshot.settings.latency_timeout_secs == 3 && snapshot.xray.latency(name).is_none()
    });
    success();
    fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .unwrap()
        .write_all(b"\n")
        .unwrap();
    wait(|| runtime.snapshot().xray.latency(name).is_none());

    let socket = start();
    let started = Instant::now();
    wait(|| !runtime.snapshot().xray.latency_busy());
    let elapsed = started.elapsed();
    assert_eq!(
        runtime.snapshot().xray.latency(name),
        Some(&ConfigLatency::Failed(ConfigFailure::Ladder(
            crate::i18n::system::latency_timeout().into()
        )))
    );
    assert!(
        (Duration::from_millis(2500)..Duration::from_secs(5)).contains(&elapsed),
        "{elapsed:?}"
    );
    eprintln!("Configured timeout 3 s; elapsed after HTTP request arrived: {elapsed:?}");
    drop(socket);

    runtime.send(Action::SetLatencyTimeout(8)).unwrap();
    wait(|| runtime.snapshot().settings.latency_timeout_secs == 8);
    let mut socket = start();
    runtime.send(Action::SetLatencyTimeout(7)).unwrap();
    wait(|| runtime.snapshot().settings.latency_timeout_secs == 7);
    assert_eq!(
        runtime.snapshot().xray.latency(name),
        Some(&ConfigLatency::Testing)
    );
    assert!(runtime.snapshot().xray.latency("other.json").is_none());

    runtime
        .send(Action::SwitchVersion(core, "2.0.0".into()))
        .unwrap();
    wait(|| {
        let state = runtime.snapshot().xray;
        state.operation.is_none() && state.selected_version.as_deref() == Some("2.0.0")
    });
    assert!(runtime.snapshot().xray.testing_version("1.0.0"));
    runtime
        .send(Action::DeleteVersion(core, "1.0.0".into()))
        .unwrap();
    wait(|| runtime.snapshot().xray.operation.is_none());
    let old_version = core.core_dir(root).join("1.0.0");
    assert!(old_version.join("xray.exe").is_file());
    assert_eq!(
        runtime.snapshot().xray.update_error.as_deref(),
        Some(crate::i18n::runtime::latency_busy())
    );
    socket
        .write_all(b"HTTP/1.1 204 No Content\r\nConnection: close\r\n\r\n")
        .unwrap();
    wait(|| !runtime.snapshot().xray.latency_busy());
    assert!(runtime.snapshot().xray.latency(name).is_none());
    runtime
        .send(Action::DeleteVersion(core, "1.0.0".into()))
        .unwrap();
    wait(|| runtime.snapshot().xray.operation.is_none());
    assert!(!old_version.exists());
    runtime.shutdown().unwrap();
}

#[test]
#[ignore = "requires a real Xray in LADDER_LATENCY_TEST_CORES; run in Windows Sandbox or a VM"]
fn batch_reports_each_result_and_drops_remaining_work_on_exit() {
    use crate::runtime::{Action, ConfigLatency, LatencyTarget, Runtime, Settings};

    let folder = tempfile::tempdir().unwrap();
    let root = folder.path();
    let core = Core::Xray;
    let current = "z-current.json";
    let invalid = "a-invalid.json";
    let last = "b-last.json";
    let configs = core.config_dir(root);
    fs::create_dir_all(&configs).unwrap();
    for name in [current, last] {
        fs::write(
            configs.join(name),
            br#"{"outbounds":[{"protocol":"freedom"}]}"#,
        )
        .unwrap();
    }
    fs::write(
        configs.join(invalid),
        br#"{"outbounds":[{"protocol":"unrecognized-protocol"}]}"#,
    )
    .unwrap();
    let version = core.core_dir(root).join("1.0.0");
    fs::create_dir_all(&version).unwrap();
    fs::copy(executable(core), version.join("xray.exe")).unwrap();

    let target = TcpListener::bind("127.0.0.1:0").unwrap();
    let settings = Settings {
        latency_target: LatencyTarget::Custom,
        latency_timeout_secs: 3,
        custom_latency_url: format!("http://{}/check", target.local_addr().unwrap()),
        ..Default::default()
    };
    fs::write(
        root.join("ladder-state.json"),
        json!({
            "settings":settings, "xray":current, "xray_version":"1.0.0"
        })
        .to_string(),
    )
    .unwrap();
    let runtime = Runtime::open(root.to_owned()).unwrap();
    let request = || {
        let socket = accept(&target);
        read_request(&socket);
        socket
    };
    let response = b"HTTP/1.1 204 No Content\r\nConnection: close\r\n\r\n";

    runtime.send(Action::TestAllLatencies(core)).unwrap();
    let mut first = request();
    let state = runtime.snapshot().xray;
    assert_eq!(state.latency_progress(), Some((1, 3)));
    assert_eq!(state.latency(current), Some(&ConfigLatency::Testing));
    for name in [invalid, last] {
        assert_eq!(state.latency(name), Some(&ConfigLatency::Queued));
    }
    assert!(runtime.send(Action::TestAllLatencies(core)).is_err());
    assert!(
        runtime
            .send(Action::TestLatency(core, last.into()))
            .is_err()
    );
    first.write_all(response).unwrap();
    let mut third = request();
    let state = runtime.snapshot().xray;
    assert_eq!(state.latency_progress(), Some((3, 3)));
    assert!(matches!(
        state.latency(current),
        Some(ConfigLatency::Measured(_))
    ));
    assert!(matches!(
        state.latency(invalid),
        Some(ConfigLatency::Failed(ConfigFailure::Core(message))) if message.contains("unrecognized-protocol")
    ));
    assert_eq!(state.latency(last), Some(&ConfigLatency::Testing));
    third.write_all(response).unwrap();
    wait(|| !runtime.snapshot().xray.latency_busy());
    let state = runtime.snapshot().xray;
    assert_eq!(state.latency_progress(), None);
    assert!(matches!(
        state.latency(last),
        Some(ConfigLatency::Measured(_))
    ));

    runtime.send(Action::TestAllLatencies(core)).unwrap();
    let mut first = request();
    runtime.send(Action::SetLatencyTimeout(4)).unwrap();
    wait(|| runtime.snapshot().settings.latency_timeout_secs == 4);
    assert_eq!(
        runtime.snapshot().xray.latency(current),
        Some(&ConfigLatency::Testing)
    );
    first.write_all(response).unwrap();
    wait(|| !runtime.snapshot().xray.latency_busy());
    let state = runtime.snapshot().xray;
    assert!(
        [current, invalid, last]
            .iter()
            .all(|name| state.latency(name).is_none())
    );
    assert_eq!(
        target.accept().unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );

    runtime.send(Action::TestAllLatencies(core)).unwrap();
    let held = request();
    runtime.shutdown().unwrap();
    drop(held);
    let state = runtime.snapshot().xray;
    assert!(!state.latency_busy());
    assert!(state.latency_progress().is_none());
    assert!(
        [current, invalid, last]
            .iter()
            .all(|name| state.latency(name).is_none())
    );
    assert_eq!(
        target.accept().unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );
    assert!(configs.read_dir().unwrap().all(|entry| {
        !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".ladder-")
    }));
}
