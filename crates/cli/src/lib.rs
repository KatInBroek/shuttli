//! CLI presentation uses the public control contract.
#[derive(Debug, PartialEq, Eq)]
pub struct Output {
    pub exit_code: u8,
    pub text: String,
}
pub mod control;
