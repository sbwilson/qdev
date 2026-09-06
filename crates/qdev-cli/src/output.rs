use qdev_core::{JsonEnvelope, JsonErrorEnvelope, QdevError};
use serde::Serialize;
use std::io::{self, Write};

pub struct OutputEmitter {
    json: bool,
}

impl OutputEmitter {
    pub fn new(json: bool) -> Self {
        Self { json }
    }

    #[allow(dead_code)]
    pub fn is_json(&self) -> bool {
        self.json
    }

    /// Emits a JSON-serializable envelope to STDOUT.
    pub fn emit_envelope<T: Serialize>(&self, envelope: &JsonEnvelope<T>) -> io::Result<()> {
        let json_str = serde_json::to_string_pretty(envelope).map_err(io::Error::other)?;
        Self::write_stdout(&json_str)
    }

    /// Emits an error per AD-13:
    /// - In JSON mode: emits JsonErrorEnvelope to STDOUT.
    /// - In text mode: emits formatted error to STDERR.
    pub fn emit_error(&self, error: &QdevError) -> io::Result<()> {
        if self.json {
            let envelope = JsonErrorEnvelope::from(error);
            Self::emit_error_envelope(&envelope)
        } else {
            eprintln!("{}", error);
            let _ = io::stderr().flush();
            Ok(())
        }
    }

    /// Direct helper to emit a JsonErrorEnvelope to STDOUT per AD-13.
    pub fn emit_error_envelope(envelope: &JsonErrorEnvelope) -> io::Result<()> {
        let json_str = serde_json::to_string_pretty(envelope).map_err(io::Error::other)?;
        Self::write_stdout(&json_str)
    }

    fn write_stdout(content: &str) -> io::Result<()> {
        let mut stdout = io::stdout().lock();
        if let Err(e) = writeln!(stdout, "{}", content) {
            if e.kind() != io::ErrorKind::BrokenPipe {
                return Err(e);
            }
        }
        if let Err(e) = stdout.flush() {
            if e.kind() != io::ErrorKind::BrokenPipe {
                return Err(e);
            }
        }
        Ok(())
    }
}
