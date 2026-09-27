//! Composition helper. Async effect execution is intentionally not implemented yet.

use shuttli_application::Application;
use shuttli_ports::AutostartPort;

pub fn assemble<P: AutostartPort>(platform: P) -> Application<P> {
    Application::new(platform)
}
