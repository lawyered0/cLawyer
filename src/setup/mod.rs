//! Interactive setup wizard for cLawyer.
//!
//! Provides a guided setup experience for:
//! - Quickstart mode (lawyer-first defaults)
//! - Advanced mode (full technical setup)
//!
//! # Example
//!
//! ```ignore
//! use clawyer::setup::SetupWizard;
//!
//! let mut wizard = SetupWizard::new();
//! wizard.run().await?;
//! ```

mod channels;
mod prompts;
#[cfg(any(feature = "postgres", feature = "libsql"))]
mod wizard;

pub use channels::{
    ChannelSetupError, SecretsContext, setup_http, setup_telegram, setup_tunnel,
    validate_telegram_token,
};
pub use prompts::{
    confirm, input, optional_input, print_error, print_header, print_info, print_step,
    print_success, secret_input, select_many, select_one,
};
#[cfg(any(feature = "postgres", feature = "libsql"))]
pub use wizard::{SetupConfig, SetupMode, SetupWizard};
