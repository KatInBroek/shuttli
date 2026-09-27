//! First application slice: version checking and observed autostart status.
//! Settings persistence and OS registration are separate later tasks.

use shuttli_api::{
    API_VERSION, ApiError, ApplicationApi, AutostartStatus, Command, ObservedAutostart, Request,
    Response,
};
use shuttli_model::{AutostartObserved, PortError};
use shuttli_ports::AutostartPort;

pub struct Application<P> {
    autostart: P,
    requested_autostart: Option<bool>,
}

impl<P: AutostartPort> Application<P> {
    pub fn new(autostart: P) -> Self {
        Self {
            autostart,
            requested_autostart: None,
        }
    }

    fn autostart_status(&mut self, setting_error: Option<ApiError>) -> Response {
        let (observed, error) = match self.autostart.query() {
            Ok(value) => (
                match value {
                    AutostartObserved::Enabled => ObservedAutostart::Enabled,
                    AutostartObserved::Disabled => ObservedAutostart::Disabled,
                    AutostartObserved::RequiresUserAction => ObservedAutostart::RequiresUserAction,
                    AutostartObserved::Unavailable => ObservedAutostart::Unavailable,
                    AutostartObserved::Unknown => ObservedAutostart::Unknown,
                },
                setting_error,
            ),
            Err(error) => (
                ObservedAutostart::Unknown,
                setting_error.or(Some(map_error(error))),
            ),
        };
        Response::Autostart(AutostartStatus {
            requested_enabled: self.requested_autostart,
            observed,
            error,
        })
    }
}

impl<P: AutostartPort> ApplicationApi for Application<P> {
    fn dispatch(&mut self, request: Request) -> Result<Response, ApiError> {
        if request.version != API_VERSION {
            return Err(ApiError::UnsupportedVersion);
        }
        Ok(match request.command {
            Command::Status => Response::Status {
                sync_available: false,
            },
            Command::GetAutostartStatus => self.autostart_status(None),
            Command::SetAutostart { enabled } => {
                self.requested_autostart = Some(enabled);
                let error = self.autostart.set_enabled(enabled).err().map(map_error);
                self.autostart_status(error)
            }
        })
    }
}

fn map_error(error: PortError) -> ApiError {
    match error {
        PortError::Unavailable => ApiError::Unavailable,
        PortError::PermissionDenied => ApiError::PermissionDenied,
        PortError::Busy => ApiError::Busy,
        _ => ApiError::OperationFailed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Platform {
        state: AutostartObserved,
        set_error: Option<PortError>,
        query_error: Option<PortError>,
        writes: usize,
    }
    impl AutostartPort for Platform {
        fn query(&mut self) -> Result<AutostartObserved, PortError> {
            self.query_error.map_or(Ok(self.state), Err)
        }
        fn set_enabled(&mut self, _: bool) -> Result<(), PortError> {
            self.writes += 1;
            self.set_error.map_or(Ok(()), Err)
        }
    }
    fn platform(state: AutostartObserved) -> Platform {
        Platform {
            state,
            set_error: None,
            query_error: None,
            writes: 0,
        }
    }
    #[test]
    fn an_accepted_request_does_not_fake_os_registration() {
        for state in [
            AutostartObserved::Disabled,
            AutostartObserved::RequiresUserAction,
            AutostartObserved::Unavailable,
            AutostartObserved::Unknown,
            AutostartObserved::Enabled,
        ] {
            let mut app = Application::new(platform(state));
            let result = app
                .dispatch(Request::new(Command::SetAutostart { enabled: true }))
                .unwrap();
            let Response::Autostart(status) = result else {
                panic!("wrong response")
            };
            assert_eq!(status.requested_enabled, Some(true));
            assert_eq!(
                status.observed,
                match state {
                    AutostartObserved::Disabled => ObservedAutostart::Disabled,
                    AutostartObserved::RequiresUserAction => ObservedAutostart::RequiresUserAction,
                    AutostartObserved::Unavailable => ObservedAutostart::Unavailable,
                    AutostartObserved::Unknown => ObservedAutostart::Unknown,
                    AutostartObserved::Enabled => ObservedAutostart::Enabled,
                }
            );
        }
    }
    #[test]
    fn failed_disable_retains_actual_enabled_status() {
        let mut port = platform(AutostartObserved::Enabled);
        port.set_error = Some(PortError::PermissionDenied);
        let mut app = Application::new(port);
        assert_eq!(
            app.dispatch(Request::new(Command::SetAutostart { enabled: false })),
            Ok(Response::Autostart(AutostartStatus {
                requested_enabled: Some(false),
                observed: ObservedAutostart::Enabled,
                error: Some(ApiError::PermissionDenied),
            }))
        );
    }
    #[test]
    fn external_disable_is_observed_without_reregistering() {
        let mut app = Application::new(platform(AutostartObserved::Enabled));
        app.dispatch(Request::new(Command::SetAutostart { enabled: true }))
            .unwrap();
        app.autostart.state = AutostartObserved::Disabled;
        let Response::Autostart(status) = app
            .dispatch(Request::new(Command::GetAutostartStatus))
            .unwrap()
        else {
            panic!("wrong response")
        };
        assert_eq!(status.observed, ObservedAutostart::Disabled);
        assert_eq!(app.autostart.writes, 1);
    }
    #[test]
    fn query_failure_is_unknown_not_disabled() {
        let mut port = platform(AutostartObserved::Enabled);
        port.query_error = Some(PortError::Unavailable);
        let mut app = Application::new(port);
        let Response::Autostart(status) = app
            .dispatch(Request::new(Command::GetAutostartStatus))
            .unwrap()
        else {
            panic!("wrong response")
        };
        assert_eq!(status.observed, ObservedAutostart::Unknown);
        assert_eq!(status.error, Some(ApiError::Unavailable));
    }
    #[test]
    fn unknown_api_version_has_no_side_effects() {
        let mut app = Application::new(platform(AutostartObserved::Disabled));
        assert_eq!(
            app.dispatch(Request {
                version: 999,
                command: Command::SetAutostart { enabled: true }
            }),
            Err(ApiError::UnsupportedVersion)
        );
        assert_eq!(app.autostart.writes, 0);
    }
}

pub mod service;

#[cfg(test)]
mod service_tests;
