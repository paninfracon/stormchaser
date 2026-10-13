//! Keeps stored secrets out of API responses.
//!
//! Connections, webhooks and cron schedules hold credentials that the engine needs but
//! that callers of the API never need to read back. Responses carry [`MASK`] in their
//! place, and an update that sends [`MASK`] back means "leave this value as it is", so
//! a client can load an object, edit other fields and save it without losing secrets.

use serde_json::Value;
use stormchaser_model::connections::Connection;
use stormchaser_model::cron::CronWorkflow;
use stormchaser_model::event_rules::WebhookConfig;

/// Shown in place of a stored secret.
pub const MASK: &str = "********";

/// True for configuration keys whose values are credentials.
fn is_secret_key(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    [
        "secret",
        "password",
        "passwd",
        "token",
        "private",
        "ssh_key",
        "credential",
        "api_key",
        "apikey",
        "authorization",
        "cookie",
    ]
    .iter()
    .any(|needle| key.contains(needle))
}

/// Replaces the password in a URL such as `postgres://user:pass@host/db`, if it has one.
fn mask_url_password(value: &str) -> Option<String> {
    let mut url = url::Url::parse(value).ok()?;
    url.password()?;
    url.set_password(Some(MASK)).ok()?;
    Some(url.to_string())
}

/// Masks every credential in a connection's free-form configuration.
pub fn mask_config(config: &mut Value) {
    match config {
        Value::Object(map) => {
            for (key, value) in map.iter_mut() {
                match value {
                    Value::String(s) if is_secret_key(key) => {
                        if !s.is_empty() {
                            *s = MASK.to_string();
                        }
                    }
                    Value::String(s) => {
                        if let Some(masked) = mask_url_password(s) {
                            *s = masked;
                        }
                    }
                    other => mask_config(other),
                }
            }
        }
        Value::Array(items) => items.iter_mut().for_each(mask_config),
        _ => {}
    }
}

/// Puts stored secrets back wherever an update carries the mask instead of a new value.
pub fn restore_masked(incoming: &mut Value, stored: &Value) {
    match (incoming, stored) {
        (Value::Object(new), Value::Object(old)) => {
            for (key, value) in new.iter_mut() {
                if let Some(old_value) = old.get(key) {
                    restore_masked(value, old_value);
                }
            }
        }
        (Value::Array(new), Value::Array(old)) => {
            for (value, old_value) in new.iter_mut().zip(old) {
                restore_masked(value, old_value);
            }
        }
        (Value::String(new), Value::String(old)) => {
            if new == MASK {
                *new = old.clone();
            } else if let (Ok(mut new_url), Ok(old_url)) =
                (url::Url::parse(new), url::Url::parse(old))
            {
                if new_url.password() == Some(MASK)
                    && new_url.set_password(old_url.password()).is_ok()
                {
                    *new = new_url.to_string();
                }
            }
        }
        _ => {}
    }
}

/// A connection as the API returns it: credentials masked.
pub fn connection(mut connection: Connection) -> Connection {
    mask_config(&mut connection.config);
    connection.encrypted_credentials = connection.encrypted_credentials.map(|_| MASK.to_string());
    connection.client_key = connection.client_key.map(|_| MASK.to_string());
    connection
}

/// A webhook as the API returns it: signing secret and client key masked.
pub fn webhook(mut webhook: WebhookConfig) -> WebhookConfig {
    webhook.secret_token = webhook.secret_token.map(|_| MASK.to_string());
    webhook.client_key = webhook.client_key.map(|_| MASK.to_string());
    webhook
}

/// A cron schedule as the API returns it. Its trigger token is shown once, when the
/// schedule is created.
pub fn cron_workflow(mut cron: CronWorkflow) -> CronWorkflow {
    cron.secret_token = MASK.to_string();
    cron
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn config_credentials_are_masked() {
        let mut config = json!({
            "bucket": "artifacts",
            "access_key": "AKIAEXAMPLE",
            "secret_key": "s3-secret",
            "url": "postgres://app:db-pass@db.internal:5432/app",
            "base_url": "https://api.example.com",
            "headers": { "Authorization": "Bearer abc", "Accept": "application/json" },
            "ssh_key": "-----BEGIN OPENSSH PRIVATE KEY-----",
            "password": "",
        });
        mask_config(&mut config);

        let text = config.to_string();
        for leaked in ["s3-secret", "db-pass", "Bearer abc", "OPENSSH"] {
            assert!(!text.contains(leaked), "{leaked} leaked: {text}");
        }
        assert_eq!(config["bucket"], "artifacts");
        assert_eq!(config["access_key"], "AKIAEXAMPLE");
        assert_eq!(config["base_url"], "https://api.example.com");
        assert_eq!(config["headers"]["Accept"], "application/json");
        assert_eq!(
            config["url"],
            "postgres://app:********@db.internal:5432/app"
        );
        // An unset secret stays visibly unset.
        assert_eq!(config["password"], "");
    }

    #[test]
    fn saving_a_masked_config_keeps_the_stored_secrets() {
        let stored = json!({
            "bucket": "artifacts",
            "secret_key": "s3-secret",
            "url": "postgres://app:db-pass@db.internal:5432/app",
            "headers": { "Authorization": "Bearer abc" },
        });
        let mut edited = stored.clone();
        mask_config(&mut edited);
        // The client changes one ordinary field and one secret, and sends the rest back.
        edited["bucket"] = json!("artifacts-v2");
        edited["headers"]["Authorization"] = json!("Bearer new");

        restore_masked(&mut edited, &stored);

        assert_eq!(edited["bucket"], "artifacts-v2");
        assert_eq!(edited["secret_key"], "s3-secret");
        assert_eq!(edited["url"], "postgres://app:db-pass@db.internal:5432/app");
        assert_eq!(edited["headers"]["Authorization"], "Bearer new");
    }
}
