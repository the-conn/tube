# AGENTS.md

## Development Standards and Constraints

This document provides mandatory instructions for all AI-assisted development on the **Tube** binary and **Jefferies** backend.

### 1. Verification and Quality Control
* **Command Interface**: All development tasks must be executed via `make` targets. Do not call `cargo` directly if a corresponding `make` target exists.
* **Validation**: All changes must pass the following sequence before finalization:
    * `make fmt`: Ensure code is properly formatted.
    * `make lint`: Ensure `clippy` returns zero warnings or errors.
    * `make test`: Ensure all unit and integration tests pass.
* **Testing**: New functionality must be accompanied by tests. When modifying existing code, update relevant tests to reflect logic changes.

### 2. Code Philosophy
* **Self-Documentation**: Code must be readable and self-documenting. Use clear, descriptive naming conventions.
* **Clean Code**: Avoid unnecessary inline comments. Comments should explain "why," not "what."
* **Modularization**: Large functions must be decomposed into small, single-purpose helper functions to maintain readability.
* **Encapsulation**: Items must not be marked `pub` unless explicitly required by an external module.

### 3. Formatting and Syntax
* **Standard Characters**: No emojis, non-standard Unicode, or decorative ASCII.
* **Documentation**: Keep both `README.md` and `docs/architecture.md` in sync with the code.
    * Update `README.md` when changes affect the configuration schema, environment variables, lifecycle, secrets handling, or deployment instructions.
    * Update `docs/architecture.md` when changes affect the Pod composition (init containers, mounts, entrypoint), execution flow, secret-handling model, logging/upload behavior, failure surface, or the boundaries of what `tube` does and does not do. Keep its mermaid diagram accurate when the flow changes.

### 4. Technical Constraints
* **Error Handling**: Adhere to established unified error enum patterns. Never use `unwrap` or `expect` in production code; all fallible operations must propagate errors through the appropriate error enum variant.
* **Performance**: Prioritize streaming and zero-copy operations for IO and network tasks.
