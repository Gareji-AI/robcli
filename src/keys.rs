use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::net::IpAddr;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use zeroize::Zeroizing;

use crate::error::AppError;

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct KeySpec {
    name: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    is_enabled: bool,
    pub expiration_time: String,
    allowed_cidrs: Vec<String>,
    scopes: Vec<Scope>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Scope {
    scope_type: String,
    target_parts: Vec<String>,
    operations: Vec<String>,
}

fn invalid() -> AppError {
    AppError::new(
        "key_spec_invalid",
        "The key plan is invalid or insufficiently scoped.",
        "Provide a metadata-only JSON plan: explicit non-wildcard scopes and targets, non-/0 CIDRs, and an expiry within 30 days. Raw credentials and unknown fields are rejected.",
    )
}

fn safe_text(value: &str, max: usize) -> bool {
    !value.is_empty() && value.len() <= max && !value.chars().any(char::is_control)
}

impl KeySpec {
    pub fn load(path: &Path, now: u64) -> Result<Self, AppError> {
        let mut bytes = Vec::new();
        std::fs::File::open(path)
            .map_err(|_| invalid())?
            .take(32769)
            .read_to_end(&mut bytes)
            .map_err(|_| invalid())?;
        Self::parse(&bytes, now)
    }

    pub fn parse(bytes: &[u8], now: u64) -> Result<Self, AppError> {
        if bytes.len() > 32768 {
            return Err(invalid());
        }
        let spec: Self = serde_json::from_slice(bytes).map_err(|_| invalid())?;
        let expiry = chrono::DateTime::parse_from_rfc3339(&spec.expiration_time)
            .map_err(|_| invalid())?
            .timestamp();
        let now = i64::try_from(now).map_err(|_| invalid())?;
        if !safe_text(&spec.name, 50)
            || spec.description.len() > 1000
            || spec.description.chars().any(char::is_control)
            || expiry <= now
            || expiry - now > 30 * 86400
            || spec.allowed_cidrs.is_empty()
            || spec.allowed_cidrs.len() > 16
            || spec.scopes.is_empty()
            || spec.scopes.len() > 16
        {
            return Err(invalid());
        }
        for cidr in &spec.allowed_cidrs {
            let (ip, prefix) = cidr.split_once('/').ok_or_else(invalid)?;
            let ip: IpAddr = ip.parse().map_err(|_| invalid())?;
            let prefix: u8 = prefix.parse().map_err(|_| invalid())?;
            if prefix == 0 || prefix > if ip.is_ipv4() { 32 } else { 128 } {
                return Err(invalid());
            }
        }
        for scope in &spec.scopes {
            if !safe_text(&scope.scope_type, 100)
                || scope.scope_type.contains('*')
                || scope.target_parts.is_empty()
                || scope.target_parts.len() > 16
                || scope.operations.is_empty()
                || scope.operations.len() > 32
                || scope
                    .target_parts
                    .iter()
                    .any(|value| !safe_text(value, 128) || value.contains('*'))
                || scope
                    .operations
                    .iter()
                    .any(|value| !safe_text(value, 128) || value.contains('*'))
            {
                return Err(invalid());
            }
        }
        Ok(spec)
    }

    pub fn body(&self) -> Value {
        json!({ "cloudAuthUserConfiguredProperties": self })
    }

    pub fn requested_enabled(&self) -> bool {
        self.is_enabled
    }

    pub fn expiry_confirmed(&self, actual: Option<&str>) -> bool {
        actual
            .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
            .zip(chrono::DateTime::parse_from_rfc3339(&self.expiration_time).ok())
            .is_some_and(|(actual, requested)| actual == requested)
    }

    pub fn plan(&self, secret_name: &str) -> Value {
        json!({ "dryRun": true, "owner": "explicitly confirmed signed-in user", "request": self.body(),
            "secretName": secret_name, "secretOutput": "dedicated local .key file outside Git/Vault; no stdout secret",
            "requirements": ["--apply", "--live", "--confirm-user <id>"],
            "note": "Group-owned key creation is not inferred; scopes may explicitly target group resources. No automatic enable, expiry repair, revoke, or retry." })
    }
}

#[derive(Deserialize)]
#[serde(try_from = "CreatedReply")]
pub(crate) struct Created {
    pub id: String,
    secret: Zeroizing<String>,
    pub expiration_time: Option<String>,
    pub is_enabled: Option<bool>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreatedReply {
    cloud_auth_info: CreatedInfo,
    apikey_secret: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreatedInfo {
    id: String,
    cloud_auth_user_configured_properties: Option<CreatedProperties>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreatedProperties {
    expiration_time: Option<String>,
    is_enabled: Option<bool>,
}

impl TryFrom<CreatedReply> for Created {
    type Error = &'static str;
    fn try_from(reply: CreatedReply) -> Result<Self, Self::Error> {
        let secret = Zeroizing::new(reply.apikey_secret);
        if reply.cloud_auth_info.id.is_empty()
            || reply.cloud_auth_info.id.len() > 512
            || secret.is_empty()
            || secret.len() > 65536
            || secret
                .chars()
                .any(|character| character.is_whitespace() || character.is_control())
        {
            return Err("The create response was not recognized; no secret is echoed.");
        }
        let properties = reply.cloud_auth_info.cloud_auth_user_configured_properties;
        Ok(Self {
            id: reply.cloud_auth_info.id,
            secret,
            expiration_time: properties
                .as_ref()
                .and_then(|value| value.expiration_time.clone()),
            is_enabled: properties.and_then(|value| value.is_enabled),
        })
    }
}

pub(crate) struct SecretFile {
    file: File,
    path: PathBuf,
    retain: bool,
}

impl SecretFile {
    pub fn validate_name(name: &str) -> Result<(), AppError> {
        let device = ["con", "prn", "aux", "nul"].contains(&name)
            || (name.len() == 4
                && (name.starts_with("com") || name.starts_with("lpt"))
                && matches!(name.as_bytes()[3], b'1'..=b'9'));
        if name.is_empty()
            || name.len() > 64
            || !name
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
            || name.starts_with('-')
            || name.ends_with('-')
            || device
        {
            return Err(AppError::new(
                "secret_name_invalid",
                "Choose a lowercase ASCII slug for the secret file.",
                "Use --secret-name with letters, digits and internal hyphens; no paths or file extensions.",
            ));
        }
        Ok(())
    }

    pub fn reserve(root: &Path, name: &str) -> Result<Self, AppError> {
        Self::validate_name(name)?;
        std::fs::create_dir_all(root).map_err(|_| secret_error())?;
        let root = std::fs::canonicalize(root).map_err(|_| secret_error())?;
        if root.ancestors().any(|parent| parent.join(".git").exists()) {
            return Err(AppError::new(
                "secret_store_in_git",
                "Refusing to save a generated key inside a Git worktree or Vault.",
                "Use the dedicated OS-local robcli key store. Ignored files inside Git are refused too.",
            ));
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700))
                .map_err(|_| secret_error())?;
        }
        let path = root.join(format!("{name}.key"));
        let mut options = OpenOptions::new();
        options.create_new(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let file = options.open(&path).map_err(|_| AppError::new("secret_file_unavailable", "The new secret file cannot be exclusively reserved; existing files are never overwritten.", "Choose a new secret name outside Git. Do not overwrite an existing key or rerun an uncertain create."))?;
        Ok(Self {
            file,
            path,
            retain: false,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn retain_for_reconciliation(&mut self) {
        self.retain = true;
    }

    pub fn persist(&mut self, created: &Created) -> Result<Value, AppError> {
        self.retain = true;
        self.file
            .write_all(created.secret.as_bytes())
            .and_then(|()| self.file.write_all(b"\n"))
            .and_then(|()| self.file.flush())
            .and_then(|()| self.file.sync_all())
            .map_err(|_| secret_error())?;
        Ok(
            json!({ "keyId": created.id, "secretFile": self.path.to_string_lossy(), "secretPrinted": false,
            "serverExpirationTime": created.expiration_time, "serverEnabled": created.is_enabled }),
        )
    }
}

impl Drop for SecretFile {
    fn drop(&mut self) {
        if !self.retain {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

fn secret_error() -> AppError {
    AppError::new(
        "secret_store_failed",
        "Cannot securely reserve or write the local secret file.",
        "If a create was submitted, do not repeat it: inspect the Dashboard and the reserved file. No raw key is printed as a fallback.",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn key_plan_is_explicitly_scoped_and_rejects_broad_permissions() {
        let narrow = json!({
            "name": "analytics-read", "description": "synthetic", "isEnabled": false,
            "expirationTime": "1970-01-02T00:00:00Z", "allowedCidrs": ["203.0.113.7/32"],
            "scopes": [{ "scopeType": "universe-analytics", "targetParts": ["777"], "operations": ["universe.analytics:read"] }]
        });
        let plan = KeySpec::parse(&serde_json::to_vec(&narrow).unwrap(), 1000).unwrap();
        assert_eq!(
            plan.body()["cloudAuthUserConfiguredProperties"]["scopes"][0]["targetParts"][0],
            "777"
        );
        let mut broad = narrow.clone();
        broad["allowedCidrs"] = json!(["0.0.0.0/0"]);
        assert!(KeySpec::parse(&serde_json::to_vec(&broad).unwrap(), 1000).is_err());
        broad = narrow;
        broad["scopes"][0]["targetParts"] = json!(["*"]);
        assert!(KeySpec::parse(&serde_json::to_vec(&broad).unwrap(), 1000).is_err());
    }

    #[test]
    fn secret_is_saved_once_outside_git_and_never_returned_in_receipt() {
        let root = tempfile::tempdir().unwrap();
        let mut sink = SecretFile::reserve(root.path(), "analytics-read").unwrap();
        let created: Created = serde_json::from_value(json!({
            "cloudAuthInfo": { "id": "key-123" }, "apikeySecret": "synthetic-created-secret"
        }))
        .unwrap();
        let receipt = sink.persist(&created).unwrap();
        assert!(!receipt.to_string().contains("synthetic-created-secret"));
        assert_eq!(
            std::fs::read_to_string(sink.path()).unwrap(),
            "synthetic-created-secret\n"
        );
        assert!(SecretFile::reserve(root.path(), "analytics-read").is_err());
        assert_eq!(receipt["keyId"], "key-123");
        let git_root = tempfile::tempdir().unwrap();
        std::fs::create_dir(git_root.path().join(".git")).unwrap();
        assert!(SecretFile::reserve(&git_root.path().join("keys"), "must-refuse").is_err());
    }

    #[test]
    fn receipt_reports_server_enable_state_and_equivalent_expiry_without_exposing_secret() {
        let spec = KeySpec::parse(
            br#"{"name":"read","expirationTime":"1970-01-02T00:00:00Z","allowedCidrs":["203.0.113.7/32"],"scopes":[{"scopeType":"universe-analytics","targetParts":["777"],"operations":["universe.analytics:read"]}]}"#,
            1000,
        ).unwrap();
        let created: Created = serde_json::from_value(json!({
            "cloudAuthInfo": { "id": "key-123", "cloudAuthUserConfiguredProperties": {
                "expirationTime": "1970-01-02T00:00:00+00:00", "isEnabled": false
            } }, "apikeySecret": "synthetic-created-secret"
        }))
        .unwrap();
        let root = tempfile::tempdir().unwrap();
        let mut sink = SecretFile::reserve(root.path(), "receipt").unwrap();
        let receipt = sink.persist(&created).unwrap();
        assert_eq!(receipt["serverEnabled"], false);
        assert!(!spec.requested_enabled());
        assert!(spec.expiry_confirmed(created.expiration_time.as_deref()));
        assert!(!spec.expiry_confirmed(None));
        assert!(!spec.expiry_confirmed(Some("invalid")));
        assert!(!receipt.to_string().contains("synthetic-created-secret"));
    }

    #[test]
    fn uncertain_create_keeps_an_empty_reservation_instead_of_allowing_retry() {
        let root = tempfile::tempdir().unwrap();
        let path;
        {
            let mut sink = SecretFile::reserve(root.path(), "uncertain").unwrap();
            path = sink.path().to_path_buf();
            sink.retain_for_reconciliation();
        }
        assert_eq!(std::fs::read(&path).unwrap(), b"");
        assert!(SecretFile::reserve(root.path(), "uncertain").is_err());
        {
            let sink = SecretFile::reserve(root.path(), "not-submitted").unwrap();
            assert!(sink.path().exists());
        }
        assert!(!root.path().join("not-submitted.key").exists());
    }

    #[test]
    fn secret_names_cannot_escape_storage_or_use_windows_devices() {
        for name in [
            "../escape",
            "CON",
            "con",
            "nul",
            "aux",
            "com1",
            "lpt9",
            "file.key",
            "-name",
            "name-",
        ] {
            assert!(SecretFile::validate_name(name).is_err());
        }
    }
}
