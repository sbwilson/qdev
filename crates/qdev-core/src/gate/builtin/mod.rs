//! Built-in gate verifiers.
//!
//! The three built-in gates (`qdev-scope`, `qdev-deps`, `qdev-hygiene`) verify
//! the worktree directly instead of running an external command; each lives in
//! its own module. The generic command runner in [`super::runner`] dispatches to
//! them by ID.

mod deps;
mod hygiene;
mod scope;

pub use deps::{execute_deps_gate, scan_rust_imports, scan_swift_imports};
pub use hygiene::execute_hygiene_gate;
pub use scope::execute_scope_gate;

/// Gate ID of the built-in scope gate.
pub const BUILTIN_GATE_SCOPE: &str = "qdev-scope";

/// Gate ID of the built-in dependency gate.
pub const BUILTIN_GATE_DEPS: &str = "qdev-deps";

/// Gate ID of the built-in hygiene gate.
pub const BUILTIN_GATE_HYGIENE: &str = "qdev-hygiene";

/// `true` when `gate_id` is one of the built-in gate IDs.
pub fn is_builtin_gate(gate_id: &str) -> bool {
    matches!(
        gate_id,
        BUILTIN_GATE_SCOPE | BUILTIN_GATE_DEPS | BUILTIN_GATE_HYGIENE
    )
}
