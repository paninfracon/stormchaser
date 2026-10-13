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
use stormchaser_model::step::{StepInstance, StepOutput};

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

/// Secret values shorter than this are not scrubbed: replacing every `1` or `ok` in a
/// response would wreck it without protecting anything.
const MIN_SECRET_LEN: usize = 4;

/// Scrubs one run's known secret values out of whatever the API returns about it.
///
/// A run's decrypted secrets and its `secret = true` inputs get interpolated into step
/// specs and parameters, and can come back in outputs, errors and log lines. Run details
/// are readable by more people than may read those secrets, so every occurrence is
/// replaced with [`MASK`] on the way out.
#[derive(Debug, Default, Clone)]
pub struct RunRedactor {
    /// Longest first, so a secret that contains another is masked whole.
    needles: Vec<String>,
}

impl RunRedactor {
    /// Builds a redactor from a run's stored context.
    pub fn new(
        sensitive_values: Vec<String>,
        secrets: &Value,
        inputs: &Value,
        inputs_schema: Option<&Value>,
    ) -> Self {
        let mut needles: Vec<String> = sensitive_values;
        collect_leaves(secrets, &mut needles);

        if let (Some(inputs), Some(props)) = (
            inputs.as_object(),
            inputs_schema
                .and_then(|schema| schema.get("properties"))
                .and_then(Value::as_object),
        ) {
            for (name, prop) in props {
                if prop.get("secret").and_then(Value::as_bool) == Some(true) {
                    if let Some(value) = inputs.get(name) {
                        collect_leaves(value, &mut needles);
                    }
                }
            }
        }

        needles.retain(|needle| needle.len() >= MIN_SECRET_LEN && needle != MASK);
        needles.sort_by(|a, b| b.len().cmp(&a.len()).then_with(|| a.cmp(b)));
        needles.dedup();
        Self { needles }
    }

    /// Loads the redactor for a run, live or archived. A run with no stored context
    /// yields a redactor that changes nothing.
    pub async fn load(
        pool: &sqlx::PgPool,
        run_id: stormchaser_model::RunId,
    ) -> Result<Self, sqlx::Error> {
        let row: Option<(Vec<String>, Value, Value, Option<Value>)> = sqlx::query_as(
            r#"
            SELECT sensitive_values, secrets, inputs, workflow_definition->'inputs_schema'
            FROM run_contexts WHERE run_id = $1
            UNION ALL
            SELECT sensitive_values, secrets, inputs, workflow_definition->'inputs_schema'
            FROM archived_run_contexts WHERE run_id = $1
            LIMIT 1
            "#,
        )
        .bind(run_id)
        .fetch_optional(pool)
        .await?;

        Ok(match row {
            Some((sensitive_values, secrets, inputs, schema)) => {
                Self::new(sensitive_values, &secrets, &inputs, schema.as_ref())
            }
            None => Self::default(),
        })
    }

    /// Replaces every known secret in `text`.
    pub fn text(&self, text: &str) -> String {
        let mut out = text.to_string();
        for needle in &self.needles {
            if out.contains(needle.as_str()) {
                out = out.replace(needle.as_str(), MASK);
            }
        }
        out
    }

    /// Replaces every known secret in the strings and keys of a JSON value.
    pub fn value(&self, value: &mut Value) {
        if self.needles.is_empty() {
            return;
        }
        match value {
            Value::String(s) => *s = self.text(s),
            Value::Number(n) => {
                if self.needles.contains(&n.to_string()) {
                    *value = Value::String(MASK.to_string());
                }
            }
            Value::Array(items) => items.iter_mut().for_each(|item| self.value(item)),
            Value::Object(map) => {
                let entries = std::mem::take(map);
                for (key, mut item) in entries {
                    self.value(&mut item);
                    map.insert(self.text(&key), item);
                }
            }
            Value::Null | Value::Bool(_) => {}
        }
    }

    /// Scrubs a step's spec, parameters and error message.
    pub fn step_instance(&self, mut instance: StepInstance) -> StepInstance {
        self.value(&mut instance.spec);
        self.value(&mut instance.params);
        instance.error = instance.error.map(|error| self.text(&error));
        instance
    }

    /// Masks an output marked sensitive outright, and scrubs known secrets from the rest.
    pub fn step_output(&self, mut output: StepOutput) -> StepOutput {
        if output.is_sensitive {
            output.value = Value::String(MASK.to_string());
        } else {
            self.value(&mut output.value);
        }
        output
    }
}

/// Collects every string and number in a JSON value as text.
fn collect_leaves(value: &Value, out: &mut Vec<String>) {
    match value {
        Value::String(s) => out.push(s.clone()),
        Value::Number(n) => out.push(n.to_string()),
        Value::Array(items) => items.iter().for_each(|item| collect_leaves(item, out)),
        Value::Object(map) => map.values().for_each(|item| collect_leaves(item, out)),
        Value::Null | Value::Bool(_) => {}
    }
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
    fn run_redactor() -> RunRedactor {
        RunRedactor::new(
            vec!["sops-value-1".to_string()],
            &serde_json::json!({"db": {"password": "hunter2hunter2", "port": 5432}, "pin": "12"}),
            &serde_json::json!({"api_token": "tok_live_abcdef", "region": "eu-west-1"}),
            Some(&serde_json::json!({"properties": {
                "api_token": {"type": "string", "secret": true},
                "region": {"type": "string"}
            }})),
        )
    }

    #[test]
    fn run_secrets_are_scrubbed_from_text_and_json() {
        let redactor = run_redactor();
        assert_eq!(
            redactor.text("curl -H 'Authorization: Bearer tok_live_abcdef' in eu-west-1"),
            format!("curl -H 'Authorization: Bearer {MASK}' in eu-west-1")
        );

        let mut spec = serde_json::json!({
            "env": [{"name": "PGPASSWORD", "value": "hunter2hunter2"}],
            "args": ["--token=tok_live_abcdef", "--sops", "sops-value-1"],
            "port": 5432,
            "region": "eu-west-1",
        });
        redactor.value(&mut spec);
        assert_eq!(spec["env"][0]["value"], MASK);
        assert_eq!(spec["args"][0], format!("--token={MASK}"));
        assert_eq!(spec["args"][2], MASK);
        assert_eq!(spec["port"], MASK);
        assert_eq!(spec["region"], "eu-west-1");
    }

    #[test]
    fn short_values_and_ordinary_inputs_are_left_alone() {
        let redactor = run_redactor();
        // "12" is a secret but too short to scrub safely; region is not a secret input.
        assert_eq!(
            redactor.text("step 12 of 120 in eu-west-1"),
            "step 12 of 120 in eu-west-1"
        );
    }

    #[test]
    fn a_secret_containing_another_is_masked_whole() {
        let redactor = RunRedactor::new(
            vec!["abcd".to_string(), "abcd-efgh-ijkl".to_string()],
            &Value::Null,
            &Value::Null,
            None,
        );
        assert_eq!(redactor.text("key=abcd-efgh-ijkl"), format!("key={MASK}"));
    }

    #[test]
    fn sensitive_outputs_are_masked_outright() {
        let redactor = RunRedactor::default();
        let output = redactor.step_output(StepOutput {
            step_instance_id: stormchaser_model::StepInstanceId::new_v4(),
            key: "session".to_string(),
            value: serde_json::json!({"token": "never-seen-before"}),
            is_sensitive: true,
        });
        assert_eq!(output.value, MASK);
    }
}
