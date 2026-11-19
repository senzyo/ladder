use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, RwLock, RwLockReadGuard};

use base64::{
    Engine as _,
    engine::general_purpose::{STANDARD_NO_PAD, URL_SAFE_NO_PAD},
};
use serde_json::Value;

use crate::core::Core;
use crate::tun_config::config_key;

/// An unknown identity (for example a subscription that may change) can overlap any WG key.
#[derive(Default)]
pub(crate) struct Identities(Vec<Option<[u8; 32]>>);

impl Identities {
    pub(crate) fn unknown() -> Self {
        Self(vec![None])
    }

    pub(crate) fn read(core: Core, config: &Value) -> Self {
        Self::from_config(core, config, true)
    }

    pub(crate) fn probe_keys(core: Core, config: &Value) -> Self {
        Self::from_config(core, config, false)
    }

    fn from_config(core: Core, config: &Value, live: bool) -> Self {
        let mut ids = Self::default();
        let collect = |ids: &mut Self, items: &Value| {
            for item in items.as_array().into_iter().flatten() {
                let (protocol, key) = match core {
                    Core::Mihomo => (&item["type"], &item["private-key"]),
                    Core::SingBox => (&item["type"], &item["private_key"]),
                    Core::Xray => {
                        let settings = &item[config_key(core, item, "settings")];
                        (
                            &item[config_key(core, item, "protocol")],
                            &settings[config_key(core, settings, "secretKey")],
                        )
                    }
                };
                if protocol
                    .as_str()
                    .is_some_and(|s| s.eq_ignore_ascii_case("wireguard"))
                {
                    ids.0.push(key.as_str().and_then(private_key));
                }
            }
        };
        match core {
            Core::Mihomo => {
                collect(&mut ids, &config["proxies"]);
                // The controller can replace the running configuration without touching its file.
                if live
                    && [
                        "external-controller",
                        "external-controller-tls",
                        "external-controller-pipe",
                        "external-controller-unix",
                    ]
                    .iter()
                    .any(|key| config[*key].as_str().is_some_and(|value| !value.is_empty()))
                {
                    ids.0.push(None);
                }
                if let Some(providers) = config["proxy-providers"].as_object() {
                    for provider in providers.values() {
                        if !provider["override"]["override-expr"].is_null() {
                            ids.0.push(None);
                            continue;
                        }
                        if provider["exclude-type"].as_str().is_some_and(|types| {
                            types
                                .split('|')
                                .any(|ty| ty.eq_ignore_ascii_case("wireguard"))
                        }) {
                            continue;
                        }
                        if provider["type"] != "inline" {
                            ids.0.push(None);
                            continue;
                        }
                        collect(&mut ids, &provider["payload"]);
                    }
                }
            }
            Core::SingBox => {
                collect(&mut ids, &config["endpoints"]);
                collect(&mut ids, &config["outbounds"]);
            }
            Core::Xray => {
                if live {
                    collect(&mut ids, &config[config_key(core, config, "inbounds")]);
                    let api = &config[config_key(core, config, "api")];
                    if api[config_key(core, api, "services")]
                        .as_array()
                        .is_some_and(|services| {
                            services.iter().any(|s| {
                                s.as_str()
                                    .is_some_and(|s| s.eq_ignore_ascii_case("HandlerService"))
                            })
                        })
                    {
                        ids.0.push(None);
                    }
                }
                collect(&mut ids, &config[config_key(core, config, "outbounds")]);
            }
        }
        ids
    }

    fn overlaps(&self, other: &Self) -> bool {
        self.0
            .iter()
            .any(|a| other.0.iter().any(|b| a.is_none() || b.is_none() || a == b))
    }

    fn shares_key(&self, other: &Self) -> bool {
        self.0
            .iter()
            .flatten()
            .any(|key| other.0.contains(&Some(*key)))
    }
}

fn private_key(text: &str) -> Option<[u8; 32]> {
    let text = text.trim().trim_end_matches('=');
    let mut key = [0; 32];
    if text.len() == 64 {
        for (i, byte) in key.iter_mut().enumerate() {
            *byte = u8::from_str_radix(text.get(2 * i..2 * i + 2)?, 16).ok()?;
        }
    } else {
        let len = STANDARD_NO_PAD
            .decode_slice(text, &mut key)
            .or_else(|_| URL_SAFE_NO_PAD.decode_slice(text, &mut key))
            .ok()?;
        if len != key.len() {
            return None;
        }
    }
    // X25519 ignores these scalar bits. Different encodings can identify the same peer.
    key[0] &= 248;
    key[31] &= 127;
    key[31] |= 64;
    Some(key)
}

#[derive(PartialEq, Eq)]
pub(crate) struct Profile {
    pub(crate) source: PathBuf,
    pub(crate) exe: PathBuf,
    pub(crate) digest: [u8; 32],
}

struct Session {
    ids: Identities,
    live: Option<(Profile, RwLock<Option<u32>>)>,
}

#[derive(Clone, Default)]
pub(crate) struct Sessions(Arc<Registry>);

#[derive(Default)]
struct Registry {
    active: Mutex<Vec<Arc<Session>>>,
    finished: Condvar,
}

pub(crate) struct Lease {
    session: Arc<Session>,
    registry: Arc<Registry>,
}

impl Lease {
    pub(crate) fn set_pid(&self, pid: Option<u32>) {
        if let Some((_, current)) = &self.session.live {
            *current.write().unwrap() = pid;
        }
    }
}

impl Drop for Lease {
    fn drop(&mut self) {
        self.set_pid(None);
        self.registry
            .active
            .lock()
            .unwrap()
            .retain(|s| !Arc::ptr_eq(s, &self.session));
        self.registry.finished.notify_all();
    }
}

pub(crate) enum Probe {
    Isolated(Option<Lease>),
    Online(Online),
    Conflict,
    UnknownConflict,
}

pub(crate) struct Online(Arc<Session>);

impl Online {
    pub(crate) fn pid(&self) -> RwLockReadGuard<'_, Option<u32>> {
        self.0.live.as_ref().unwrap().1.read().unwrap()
    }
}

impl Sessions {
    pub(crate) fn start(&self, ids: Identities, profile: Profile) -> Option<Lease> {
        if ids.0.is_empty() {
            return None;
        }
        let session = Arc::new(Session {
            ids,
            live: Some((profile, RwLock::new(None))),
        });
        let mut active = self.0.active.lock().unwrap();
        // Register before waiting so new probes cannot delay an explicit start.
        active.push(session.clone());
        drop(
            self.0
                .finished
                .wait_while(active, |active| {
                    active
                        .iter()
                        .any(|other| other.live.is_none() && other.ids.overlaps(&session.ids))
                })
                .unwrap(),
        );
        Some(Lease {
            session,
            registry: self.0.clone(),
        })
    }

    pub(crate) fn probe(
        &self,
        ids: Identities,
        source: &Path,
        exe: &Path,
        digest: [u8; 32],
    ) -> Probe {
        if ids.0.is_empty() {
            return Probe::Isolated(None);
        }
        let mut active = self.0.active.lock().unwrap();
        if let Some(session) = active.iter().find(|s| {
            s.live
                .as_ref()
                .is_some_and(|(p, _)| p.source == source && p.exe == exe && p.digest == digest)
        }) {
            return Probe::Online(Online(session.clone()));
        }
        if active.iter().any(|s| ids.shares_key(&s.ids)) {
            return Probe::Conflict;
        }
        if active.iter().any(|s| ids.overlaps(&s.ids)) {
            return Probe::UnknownConflict;
        }
        let session = Arc::new(Session { ids, live: None });
        active.push(session.clone());
        Probe::Isolated(Some(Lease {
            session,
            registry: self.0.clone(),
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::mpsc;
    use std::time::Duration;

    fn key(core: Core, n: u8) -> Identities {
        let mut bytes = [n; 32];
        let config = match core {
            Core::Mihomo => {
                json!({"proxies":[{"type":"wireguard", "private-key":STANDARD_NO_PAD.encode(bytes)}]})
            }
            Core::SingBox => {
                json!({"endpoints":[{"type":"wireguard", "private_key":format!("{}=",STANDARD_NO_PAD.encode(bytes))}]})
            }
            Core::Xray => {
                bytes[0] ^= 7;
                bytes[31] ^= 192;
                let hex: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
                json!({"Outbounds":[{"Protocol":"WireGuard","Settings":{"SecretKey":hex}}]})
            }
        };
        Identities::read(core, &config)
    }

    fn profile() -> Profile {
        Profile {
            source: "node.json".into(),
            exe: "core".into(),
            digest: [1; 32],
        }
    }

    #[test]
    fn matching_live_profile_is_reused_and_shared_wireguard_identities_cannot_be_probed() {
        let sessions = Sessions::default();
        let p = profile();
        let live = sessions.start(key(Core::SingBox, 1), profile()).unwrap();
        live.set_pid(Some(7));
        let Probe::Online(online) =
            sessions.probe(key(Core::SingBox, 1), &p.source, &p.exe, p.digest)
        else {
            panic!("did not reuse exact running profile")
        };
        assert_eq!(*online.pid(), Some(7));
        let copy = Path::new("copy.yaml");
        assert!(matches!(
            sessions.probe(key(Core::Mihomo, 1), copy, &p.exe, p.digest),
            Probe::Conflict
        ));
        assert!(matches!(
            sessions.probe(key(Core::Xray, 1), &p.source, &p.exe, [2; 32]),
            Probe::Conflict
        ));
        assert!(matches!(
            sessions.probe(key(Core::Mihomo, 2), copy, &p.exe, p.digest),
            Probe::Isolated(Some(_))
        ));
        let remote = Identities::read(
            Core::Mihomo,
            &json!({"proxy-providers":{"sub":{"type":"http"}}}),
        );
        assert!(matches!(
            sessions.probe(remote, copy, &p.exe, p.digest),
            Probe::UnknownConflict
        ));
        assert!(matches!(
            sessions.probe(Identities::default(), copy, &p.exe, p.digest),
            Probe::Isolated(None)
        ));
        drop(live);
        assert_eq!(*online.pid(), None);
        assert!(matches!(
            sessions.probe(key(Core::Mihomo, 1), copy, &p.exe, p.digest),
            Probe::Isolated(Some(_))
        ));
    }

    #[test]
    fn live_controller_can_add_wireguard_without_rewriting_the_source_file() {
        let source = json!({"external-controller-pipe":"pipe", "proxies":[{"type":"wireguard", "private-key": STANDARD_NO_PAD.encode([3;32])}]});
        let sessions = Sessions::default();
        let p = profile();
        let _live = sessions
            .start(Identities::read(Core::Mihomo, &source), profile())
            .unwrap();
        let check = |ids| sessions.probe(ids, Path::new("other.json"), &p.exe, p.digest);
        assert!(matches!(check(key(Core::Xray, 3)), Probe::Conflict));
        assert!(matches!(
            check(key(Core::SingBox, 4)),
            Probe::UnknownConflict
        ));
        assert!(matches!(
            check(Identities::default()),
            Probe::Isolated(None)
        ));
        assert!(matches!(
            sessions.probe(
                Identities::probe_keys(Core::Mihomo, &source),
                &p.source,
                &p.exe,
                p.digest
            ),
            Probe::Online(_)
        ));
        assert!(
            !Identities::read(Core::Xray, &json!({"API":{"Services":["HandlerService"]}}))
                .0
                .is_empty()
        );
        assert!(
            Identities::read(Core::Xray, &json!({"api":{"services":["StatsService"]}}))
                .0
                .is_empty()
        );
    }

    #[test]
    fn a_live_start_waits_for_one_conflicting_probe() {
        let sessions = Sessions::default();
        let p = profile();
        let Probe::Isolated(Some(probe)) =
            sessions.probe(key(Core::Xray, 1), &p.source, &p.exe, p.digest)
        else {
            panic!()
        };
        let (tx, rx) = mpsc::channel();
        std::thread::scope(|scope| {
            scope.spawn(|| {
                let live = sessions.start(key(Core::Mihomo, 1), profile()).unwrap();
                tx.send(live).unwrap();
            });
            let deadline = std::time::Instant::now() + Duration::from_secs(2);
            while sessions.0.active.lock().unwrap().len() != 2 {
                assert!(std::time::Instant::now() < deadline);
                std::thread::yield_now();
            }
            assert!(rx.try_recv().is_err());
            drop(probe);
            assert!(rx.recv_timeout(Duration::from_secs(2)).is_ok());
        });
    }
}
