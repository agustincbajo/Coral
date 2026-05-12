//! `GET /health` — simple liveness probe.

use std::sync::Arc;

use serde::Serialize;

use crate::error::ApiError;
use crate::state::AppState;

#[derive(Serialize)]
struct Health {
    status: &'static str,
    version: &'static str,
}

#[derive(Serialize)]
struct Envelope<T: Serialize> {
    data: T,
}

pub fn handle(_state: &Arc<AppState>) -> Result<Vec<u8>, ApiError> {
    let body = Envelope {
        data: Health {
            status: "ok",
            version: env!("CARGO_PKG_VERSION"),
        },
    };
    Ok(serde_json::to_vec(&body).map_err(|e| anyhow::anyhow!(e))?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn state() -> Arc<AppState> {
        Arc::new(AppState {
            bind: "127.0.0.1".into(),
            port: 3838,
            wiki_root: PathBuf::from(".wiki"),
            token: None,
            allow_write_tools: false,
            runner: None,
        })
    }

    #[test]
    fn health_returns_status_ok_envelope() {
        let body = handle(&state()).unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["data"]["status"], "ok");
        assert!(v["data"]["version"].is_string());
    }
}
