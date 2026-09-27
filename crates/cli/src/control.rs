//! A presentation client: it has no clipboard, database, or peer network access.
use crate::Output;
use shuttli_api::control::*;
use shuttli_model::sync::{HistoryMode, PeerPolicy, Settings};
pub fn run(args: &[String], api: &mut impl ControlApi) -> Output {
    let mut args = args.to_vec();
    let json = args.iter().any(|a| a == "--json");
    args.retain(|a| a != "--json");
    let a: Vec<_> = args.iter().map(String::as_str).collect();
    let action = match parse(&a, api) {
        Ok(Some(a)) => a,
        Ok(None) => {
            return Output {
                exit_code: 0,
                text: format!("{}\n{HELP}", shuttli_brand::NAME),
            };
        }
        Err(e) => {
            return Output {
                exit_code: 2,
                text: if json {
                    format!(
                        "{}\n",
                        serde_json::to_string(&Answer::Error { message: e }).unwrap_or_default()
                    )
                } else {
                    format!("{e}\n")
                },
            };
        }
    };
    let answer = api.request(ControlRequest {
        version: VERSION,
        action,
    });
    let exit_code = if matches!(answer, Answer::Error { .. } | Answer::Stopped) {
        1
    } else {
        0
    };
    let text = if json {
        serde_json::to_string(&answer).unwrap_or_default()
    } else {
        match &answer {
            Answer::Stopped => "Agent has stopped; start `shuttli daemon` or `shuttli ui`".into(),
            Answer::Autostart { status } => status.message.clone(),
            Answer::Done { message } | Answer::Error { message } => message.clone(),
            Answer::Status { status } => format!(
                "Device: {}\nClipboard: {} ({})\nSend: {} | Receive: {} | Automatic: {}\nLast error: {}",
                status.device,
                status.clipboard,
                if status.clipboard_available {
                    "available"
                } else {
                    "unavailable"
                },
                status.settings.send,
                status.settings.receive,
                status.settings.automatic,
                status.last_error.as_deref().unwrap_or("none")
            ),
            Answer::Devices { devices, settings } => devices
                .iter()
                .map(|p| {
                    let policy = settings.peers.get(&p.id).cloned().unwrap_or_default();
                    format!(
                        "{}\n  {} {} | send={} receive={}",
                        p.id, p.name, p.address, policy.send, policy.receive
                    )
                })
                .collect::<Vec<_>>()
                .join("\n"),
            Answer::History { entries } => entries
                .iter()
                .map(|h| {
                    format!(
                        "#{} {} {} {} {} bytes | content={} | {}",
                        h.id,
                        h.direction,
                        if h.direction == "local" {
                            "Copied locally".into()
                        } else {
                            format!("{:?}", h.state)
                        },
                        match h.format {
                            shuttli_model::sync::Format::Text => "Text",
                            shuttli_model::sync::Format::Png => "Image",
                        },
                        h.bytes,
                        h.available,
                        h.detail
                    )
                })
                .collect::<Vec<_>>()
                .join("\n"),
            _ => serde_json::to_string_pretty(&answer).unwrap_or_default(),
        }
    };
    Output {
        exit_code,
        text: format!("{text}\n"),
    }
}
fn settings(api: &mut impl ControlApi) -> Result<Settings, String> {
    match api.request(ControlRequest {
        version: VERSION,
        action: Action::Settings,
    }) {
        Answer::Settings { settings } => Ok(settings),
        Answer::Error { message } => Err(message),
        _ => Err("invalid settings response".into()),
    }
}
fn switch(s: &str) -> Result<bool, String> {
    match s {
        "on" => Ok(true),
        "off" => Ok(false),
        _ => Err("expected on or off".into()),
    }
}
fn id(s: &str) -> Result<i64, String> {
    s.parse().map_err(|_| "invalid history ID".into())
}
fn parse(a: &[&str], api: &mut impl ControlApi) -> Result<Option<Action>, String> {
    Ok(Some(match a {
        [] | ["help"] | ["--help"] | ["-h"] => return Ok(None),
        ["status"] => Action::Status,
        ["quit"] => Action::Quit,
        ["devices"] => Action::Devices,
        ["settings"] => Action::Settings,
        ["refresh"] => Action::Refresh,
        ["send"] => Action::Send,
        ["set", "send", value] => Action::SetDirections {
            send: Some(switch(value)?),
            receive: None,
        },
        ["set", "receive", value] => Action::SetDirections {
            send: None,
            receive: Some(switch(value)?),
        },
        ["history"] => Action::History {
            offset: 0,
            limit: 50,
        },
        ["history", "clear"] => Action::ClearHistory,
        ["history", "--offset", n] => Action::History {
            offset: n.parse().map_err(|_| "invalid offset")?,
            limit: 50,
        },
        ["history", "preview", n] => Action::Preview { id: id(n)? },
        ["history", "resend", n] => Action::Resend { id: id(n)? },
        ["history", "copy", n] => Action::Copy {
            id: id(n)?,
            local_only: false,
        },
        ["history", "copy", n, "--local-only"] => Action::Copy {
            id: id(n)?,
            local_only: true,
        },
        ["autostart", "status"] => Action::Autostart { enabled: None },
        ["autostart", s] => Action::Autostart {
            enabled: Some(switch(s)?),
        },
        [
            "set",
            key @ ("history-limit" | "history-memory-mib" | "history-image-mib"),
            value,
        ] => {
            let mut s = settings(api)?;
            let expected = s.clone();
            let n: u64 = value
                .parse()
                .map_err(|_| "expected a nonnegative integer")?;
            match *key {
                "history-limit" => {
                    s.history_limit = usize::try_from(n).map_err(|_| "history limit too large")?
                }
                "history-memory-mib" => {
                    s.history_memory_bytes = n
                        .checked_mul(1024 * 1024)
                        .ok_or("memory budget too large")?
                }
                _ => {
                    s.history_bytes = n.checked_mul(1024 * 1024).ok_or("image budget too large")?
                }
            }
            if !s.validate() {
                return Err(
                    "history limit must be 0–10000; memory 0–64 MiB; images 0–1024 MiB".into(),
                );
            }
            Action::Configure {
                expected,
                settings: s,
            }
        }
        ["set", "history", mode] => {
            let mut s = settings(api)?;
            let expected = s.clone();
            s.history = match *mode {
                "off" => HistoryMode::Off,
                "status" => HistoryMode::Status,
                "content" => HistoryMode::Content,
                _ => return Err("expected off, status or content".into()),
            };
            Action::Configure {
                expected,
                settings: s,
            }
        }
        ["set", key, value] => {
            let mut s = settings(api)?;
            let expected = s.clone();
            let v = switch(value)?;
            match *key {
                "send" => s.send = v,
                "receive" => s.receive = v,
                "automatic" => s.automatic = v,
                "text" => s.text = v,
                "images" | "png" => s.png = v,
                "notifications" => s.notifications = v,
                _ => return Err("unknown setting".into()),
            }
            Action::Configure {
                expected,
                settings: s,
            }
        }
        ["peer", id, "history", mode] => {
            let s = settings(api)?;
            let mut p = s.peers.get(*id).cloned().unwrap_or_default();
            let expected = p.clone();
            p.history = match *mode {
                "inherit" => None,
                "off" => Some(HistoryMode::Off),
                "status" => Some(HistoryMode::Status),
                "content" => Some(HistoryMode::Content),
                _ => return Err("expected inherit, off, status or content".into()),
            };
            Action::Peer {
                id: (*id).into(),
                expected,
                policy: p,
            }
        }
        ["peer", id, key, value] => {
            let s = settings(api)?;
            let mut p = s
                .peers
                .get(*id)
                .cloned()
                .unwrap_or_else(PeerPolicy::default);
            let expected = p.clone();
            let v = switch(value)?;
            match *key {
                "send" => p.send = v,
                "receive" => p.receive = v,
                "text" => p.text = v,
                "images" | "png" => p.png = v,
                "quiet" => p.quiet = v,
                _ => return Err("unknown peer setting".into()),
            }
            Action::Peer {
                id: (*id).into(),
                expected,
                policy: p,
            }
        }
        ["request", body] => {
            return serde_json::from_str::<ControlRequest>(body)
                .and_then(|r| {
                    if r.version == VERSION {
                        Ok(r)
                    } else {
                        Err(serde_json::Error::io(std::io::Error::new(
                            std::io::ErrorKind::InvalidInput,
                            "unsupported API version",
                        )))
                    }
                })
                .map(|r| Some(r.action))
                .map_err(|_| "invalid request JSON".into());
        }
        _ => return Err("Unsupported command; use --help".into()),
    }))
}
pub const HELP: &str = "Usage: shuttli daemon | ui | status | devices | refresh | send | quit\n  settings | set <send|receive|automatic|text|images|notifications> <on|off>\n  set history <off|status|content>\n  set history-limit <0..10000>\n  set history-memory-mib <0..64> | set history-image-mib <0..1024>\n  peer <full fingerprint> <send|receive|text|images|quiet> <on|off>\n  history [preview|copy|resend <id>] | history clear\n  history copy <id> --local-only\n  autostart <status|on|off>\nAppend --json for the versioned local API response.\n";

#[cfg(test)]
mod tests {
    use super::*;
    struct Api {
        configured: Option<Settings>,
    }
    impl ControlApi for Api {
        fn request(&mut self, request: ControlRequest) -> Answer {
            match request.action {
                Action::Settings => Answer::Settings {
                    settings: Settings::default(),
                },
                Action::Configure { settings, .. } => {
                    self.configured = Some(settings.clone());
                    Answer::Settings { settings }
                }
                _ => panic!("unexpected action"),
            }
        }
    }
    #[test]
    fn retention_limit_is_validated_before_configuration_and_json_errors_are_readable() {
        let mut api = Api { configured: None };
        let args = ["set", "history-limit", "3", "--json"].map(str::to_string);
        assert_eq!(run(&args, &mut api).exit_code, 0);
        assert_eq!(api.configured.take().unwrap().history_limit, 3);
        for value in ["-1", "10001", "18446744073709551616", "oops"] {
            let args = ["set", "history-limit", value, "--json"].map(str::to_string);
            let output = run(&args, &mut api);
            assert_ne!(output.exit_code, 0);
            assert!(matches!(
                serde_json::from_str::<Answer>(&output.text).unwrap(),
                Answer::Error { .. }
            ));
            assert!(api.configured.is_none());
        }
    }
    #[test]
    fn direction_cli_uses_one_atomic_request_without_reading_settings() {
        struct DirectionApi;
        impl ControlApi for DirectionApi {
            fn request(&mut self, request: ControlRequest) -> Answer {
                assert!(matches!(
                    request.action,
                    Action::SetDirections {
                        send: Some(false),
                        receive: None
                    }
                ));
                Answer::Settings {
                    settings: Settings::default(),
                }
            }
        }
        let args = ["set", "send", "off", "--json"].map(str::to_string);
        assert_eq!(run(&args, &mut DirectionApi).exit_code, 0);
    }
}
