//! CLI presentation depends only on the public application contract.

use shuttli_api::{ApiError, ApplicationApi, Command, ObservedAutostart, Request, Response};

#[derive(Debug, PartialEq, Eq)]
pub struct Output {
    pub exit_code: u8,
    pub text: String,
}

pub fn run(args: &[String], app: &mut impl ApplicationApi) -> Output {
    let command = match args
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .as_slice()
    {
        [] | ["help"] | ["--help"] | ["-h"] => {
            return Output {
                exit_code: 0,
                text: format!(
                    "{} foundation build\nUsage: shuttli status | autostart status\nSync and OS startup registration are not implemented.\n",
                    shuttli_brand::NAME
                ),
            };
        }
        ["status"] => Command::Status,
        ["autostart", "status"] => Command::GetAutostartStatus,
        _ => {
            return Output {
                exit_code: 2,
                text: "Unsupported command; use --help.\n".into(),
            };
        }
    };
    match app.dispatch(Request::new(command)) {
        Ok(Response::Status {
            sync_available: false,
        }) => Output {
            exit_code: 0,
            text: "Foundation ready; clipboard sync: not implemented.\n".into(),
        },
        Ok(Response::Status {
            sync_available: true,
        }) => Output {
            exit_code: 0,
            text: "Clipboard sync available.\n".into(),
        },
        Ok(Response::Autostart(status)) => Output {
            exit_code: if status.error.is_some()
                || matches!(
                    status.observed,
                    ObservedAutostart::Unavailable | ObservedAutostart::Unknown
                ) {
                1
            } else {
                0
            },
            text: format!(
                "Start at login: {:?}; requested: {:?}; error: {:?}\n",
                status.observed, status.requested_enabled, status.error
            ),
        },
        Err(error) => failure(error),
    }
}

fn failure(error: ApiError) -> Output {
    Output {
        exit_code: 1,
        text: format!("Application error: {error:?}\n"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Recorder(Vec<Request>);
    impl ApplicationApi for Recorder {
        fn dispatch(&mut self, request: Request) -> Result<Response, ApiError> {
            self.0.push(request);
            Ok(Response::Status {
                sync_available: false,
            })
        }
    }
    #[test]
    fn only_public_api_receives_commands() {
        let mut app = Recorder(vec![]);
        assert_eq!(run(&["status".into()], &mut app).exit_code, 0);
        assert_eq!(app.0, [Request::new(Command::Status)]);
    }
    #[test]
    fn invalid_commands_do_not_invoke_application() {
        let mut app = Recorder(vec![]);
        assert_eq!(
            run(&["autostart".into(), "enable".into()], &mut app).exit_code,
            2
        );
        assert!(app.0.is_empty());
    }
}

pub mod control;
