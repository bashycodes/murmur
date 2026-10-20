//! THE integration point with `flow-core` (the only file that changes).
//!
//! Today: returns [`StubCore`]. When `crates/flow-core` lands:
//! 1. `Cargo.toml`: `flow-core = { path = "../../crates/flow-core" }`
//! 2. here: `struct FlowCore(flow_core::Handle)` + `impl CoreApi for FlowCore`
//!    mapping each method 1:1 onto §9 (`start_session`, `push_audio`, …),
//!    and build it in `make_core` with `flow_core::load_models(ModelConfig{..})`.

use crate::core_api::{CoreApi, StubCore};
use std::sync::Arc;

/// Where the shell looks for models (`%LOCALAPPDATA%\FlowClone\models`).
pub fn models_dir(local_appdata: Option<&str>) -> std::path::PathBuf {
    local_appdata
        .map(std::path::PathBuf::from)
        .unwrap_or_default()
        .join(crate::prefs::APP_DIR)
        .join("models")
}

pub fn make_core() -> Arc<dyn CoreApi> {
    Arc::new(StubCore::new())
}
