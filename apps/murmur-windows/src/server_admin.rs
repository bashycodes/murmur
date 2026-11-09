//! Settings → Your server → Users & keys (D39): the owner manages the
//! server's named keys (D38) from here when the saved server token is an
//! admin key. Thin over `flow_remote::admin::AdminClient` (shared with
//! Android through flow-ffi); this module only builds the client from the
//! saved settings and turns errors into page text.
//!
//! Privacy: a created key's secret goes back to the page once, in the
//! command's answer. It is never logged and never stored by the app.

use flow_protocol::admin::{code, AdminError, CreatedKey, KeyList, RevokedKey};
use serde::Serialize;

use crate::dev_settings::DevSettings;

/// What Settings needs to show or hide the page (`admin_status`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct AdminStatus {
    /// A server and a token are saved.
    pub configured: bool,
    /// The saved token is an admin key: show "Users & keys".
    pub admin: bool,
    /// The saved token's key name and id (empty when unknown).
    pub name: String,
    pub id: String,
    /// Why the status is unknown (server down, revoked key, old server).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
}

/// Text for an admin error code, for the page.
pub fn error_text(e: &AdminError) -> String {
    let hint = match e.code.as_str() {
        code::FORBIDDEN => "this app's key is not an admin key",
        code::UNAUTHORIZED => "the server no longer accepts this app's key",
        code::LAST_ADMIN => "this is the last admin key: create another admin key first",
        code::NAME_TAKEN => "a key with this name already exists",
        code::UNSUPPORTED => "this server is too old for key management: update flow-server",
        code::AUTH_OFF => "this server runs without keys",
        code::CONNECT_FAILED => "can't reach the server",
        _ => "",
    };
    match (hint, e.message.is_empty()) {
        ("", true) => e.code.clone(),
        ("", false) => e.message.clone(),
        (h, _) => h.to_string(),
    }
}

/// Admin operations against the saved server.
pub trait AdminApi {
    fn status(&self) -> AdminStatus;
    fn list(&self) -> Result<KeyList, String>;
    fn create(&self, name: &str, admin: bool) -> Result<CreatedKey, String>;
    fn revoke(&self, id: &str) -> Result<RevokedKey, String>;
}

/// The real client for the saved settings (or why there is none).
pub fn for_settings(dev: &DevSettings) -> Result<Box<dyn AdminApi + Send>, String> {
    #[cfg(feature = "remote")]
    {
        let token = dev.token();
        if dev.server.host.is_empty() || token.is_empty() {
            return Err("save a server address and token first".into());
        }
        let cfg = crate::router_adapter::server_config(dev, &token)?;
        Ok(Box::new(Remote(flow_remote::admin::AdminClient::new(
            cfg,
            &format!("flow-windows {}", flow_build_info::FULL),
        ))))
    }
    #[cfg(not(feature = "remote"))]
    {
        let _ = dev;
        Err("this build has no server support".into())
    }
}

/// [`AdminStatus`] for the saved settings, never failing.
pub fn status(dev: &DevSettings) -> AdminStatus {
    match for_settings(dev) {
        Ok(api) => api.status(),
        Err(e) => AdminStatus {
            error: Some(e),
            ..Default::default()
        },
    }
}

#[cfg(feature = "remote")]
struct Remote(flow_remote::admin::AdminClient);

#[cfg(feature = "remote")]
impl AdminApi for Remote {
    fn status(&self) -> AdminStatus {
        match self.0.whoami() {
            Ok(w) => AdminStatus {
                configured: true,
                admin: w.admin,
                name: w.name,
                id: w.id,
                error: None,
                code: None,
            },
            Err(e) => AdminStatus {
                configured: true,
                error: Some(error_text(&e)),
                code: Some(e.code),
                ..Default::default()
            },
        }
    }
    fn list(&self) -> Result<KeyList, String> {
        self.0.list_keys().map_err(|e| error_text(&e))
    }
    fn create(&self, name: &str, admin: bool) -> Result<CreatedKey, String> {
        let name = name.trim();
        if name.is_empty() {
            return Err("give the key a name, e.g. phone or laptop".into());
        }
        self.0.create_key(name, admin).map_err(|e| error_text(&e))
    }
    fn revoke(&self, id: &str) -> Result<RevokedKey, String> {
        self.0.revoke_key(id).map_err(|e| error_text(&e))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_text_hints_and_passes_messages() {
        let e = |c: &str, m: &str| AdminError::new(c, m);
        assert_eq!(
            error_text(&e(code::LAST_ADMIN, "\"owner\" is the last admin key")),
            "this is the last admin key: create another admin key first"
        );
        assert_eq!(
            error_text(&e(code::BAD_REQUEST, "key name \"a b\": ...")),
            "key name \"a b\": ..."
        );
        assert_eq!(error_text(&e("weird", "")), "weird");
        assert_eq!(error_text(&e(code::SERVER_ERROR, "disk full")), "disk full");
    }

    #[test]
    fn unconfigured_settings_have_no_status() {
        let s = status(&DevSettings::default());
        assert!(!s.configured && !s.admin && s.error.is_some());
    }
}
