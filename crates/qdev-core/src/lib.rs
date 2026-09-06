pub mod envelope;
pub mod errors;
pub mod interactivity;

pub use envelope::{ErrorPayload, JsonEnvelope, JsonErrorEnvelope, SCHEMA_VERSION};
pub use errors::{ExitCode, QdevError};
pub use interactivity::Interactivity;

use serde::{Deserialize, Serialize};

/// Basic pulse status returned by core routine for default command.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PulseStatus {
    pub name: String,
    pub version: String,
    pub interactivity: Interactivity,
}

/// Core routine returning the current status given the resolved interactivity mode.
pub fn get_pulse_status(interactivity: Interactivity) -> PulseStatus {
    PulseStatus {
        name: "qdev".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        interactivity,
    }
}
