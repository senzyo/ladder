use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

use serde_json::Value;
use ureq::{Proxy, ProxyProtocol};

use crate::{core::Core, tun_config::config_key};

pub(crate) struct Candidate {
    pub(crate) address: SocketAddr,
    pub(crate) proxy: Proxy,
}

// The caller verifies that the running core owns this listener before connecting.
pub(crate) fn candidates(core: Core, source: &Value) -> Vec<Candidate> {
    let mut result = Vec::new();
    match core {
        Core::Mihomo => mihomo_listeners(source, &mut result),
        Core::SingBox => {
            for inbound in source["inbounds"].as_array().into_iter().flatten() {
                let Some(protocol) = inbound["type"].as_str().and_then(proxy_protocol) else {
                    continue;
                };
                if inbound["tls"]["enabled"] == true {
                    continue;
                }
                let listen = inbound["listen"].as_str().unwrap_or("*");
                let auth = first_account(core, &inbound["users"], "username", "password");
                add_listener(&mut result, listen, &inbound["listen_port"], protocol, auth);
            }
        }
        Core::Xray => {
            for inbound in xray_field(source, "inbounds")
                .as_array()
                .into_iter()
                .flatten()
            {
                xray_listener(inbound, &mut result);
            }
        }
    }
    result.sort_by_key(|candidate| candidate.proxy.protocol() == ProxyProtocol::Socks5h);
    result
}

fn mihomo_listeners(source: &Value, result: &mut Vec<Candidate>) {
    let auth = source["authentication"].as_array().and_then(|users| {
        users
            .iter()
            .filter_map(Value::as_str)
            .find_map(|user| user.split_once(':'))
    });
    let listen = if source["allow-lan"] == true {
        source["bind-address"].as_str().unwrap_or("*")
    } else {
        "127.0.0.1"
    };
    for (key, protocol) in [
        ("mixed-port", ProxyProtocol::Http),
        ("port", ProxyProtocol::Http),
        ("socks-port", ProxyProtocol::Socks5h),
    ] {
        add_listener(result, listen, &source[key], protocol, auth);
    }
    for inbound in source["listeners"].as_array().into_iter().flatten() {
        let Some(protocol) = inbound["type"].as_str().and_then(proxy_protocol) else {
            continue;
        };
        if !inbound["certificate"].as_str().unwrap_or("").is_empty()
            || !inbound["reality-config"]["private-key"]
                .as_str()
                .unwrap_or("")
                .is_empty()
        {
            continue;
        }
        let users = if inbound["users"].is_null() {
            auth
        } else {
            first_account(Core::Mihomo, &inbound["users"], "username", "password")
        };
        let listen = inbound["listen"].as_str().unwrap_or("*");
        add_listener(result, listen, &inbound["port"], protocol, users);
    }
}

fn xray_field<'a>(object: &'a Value, key: &str) -> &'a Value {
    &object[config_key(Core::Xray, object, key)]
}

fn xray_listener(inbound: &Value, result: &mut Vec<Candidate>) {
    let kind = xray_field(inbound, "protocol").as_str().unwrap_or("");
    let Some(protocol) = proxy_protocol(kind) else {
        return;
    };
    let stream = xray_field(inbound, "streamSettings");
    let network = xray_field(stream, "network").as_str().unwrap_or("tcp");
    let security = xray_field(stream, "security").as_str().unwrap_or("none");
    if !matches!(network, "tcp" | "raw" | "") || !matches!(security, "none" | "") {
        return;
    }
    let raw = xray_field(stream, "rawSettings");
    let tcp = if raw.is_null() {
        xray_field(stream, "tcpSettings")
    } else {
        raw
    };
    let header = xray_field(tcp, "header");
    let socket = xray_field(stream, "sockopt");
    if xray_field(header, "type")
        .as_str()
        .is_some_and(|name| name != "none")
        || xray_field(tcp, "acceptProxyProtocol") == true
        || xray_field(socket, "acceptProxyProtocol") == true
    {
        return;
    }
    let settings = xray_field(inbound, "settings");
    let accounts = xray_field(settings, "accounts");
    let users = if accounts.is_null() {
        xray_field(settings, "users")
    } else {
        accounts
    };
    let auth = if kind.eq_ignore_ascii_case("http") || xray_field(settings, "auth") == "password" {
        first_account(Core::Xray, users, "user", "pass")
    } else {
        None
    };
    let listen = xray_field(inbound, "listen").as_str().unwrap_or("*");
    add_listener(result, listen, xray_field(inbound, "port"), protocol, auth);
}

fn first_account<'a>(
    core: Core,
    users: &'a Value,
    username: &str,
    password: &str,
) -> Option<(&'a str, &'a str)> {
    users.as_array()?.iter().find_map(|user| {
        Some((
            user[config_key(core, user, username)].as_str()?,
            user[config_key(core, user, password)]
                .as_str()
                .unwrap_or(""),
        ))
    })
}

fn proxy_protocol(kind: &str) -> Option<ProxyProtocol> {
    match kind.to_ascii_lowercase().as_str() {
        "http" | "mixed" => Some(ProxyProtocol::Http),
        "socks" => Some(ProxyProtocol::Socks5h),
        _ => None,
    }
}

fn add_listener(
    result: &mut Vec<Candidate>,
    listen: &str,
    port: &Value,
    protocol: ProxyProtocol,
    auth: Option<(&str, &str)>,
) {
    let port = port
        .as_u64()
        .and_then(|n| u16::try_from(n).ok())
        .or_else(|| port.as_str()?.parse::<u16>().ok());
    let Some(port) = port.filter(|&port| port > 0) else {
        return;
    };
    let listen = listen.trim_start_matches('[').trim_end_matches(']');
    let v4 = IpAddr::V4(Ipv4Addr::LOCALHOST);
    let v6 = IpAddr::V6(Ipv6Addr::LOCALHOST);
    let addresses = match listen.parse::<IpAddr>() {
        Ok(ip) if ip.is_loopback() => vec![ip],
        Ok(IpAddr::V6(ip)) if ip.is_unspecified() => vec![v6],
        Ok(IpAddr::V4(ip)) if ip.is_unspecified() => vec![v4, v6],
        _ if matches!(listen, "" | "*") || listen.eq_ignore_ascii_case("localhost") => vec![v4, v6],
        _ => return,
    };
    for ip in addresses {
        let address = SocketAddr::new(ip, port);
        let host = match ip {
            IpAddr::V4(ip) => ip.to_string(),
            IpAddr::V6(ip) => format!("[{ip}]"),
        };
        let mut builder = Proxy::builder(protocol).host(&host).port(port);
        if let Some((user, pass)) = auth {
            builder = builder.username(user).password(pass);
        }
        if let Ok(proxy) = builder.build() {
            result.push(Candidate { address, proxy });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn mihomo_uses_global_or_listener_auth_and_honors_allow_lan() {
        let source = json!({
            "bind-address":"192.0.2.1", "port":7890, "authentication":["first:p@ss:word"],
            "listeners":[
                {"type":"mixed","listen":"::1","port":8801},
                {"type":"http","listen":"127.0.0.2","port":8802,"users":[]},
                {"type":"socks","listen":"127.0.0.1","port":1080,"users":[{"username":"named","password":"demo"}]},
                {"type":"http","listen":"127.0.0.1","port":8803,"certificate":"fixture"}
            ]
        });
        let proxies = candidates(Core::Mihomo, &source);
        assert_eq!(proxies.len(), 4);
        assert_eq!(proxies[0].address.to_string(), "127.0.0.1:7890");
        assert_eq!(proxies[0].proxy.password(), Some("p@ss:word"));
        assert_eq!(proxies[1].address.to_string(), "[::1]:8801");
        assert_eq!(proxies[1].proxy.username(), Some("first"));
        assert_eq!(proxies[2].proxy.username(), None);
        assert_eq!(proxies[3].proxy.username(), Some("named"));
        assert!(!proxies[3].proxy.resolve_target());
    }

    #[test]
    fn sing_box_prefers_plain_http_and_leaves_socks_dns_to_the_core() {
        let proxies = candidates(
            Core::SingBox,
            &json!({"inbounds":[
                {"type":"socks","listen":"127.0.0.1","listen_port":1080,"users":[{"username":"one","password":"two"}]},
                {"type":"mixed","listen":"::","listen_port":7890},
                {"type":"http","listen":"::1","listen_port":1234,"tls":{"enabled":true}},
                {"type":"http","listen":"192.0.2.1","listen_port":1235},
                {"type":"tun","interface_name":"unused"}
            ]}),
        );
        assert_eq!(proxies.len(), 2);
        assert_eq!(proxies[0].address.to_string(), "[::1]:7890");
        assert_eq!(proxies[0].proxy.protocol(), ProxyProtocol::Http);
        assert_eq!(proxies[1].proxy.protocol(), ProxyProtocol::Socks5h);
        assert_eq!(proxies[1].proxy.username(), Some("one"));
        assert!(!proxies[1].proxy.resolve_target());
    }

    #[test]
    fn xray_honors_accounts_alias_auth_and_stream_settings() {
        let proxies = candidates(
            Core::Xray,
            &json!({"InboundS":[
                {"protocol":"socks","listen":"127.0.0.1","port":1080,"settings":{"auth":"noauth","users":[{"user":"unused","pass":"unused"}]}},
                {"Protocol":"mixed","Listen":"127.0.0.1","Port":"7890","Settings":{"Auth":"password","Users":[{"user":"old"}],"Accounts":[{"USER":"fixture","PASS":"p:ä%"}]}},
                {"protocol":"http","port":1081,"streamSettings":{"network":"ws"}},
                {"protocol":"http","port":1082,"streamSettings":{"security":"tls"}},
                {"protocol":"http","port":1083,"streamSettings":{"tcpSettings":{"header":{"type":"http"}}}},
                {"protocol":"http","port":1084,"streamSettings":{"sockopt":{"acceptProxyProtocol":true}}}
            ]}),
        );
        assert_eq!(proxies.len(), 2);
        assert_eq!(proxies[0].proxy.username(), Some("fixture"));
        assert_eq!(proxies[0].proxy.password(), Some("p:ä%"));
        assert_eq!(proxies[1].proxy.username(), None);
        assert_eq!(proxies[1].address.port(), 1080);
    }
}
