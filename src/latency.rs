use std::fs;
use std::io::{self, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use windows_sys::Win32::{
    Foundation::{ERROR_INSUFFICIENT_BUFFER, NO_ERROR},
    NetworkManagement::IpHelper::{
        GetExtendedTcpTable, MIB_TCP6ROW_OWNER_PID, MIB_TCPROW_OWNER_PID,
        TCP_TABLE_OWNER_PID_LISTENER,
    },
    Networking::WinSock::{AF_INET, AF_INET6},
};

use crate::core::Core;
use crate::core_output::{ConfigFailure, probe_diagnostic};
use crate::i18n::system as t;
use crate::managed_child::JobProcess;
use crate::runtime::ValidatedConfig;
use crate::tun_config::{config_key, parse_json, parse_mihomo};
use crate::wireguard::{Identities, Probe, Sessions};

mod mihomo_cache;

#[cfg(test)]
#[path = "../tests/fixtures/latency_e2e.rs"]
mod latency_e2e;

pub(crate) const CONFIG_PREFIX: &str = ".ladder-latency-";

pub(crate) fn measure(
    core: Core,
    root: &Path,
    checked: ValidatedConfig,
    url: &str,
    timeout: Duration,
    cancelled: &AtomicBool,
    sessions: &Sessions,
) -> Result<Duration, ConfigFailure> {
    if cancelled.load(Ordering::Acquire) {
        return Err(t::latency_cancelled().into());
    }
    let bytes = fs::read(&checked.config).map_err(t::core_config_read)?;
    let digest = Sha256::digest(&bytes).into();
    let mut source = if core == Core::Mihomo {
        let yaml = parse_mihomo(&bytes).map_err(|_| t::latency_config())?;
        serde_json::to_value(yaml).map_err(|_| t::latency_config())?
    } else {
        parse_json(core, bytes).map_err(|_| t::latency_config())?
    };
    let probe = |source: &Value| {
        sessions.probe(
            Identities::probe_keys(core, source),
            &checked.config,
            &checked.exe,
            digest,
        )
    };
    let mut plan = probe(&source);
    if matches!(plan, Probe::UnknownConflict)
        && core == Core::Mihomo
        && freeze_cached_providers(
            &mut source,
            checked.config.parent().ok_or(t::core_config_directory())?,
        )
    {
        plan = probe(&source);
    }
    let _session = match plan {
        Probe::Isolated(session) => session,
        Probe::Conflict => return Err(t::latency_wireguard_conflict().into()),
        Probe::UnknownConflict => return Err(t::latency_wireguard_unknown().into()),
        Probe::Online(online) => {
            let running = online.pid();
            if let Some(pid) = *running
                && let Some(candidate) = crate::live_proxy::candidates(core, &source)
                    .into_iter()
                    .find(|candidate| owns_address(pid, candidate.address).unwrap_or(false))
            {
                drop(checked);
                return request_via(candidate.proxy, url, timeout).map_err(ConfigFailure::Ladder);
            }
            drop(running);
            // A subscription can also turn out to contain no WireGuard nodes at all.
            if core != Core::Mihomo
                || !freeze_cached_providers(
                    &mut source,
                    checked.config.parent().ok_or(t::core_config_directory())?,
                )
                || !matches!(probe(&source), Probe::Isolated(None))
            {
                return Err(t::latency_wireguard_no_proxy().into());
            }
            None
        }
    };
    let reservation =
        TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).map_err(|_| t::latency_start())?;
    let port = reservation
        .local_addr()
        .map_err(|_| t::latency_start())?
        .port();
    let mut config = if core == Core::Mihomo {
        mihomo_config(source, port)?
    } else {
        json_config(core, source, port)?
    };
    let source_dir = checked.config.parent().ok_or(t::core_config_directory())?;
    let home = if core == Core::Mihomo && config["profile"]["store-selected"] != false {
        let snapshot = mihomo_cache::probe_home(source_dir).map_err(t::latency_cache)?;
        if snapshot.is_none() {
            config["profile"]["store-selected"] = false.into();
        }
        snapshot
    } else {
        None
    };
    // JSON is also valid mihomo YAML.
    let mut file = tempfile::Builder::new()
        .prefix(CONFIG_PREFIX)
        .suffix(if core == Core::Mihomo {
            ".yaml"
        } else {
            ".json"
        })
        .tempfile_in(home.as_ref().map_or(source_dir, |dir| dir.path()))
        .map_err(|_| t::latency_config())?;
    serde_json::to_writer(file.as_file_mut(), &config).map_err(|_| t::latency_config())?;
    file.flush().map_err(|_| t::latency_config())?;
    let (reader, writer) = io::pipe().map_err(|_| t::latency_start())?;
    drop(reservation);
    let child = JobProcess::spawn_latency(core, &checked.exe, root, file.path(), &writer)
        .map_err(|_| t::latency_start())?;
    drop(checked);
    drop(writer);
    let output = thread::Builder::new()
        .name("latency-core-output".into())
        .spawn(move || probe_diagnostic(reader))
        .map_err(|_| t::latency_start())?;
    let ready = wait_for_listener(&child, port, cancelled);
    let early_exit = ready.is_err() && child.try_wait().is_ok_and(|status| status.is_some());
    let result = ready.and_then(|_| request(port, url, timeout));
    let stopped = child.terminate();
    drop(child);
    let (diagnostic, unlabelled) = output.join().unwrap_or_default();
    if cancelled.load(Ordering::Acquire) {
        return Err(t::latency_cancelled().into());
    }
    stopped
        .map_err(|_| t::managed_process_stop_timeout().to_owned())
        .and(result)
        .map_err(
            |fallback| match diagnostic.or(unlabelled.filter(|_| early_exit)) {
                Some(message) => ConfigFailure::Core(message),
                None => ConfigFailure::Ladder(fallback),
            },
        )
}

// Pin the exact nodes we inspected so provider reloads cannot introduce a shared WG identity.
// Normal tests keep the original provider and its refresh behavior.
fn freeze_cached_providers(config: &mut Value, directory: &Path) -> bool {
    let Some(providers) = config
        .get_mut("proxy-providers")
        .and_then(Value::as_object_mut)
    else {
        return false;
    };
    let Ok(directory) = directory.canonicalize() else {
        return false;
    };
    let mut changed = false;
    for provider in providers.values_mut() {
        if !matches!(provider["type"].as_str(), Some("file" | "http"))
            || provider["age-secret-key"]
                .as_str()
                .is_some_and(|value| !value.is_empty())
        {
            continue;
        }
        let Some(path) = provider["path"].as_str().filter(|path| !path.is_empty()) else {
            continue;
        };
        let Ok(path) = directory.join(path).canonicalize() else {
            continue;
        };
        if !path.starts_with(&directory) {
            continue;
        }
        let Ok(bytes) = fs::read(path) else { continue };
        let Some(nodes) = parse_mihomo(&bytes)
            .ok()
            .and_then(|yaml| serde_json::to_value(yaml.get("proxies")?).ok())
            .filter(Value::is_array)
        else {
            continue;
        };
        provider["type"] = "inline".into();
        provider["payload"] = nodes;
        changed = true;
    }
    changed
}

fn json_config(core: Core, source: Value, port: u16) -> Result<Value, String> {
    if !source.is_object() {
        return Err(t::latency_config().into());
    }
    let field = |object: &Value, key: &str| config_key(core, object, key);
    let (kind, keys, mut inbound) = match core {
        Core::SingBox => (
            "type",
            &[
                "dns",
                "outbounds",
                "endpoints",
                "route",
                "certificate",
                "http_clients",
            ][..],
            json!({"type":"http","listen":"127.0.0.1","listen_port":port}),
        ),
        Core::Xray => (
            "protocol",
            &[
                "dns",
                "outbounds",
                "routing",
                "policy",
                "transport",
                "fakedns",
                "observatory",
                "burstObservatory",
            ][..],
            json!({"protocol":"http","listen":"127.0.0.1","port":port,"settings":{}}),
        ),
        Core::Mihomo => unreachable!(),
    };
    let mut config = json!({});
    for key in keys {
        if let Some(value) = source.get(field(&source, key)) {
            config[key] = value.clone();
        }
    }
    let original = source
        .get(field(&source, "inbounds"))
        .and_then(Value::as_array)
        .and_then(|items| {
            items
                .iter()
                .filter_map(|item| {
                    let kind = item.get(field(item, kind))?.as_str()?;
                    match kind.to_ascii_lowercase().as_str() {
                        "tun" => Some((0, item)),
                        "http" | "socks" | "mixed" => Some((1, item)),
                        _ => None,
                    }
                })
                .min_by_key(|(priority, _)| *priority)
        })
        .map(|(_, item)| item);
    if let Some(original) = original {
        for key in if core == Core::Xray {
            &["tag", "sniffing"][..]
        } else {
            &[
                "tag",
                "sniff",
                "sniff_override_destination",
                "domain_strategy",
            ][..]
        } {
            if let Some(value) = original.get(field(original, key)) {
                inbound[key] = value.clone();
            }
        }
    }
    config["inbounds"] = json!([inbound]);
    if core == Core::SingBox {
        // DNS and routing rules can depend on the configured Clash mode, even without a controller.
        if let Some(api) = source["experimental"]["clash_api"].as_object() {
            let mut mode = json!({});
            if let Some(default) = api.get("default_mode") {
                mode["default_mode"] = default.clone();
            }
            config["experimental"] = json!({"clash_api":mode});
        }
        // A system-mode endpoint can create an adapter independently of TUN inbounds.
        if let Some(endpoints) = config.get_mut("endpoints").and_then(Value::as_array_mut) {
            for endpoint in endpoints {
                if endpoint["type"] != "wireguard" {
                    return Err(t::latency_endpoint().into());
                }
                endpoint["system"] = false.into();
                endpoint["listen_port"] = 0.into();
            }
        }
        if let Some(outbounds) = config.get_mut("outbounds").and_then(Value::as_array_mut) {
            for outbound in outbounds {
                if outbound["type"] == "wireguard" {
                    outbound["system_interface"] = false.into();
                }
            }
        }
    } else if let Some(outbounds) = config.get_mut("outbounds").and_then(Value::as_array_mut) {
        for outbound in outbounds {
            if outbound
                .get(field(outbound, "protocol"))
                .and_then(Value::as_str)
                .is_some_and(|protocol| protocol.eq_ignore_ascii_case("wireguard"))
            {
                let settings = field(outbound, "settings");
                let key = field(&outbound[&settings], "noKernelTun");
                let settings = &mut outbound[&settings];
                if !settings.is_object() {
                    return Err(t::latency_config().into());
                }
                settings[&key] = true.into();
            }
        }
    }
    Ok(config)
}

fn mihomo_config(source: Value, port: u16) -> Result<Value, String> {
    if !source.is_object() {
        return Err(t::latency_config().into());
    }
    // Keep the fields that construct the outbound path; create all local services from scratch.
    let mut config = json!({
        "profile": {"store-fake-ip": false},
        "etag-support": false,
    });
    if let Some(selected) = source["profile"].get("store-selected") {
        config["profile"]["store-selected"] = selected.clone();
    }
    for key in [
        "proxies",
        "proxy-groups",
        "proxy-providers",
        "rule-providers",
        "rules",
        "sub-rules",
        "mode",
        "hosts",
        "dns",
        "ipv6",
        "sniffer",
        "interface-name",
        "routing-mark",
        "tcp-concurrent",
        "find-process-mode",
        "global-client-fingerprint",
        "global-ua",
        "geodata-mode",
        "geodata-loader",
        "geosite-matcher",
        "geox-url",
        "tls",
    ] {
        if let Some(value) = source.get(key) {
            config[key] = value.clone();
        }
    }
    if let Some(dns) = config.get_mut("dns").and_then(Value::as_object_mut) {
        dns.remove("listen");
    }
    let original = source["listeners"].as_array().and_then(|items| {
        items.iter().find(|item| {
            matches!(
                item["type"].as_str(),
                Some("tun" | "mixed" | "http" | "socks")
            )
        })
    });
    let mut inbound = json!({"type":"http", "listen":"127.0.0.1", "port":port});
    if source["tun"]["enable"] == true {
        inbound["name"] = "DEFAULT-TUN".into();
    } else if let Some(original) = original {
        for key in ["name", "rule", "proxy"] {
            if let Some(value) = original.get(key) {
                inbound[key] = value.clone();
            }
        }
    } else {
        let name = if source["mixed-port"].as_u64().is_some_and(|port| port != 0) {
            "DEFAULT-MIXED"
        } else if source["socks-port"].as_u64().is_some_and(|port| port != 0) {
            "DEFAULT-SOCKS"
        } else {
            "DEFAULT-HTTP"
        };
        inbound["name"] = name.into();
    }
    config["listeners"] = json!([inbound]);
    Ok(config)
}

fn request(port: u16, url: &str, timeout: Duration) -> Result<Duration, String> {
    let proxy =
        ureq::Proxy::new(&format!("http://127.0.0.1:{port}")).map_err(|_| t::latency_request())?;
    request_via(proxy, url, timeout)
}

fn request_via(proxy: ureq::Proxy, url: &str, timeout: Duration) -> Result<Duration, String> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .proxy(Some(proxy))
        .timeout_global(Some(timeout))
        .max_redirects(0)
        .http_status_as_error(false)
        .build()
        .into();
    let start = Instant::now();
    let response = agent.get(url).call().map_err(|err| match err {
        ureq::Error::Timeout(_) => t::latency_timeout(),
        _ => t::latency_request(),
    })?;
    if !response.status().is_success() {
        return Err(t::latency_status(response.status().as_u16()));
    }
    Ok(start.elapsed())
}

fn wait_for_listener(child: &JobProcess, port: u16, cancelled: &AtomicBool) -> Result<(), String> {
    // Exclude startup and rule loading from the measured latency.
    let deadline = Instant::now() + Duration::from_secs(10);
    let pid = child.id();
    loop {
        if cancelled.load(Ordering::Acquire) {
            return Err(t::latency_cancelled().into());
        }
        if child.try_wait().map_err(|_| t::latency_start())?.is_some() {
            return Err(t::latency_start().into());
        }
        if owns_address(pid, (Ipv4Addr::LOCALHOST, port).into()).map_err(|_| t::latency_start())? {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(t::latency_start_timeout().into());
        }
        thread::sleep(Duration::from_millis(25));
    }
}

// Verify the listener owner before using either a temporary or an already running core.
fn owns_address(pid: u32, address: SocketAddr) -> io::Result<bool> {
    let mut buffer = vec![0u32; 256];
    loop {
        let mut bytes = (buffer.len() * size_of::<u32>()) as u32;
        let status = unsafe {
            GetExtendedTcpTable(
                buffer.as_mut_ptr().cast(),
                &mut bytes,
                0,
                (if address.is_ipv4() { AF_INET } else { AF_INET6 }).into(),
                TCP_TABLE_OWNER_PID_LISTENER,
                0,
            )
        };
        if status == ERROR_INSUFFICIENT_BUFFER {
            buffer.resize((bytes as usize).div_ceil(size_of::<u32>()), 0);
            continue;
        }
        if status != NO_ERROR {
            return Err(io::Error::from_raw_os_error(status as i32));
        }
        // MIB_TCPTABLE_OWNER_PID is a DWORD count followed by DWORD-aligned rows.
        let count = buffer[0] as usize;
        let table = buffer[1..].as_ptr();
        return Ok(match address.ip() {
            IpAddr::V4(ip) => {
                let rows = unsafe {
                    std::slice::from_raw_parts(table.cast::<MIB_TCPROW_OWNER_PID>(), count)
                };
                let owners = rows
                    .iter()
                    .filter(|row| {
                        (row.dwLocalAddr == 0 || row.dwLocalAddr.to_ne_bytes() == ip.octets())
                            && u16::from_be(row.dwLocalPort as u16) == address.port()
                    })
                    .map(|row| row.dwOwningPid);
                only_owner(owners, pid)
            }
            IpAddr::V6(ip) => {
                let rows = unsafe {
                    std::slice::from_raw_parts(table.cast::<MIB_TCP6ROW_OWNER_PID>(), count)
                };
                let owners = rows
                    .iter()
                    .filter(|row| {
                        (row.ucLocalAddr == [0; 16] || row.ucLocalAddr == ip.octets())
                            && u16::from_be(row.dwLocalPort as u16) == address.port()
                    })
                    .map(|row| row.dwOwningPid);
                only_owner(owners, pid)
            }
        });
    }
}

fn only_owner(mut owners: impl Iterator<Item = u32>, pid: u32) -> bool {
    owners.next() == Some(pid) && owners.all(|owner| owner == pid)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cached_provider_nodes_remain_fixed_after_wireguard_conflict_check() {
        use crate::wireguard::Profile;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nodes.yaml");
        let mut config = json!({"proxy-providers":{"nodes":{
            "type":"file", "path":"nodes.yaml", "filter":"west", "health-check":{"enable":true}
        }}});
        let key = "01".repeat(32);
        let probe_profile = || Profile {
            source: "live.yaml".into(),
            exe: "mihomo.exe".into(),
            digest: [0; 32],
        };
        let p = probe_profile();
        let sessions = Sessions::default();
        let _live = sessions
            .start(
                Identities::read(
                    Core::Mihomo,
                    &json!({"proxies":[{"type":"wireguard","private-key":key}]}),
                ),
                probe_profile(),
            )
            .unwrap();
        let check = |cfg: &Value| {
            sessions.probe(
                Identities::read(Core::Mihomo, cfg),
                Path::new("other.yaml"),
                &p.exe,
                p.digest,
            )
        };
        let provider = |key: &str| {
            json!({"proxies":[{"name":"west", "type":"wireguard", "private-key":key}]}).to_string()
        };
        fs::write(&path, provider(&"02".repeat(32))).unwrap();
        assert!(matches!(check(&config), Probe::UnknownConflict));
        assert!(freeze_cached_providers(&mut config, dir.path()));
        fs::write(&path, provider(&key)).unwrap();
        assert!(matches!(check(&config), Probe::Isolated(Some(_))));
        assert_eq!(config["proxy-providers"]["nodes"]["filter"], "west");
        assert_eq!(
            config["proxy-providers"]["nodes"]["health-check"]["enable"],
            true
        );
    }

    #[test]
    fn xray_keeps_inbound_tag_and_routing_without_starting_configured_listeners() {
        let source = json!({
            "Inbounds": [
                {"Protocol":"http", "tag":"web"},
                {"Protocol":"TuN", "TaG":"vpn", "Settings":{"autoSystemWfpBlockLeak": true},
                    "sniffing": {"enabled":true}}
            ],
            "Routing":{"rules":[{"type":"field","inboundTag":["vpn"],"outboundTag":"vps"}]},
            "Outbounds":[{"Protocol":"WireGuard","tag":"vps","Settings":{"noKernelTun":false}}],
            "api":{"listen":"0.0.0.0:7777"},
            "metrics":{"listen":"0.0.0.0:8888"}
        });
        let probe = json_config(Core::Xray, source.clone(), 12345).unwrap();
        assert_eq!(
            probe["inbounds"],
            json!([{"protocol":"http","listen":"127.0.0.1","port":12345,
            "settings":{},"tag":"vpn","sniffing":{"enabled":true}}])
        );
        assert_eq!(probe["routing"], source["Routing"]);
        assert_eq!(probe["outbounds"][0]["Settings"]["noKernelTun"], true);
        assert!(probe.get("api").is_none() && probe.get("metrics").is_none());
    }

    #[test]
    fn sing_box_disables_system_wireguard_and_removes_services() {
        let source = json!({
            "inbounds":[{"type":"tun","tag":"vpn","auto_route":true,"strict_route":true}],
            "endpoints":[{"type":"wireguard","tag":"peer","system":true,"listen_port":1234}],
            "outbounds":[{"type":"wireguard","system_interface":true}],
            "route":{"final":"peer"}, "experimental":{"cache_file":{"enabled":true},
                "clash_api":{"default_mode":"Global","external_controller":"0.0.0.0:9000"}},
            "services":[{"type":"resolved"}]
        });
        let probe = json_config(Core::SingBox, source, 23456).unwrap();
        assert_eq!(
            probe["inbounds"],
            json!([{"type":"http","listen":"127.0.0.1","listen_port":23456,"tag":"vpn"}])
        );
        assert_eq!(probe["endpoints"][0]["system"], false);
        assert_eq!(probe["endpoints"][0]["listen_port"], 0);
        assert_eq!(probe["outbounds"][0]["system_interface"], false);
        assert_eq!(probe["route"]["final"], "peer");
        assert_eq!(
            probe["experimental"],
            json!({"clash_api":{"default_mode":"Global"}})
        );
        assert!(probe.get("services").is_none());
    }

    #[test]
    fn mihomo_keeps_outbound_rules_and_drops_dns_and_management_listeners() {
        let source = json!({
            "tun":{"enable":true,"strict-route":true}, "mixed-port":7890, "tproxy-port":7891,
            "dns":{"enable":true,"listen":"0.0.0.0:53","nameserver":["system"]},
            "listeners":[{"type":"socks", "name":"local", "port":9999}],
            "external-controller":"0.0.0.0:7878", "tunnels":["tcp,0.0.0.0:54,example.test:53"],
            "rules":["MATCH,DIRECT"]
        });
        let probe = mihomo_config(source.clone(), 12345).unwrap();
        assert_eq!(probe["rules"], source["rules"]);
        assert!(probe["profile"].get("store-selected").is_none());
        for enabled in [false, true] {
            let probe =
                mihomo_config(json!({"profile":{"store-selected":enabled}}), 12345).unwrap();
            assert_eq!(probe["profile"]["store-selected"], enabled);
        }
        assert_eq!(probe["dns"], json!({"enable":true,"nameserver":["system"]}));
        assert_eq!(
            probe["listeners"],
            json!([{"name":"DEFAULT-TUN","type":"http","listen":"127.0.0.1","port":12345}])
        );
        for key in [
            "tun",
            "mixed-port",
            "tproxy-port",
            "external-controller",
            "tunnels",
        ] {
            assert!(probe.get(key).is_none());
        }
    }

    #[test]
    fn malformed_inputs_do_not_panic_in_the_probe_worker() {
        for source in [Value::Null, json!(17), json!("text")] {
            assert!(json_config(Core::Xray, source.clone(), 1).is_err());
            assert!(mihomo_config(source, 1).is_err());
        }
        for core in [Core::Xray, Core::SingBox] {
            let _ = json_config(core, json!({"outbounds":[null,1,"x"],"inbounds":[1]}), 1);
        }
        for endpoint in [Value::Null, json!("invalid"), json!({"type":"tailscale"})] {
            assert!(json_config(Core::SingBox, json!({"endpoints":[endpoint]}), 1).is_err());
        }
        assert!(
            json_config(
                Core::Xray,
                json!({"outbounds":[{"protocol":"wireguard","settings":3}]}),
                1
            )
            .is_err()
        );
        let _ = mihomo_config(json!({"listeners":[1,"",{}],"tun":false,"dns":0}), 1);
    }
}
