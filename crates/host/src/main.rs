//! Composition root. No synchronization policy lives here.
#[cfg(unix)]
mod unix;
fn main() -> std::process::ExitCode {
    // Build metadata must work without a daemon, profile, or OS clipboard.
    if std::env::args().skip(1).eq(["--print-product-name"]) {
        println!("{}", shuttli_brand::NAME);
        return std::process::ExitCode::SUCCESS;
    }
    #[cfg(unix)]
    {
        match unix::run() {
            Ok(code) => std::process::ExitCode::from(code),
            Err(e) => {
                eprintln!("{}: {e}", shuttli_brand::NAME);
                std::process::ExitCode::FAILURE
            }
        }
    }
    #[cfg(not(unix))]
    {
        eprintln!(
            "{} daemon: platform host is not implemented on this OS",
            shuttli_brand::NAME
        );
        std::process::ExitCode::FAILURE
    }
}
