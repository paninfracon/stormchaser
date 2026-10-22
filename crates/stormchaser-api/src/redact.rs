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

/// True for configuration keys whose names say their values are credentials.
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
        "api-key",
        "authorization",
        "cookie",
        "signature",
    ]
    .iter()
    .any(|needle| key.contains(needle))
}

/// Configuration keys whose values are shown as they are.
///
/// A connection's configuration is free-form, so there is no telling what a key nobody
/// thought of holds. Everything is therefore masked except the keys listed here, which
/// name where a connection points and who it connects as, never what it proves that with.
const PUBLIC_CONFIG_KEYS: &[&str] = &[
    "url",
    "base_url",
    "endpoint",
    "endpoint_url",
    "host",
    "hostname",
    "port",
    "bucket",
    "region",
    "registry",
    "repo",
    "repo_url",
    "repository",
    "branch",
    "ref",
    "path",
    "prefix",
    "container",
    "namespace",
    "project",
    "project_id",
    "account",
    "account_name",
    "database",
    "dbname",
    "db",
    "schema",
    "sslmode",
    "username",
    "user",
    "email",
    "access_key",
    "access_key_id",
    "name",
    "type",
    "description",
    "force_path_style",
    "plain_http",
    "insecure",
    "timeout",
];

/// Request headers that describe a request without authorising it.
const PUBLIC_HEADERS: &[&str] = &[
    "accept",
    "accept-encoding",
    "accept-language",
    "content-type",
    "user-agent",
];

/// URL query parameters that are settings, not credentials.
const PUBLIC_QUERY_KEYS: &[&str] = &[
    "sslmode",
    "ssl",
    "region",
    "timeout",
    "connect_timeout",
    "application_name",
    "schema",
    "search_path",
    "charset",
    "api-version",
    "version",
];

fn is_public(key: &str, list: &[&str]) -> bool {
    let key = key.to_ascii_lowercase();
    list.contains(&key.as_str()) && !is_secret_key(&key)
}

/// Masks the credentials a URL can carry: its password, and every query parameter that
/// is not a known setting (access tokens, signatures and SAS tokens travel there).
/// Returns `None` if `value` is not a URL or holds nothing to mask.
fn mask_url(value: &str) -> Option<String> {
    let mut url = url::Url::parse(value).ok()?;
    let mut changed = false;

    if url.password().is_some() {
        url.set_password(Some(MASK)).ok()?;
        changed = true;
    }

    if url.query().is_some() {
        let pairs: Vec<(String, String)> = url
            .query_pairs()
            .map(|(k, v)| (k.into_owned(), v.into_owned()))
            .collect();
        if pairs
            .iter()
            .any(|(k, v)| !v.is_empty() && !is_public(k, PUBLIC_QUERY_KEYS))
        {
            url.query_pairs_mut()
                .clear()
                .extend_pairs(pairs.iter().map(|(k, v)| {
                    let shown = if v.is_empty() || is_public(k, PUBLIC_QUERY_KEYS) {
                        v.as_str()
                    } else {
                        MASK
                    };
                    (k.as_str(), shown)
                }));
            changed = true;
        }
    }

    changed.then(|| url.to_string())
}

/// What to show for a string stored under a public key.
fn mask_public_string(value: &str) -> Option<String> {
    if let Some(masked) = mask_url(value) {
        return Some(masked);
    }
    if url::Url::parse(value).is_ok() {
        return None;
    }
    // Not a URL: a keyword-style connection string such as `host=db password=x`
    let lower = value.to_ascii_lowercase();
    ["password=", "passwd=", "secret=", "token=", "pwd="]
        .iter()
        .any(|needle| lower.contains(needle))
        .then(|| MASK.to_string())
}

/// Masks every credential in a connection's free-form configuration.
///
/// Anything not known to be harmless is masked: see [`PUBLIC_CONFIG_KEYS`].
pub fn mask_config(config: &mut Value) {
    mask_value(config, None, PUBLIC_CONFIG_KEYS);
}

/// Masks `value`, which is stored under `key` (`None` at the top level). `public` is
/// the list of keys that may be shown at this level.
fn mask_value(value: &mut Value, key: Option<&str>, public: &'static [&'static str]) {
    match value {
        Value::Object(map) => {
            // Inside `headers` the keys are header names, with their own short list
            let public = if key.is_some_and(|k| k.eq_ignore_ascii_case("headers")) {
                PUBLIC_HEADERS
            } else {
                public
            };
            for (child_key, child) in map.iter_mut() {
                mask_value(child, Some(child_key), public);
            }
        }
        Value::Array(items) => items
            .iter_mut()
            .for_each(|item| mask_value(item, key, public)),
        Value::String(s) => {
            // An unset value stays visibly unset
            if s.is_empty() {
                return;
            }
            if key.is_some_and(|k| is_public(k, public)) {
                if let Some(masked) = mask_public_string(s) {
                    *s = masked;
                }
            } else {
                *s = MASK.to_string();
            }
        }
        Value::Number(_) => {
            if !key.is_some_and(|k| is_public(k, public)) {
                *value = Value::String(MASK.to_string());
            }
        }
        Value::Bool(_) | Value::Null => {}
    }
}

/// Puts a URL's stored credentials back where an update carries the mask.
fn restore_url(new: &str, old: &str) -> Option<String> {
    let mut new_url = url::Url::parse(new).ok()?;
    let old_url = url::Url::parse(old).ok()?;
    let mut changed = false;

    if new_url.password() == Some(MASK) {
        new_url.set_password(old_url.password()).ok()?;
        changed = true;
    }

    let pairs: Vec<(String, String)> = new_url
        .query_pairs()
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();
    if pairs.iter().any(|(_, v)| v == MASK) {
        let old_pairs: Vec<(String, String)> = old_url
            .query_pairs()
            .map(|(k, v)| (k.into_owned(), v.into_owned()))
            .collect();
        new_url
            .query_pairs_mut()
            .clear()
            .extend_pairs(pairs.iter().map(|(k, v)| {
                let stored = old_pairs
                    .iter()
                    .find(|(old_key, _)| old_key == k)
                    .map(|(_, old_value)| old_value.as_str());
                match stored {
                    Some(stored) if v == MASK => (k.as_str(), stored),
                    _ => (k.as_str(), v.as_str()),
                }
            }));
        changed = true;
    }

    changed.then(|| new_url.to_string())
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
        // The mask stands for whatever was stored, a string or (masked) a number
        (incoming @ Value::String(_), old) if incoming.as_str() == Some(MASK) => {
            if matches!(old, Value::String(_) | Value::Number(_)) {
                *incoming = old.clone();
            }
        }
        (Value::String(new), Value::String(old)) => {
            if let Some(restored) = restore_url(new, old) {
                *new = restored;
            }
        }
        _ => {}
    }
}

/// A connection as the API returns it: credentials masked.
pub fn connection(mut connection: Connection) -> Connection {
    mask_config(&mut connection.config);
    connection.credentials = connection.credentials.map(|_| MASK.to_string());
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
        // A step can print a secret encoded as easily as plain
        let encoded: Vec<String> = needles.iter().flat_map(|n| encoded_forms(n)).collect();
        needles.extend(encoded);
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

    /// Scrubs a run's error message, loading the run's secrets only when there is one.
    pub async fn scrub_run_error(
        pool: &sqlx::PgPool,
        run_id: stormchaser_model::RunId,
        error: Option<String>,
    ) -> Result<Option<String>, sqlx::Error> {
        match error {
            Some(error) => Ok(Some(Self::load(pool, run_id).await?.text(&error))),
            None => Ok(None),
        }
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

/// Encoded fragments shorter than this are not scrubbed: they are short enough to turn
/// up by chance in unrelated text.
const MIN_ENCODED_LEN: usize = 8;

/// The ways a secret commonly reaches output other than as itself: base64 (standard and
/// URL-safe, including as part of a longer encoded string such as an HTTP Basic
/// credential), percent-encoding, JSON string escaping and hex.
fn encoded_forms(secret: &str) -> Vec<String> {
    use base64::Engine;
    let bytes = secret.as_bytes();
    let mut forms = Vec::new();

    for engine in [
        &base64::engine::general_purpose::STANDARD,
        &base64::engine::general_purpose::URL_SAFE,
    ] {
        // Encoded on its own, the whole encoding (padding aside) is the secret's
        forms.push(engine.encode(bytes).trim_end_matches('=').to_string());
        // Inside a longer string the secret starts at one of three offsets within a
        // 3-byte group; only the characters that come from the secret's bits alone are
        // the same whatever surrounds it
        for offset in 1..3 {
            let mut shifted = vec![0u8; offset];
            shifted.extend_from_slice(bytes);
            let encoded = engine.encode(&shifted);
            let first = (8 * offset).div_ceil(6);
            let last = (8 * shifted.len()) / 6;
            if last > first {
                forms.push(encoded[first..last].to_string());
            }
        }
        // The tail is shared by all three alignments only up to the last whole group
        let whole = (8 * bytes.len()) / 6;
        forms.push(engine.encode(bytes)[..whole].to_string());
    }

    forms.push(urlencoding::encode(secret).into_owned());
    forms.push(secret.replace(' ', "+"));
    if let Ok(json) = serde_json::to_string(secret) {
        forms.push(json[1..json.len() - 1].to_string());
    }
    forms.push(hex::encode(bytes));

    forms.retain(|form| form.len() >= MIN_ENCODED_LEN && form != secret);
    forms
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
    fn a_credential_under_a_key_nobody_listed_is_still_masked() {
        let mut config = json!({
            "bucket": "artifacts",
            "port": 5432,
            "force_path_style": true,
            // None of these names says "secret"
            "sas": "sv=2022&sig=abc123",
            "pat": "ghp_exampleexample",
            "key": "AccountKeyValue==",
            "pin": 482913,
            "connection_string": "Server=db;Pwd=hunter2",
            "headers": { "X-Api-Key": "k-123", "X-Auth": "a-456", "Content-Type": "application/json" },
            "auths": { "ghcr.io": { "auth": "dXNlcjpwYXNz", "username": "ci" } },
            "extra": { "nested": ["one-secret", "two-secret"] },
        });
        mask_config(&mut config);

        let text = config.to_string();
        for leaked in [
            "abc123",
            "ghp_",
            "AccountKey",
            "482913",
            "hunter2",
            "k-123",
            "a-456",
            "dXNlcjpwYXNz",
            "one-secret",
            "two-secret",
        ] {
            assert!(!text.contains(leaked), "{leaked} leaked: {text}");
        }
        // What says where the connection points is still readable
        assert_eq!(config["bucket"], "artifacts");
        assert_eq!(config["port"], 5432);
        assert_eq!(config["force_path_style"], true);
        assert_eq!(config["headers"]["Content-Type"], "application/json");
        assert_eq!(config["auths"]["ghcr.io"]["username"], "ci");
    }

    #[test]
    fn credentials_inside_a_url_or_connection_string_are_masked() {
        let mut config = json!({
            "url": "https://store.example.com/c?sv=2022&sig=abc123&sslmode=require",
            "base_url": "https://api.example.com/v1?access_token=tok-1",
            "endpoint": "host=db.internal user=app password=hunter2",
            "repo_url": "https://ci:ghp_tok@github.com/org/repo.git",
        });
        mask_config(&mut config);

        let text = config.to_string();
        for leaked in ["abc123", "tok-1", "hunter2", "ghp_tok"] {
            assert!(!text.contains(leaked), "{leaked} leaked: {text}");
        }
        let url = config["url"].as_str().unwrap();
        assert!(url.starts_with("https://store.example.com/c?"), "{url}");
        assert!(url.contains("sslmode=require"), "{url}");
        assert_eq!(
            config["repo_url"],
            "https://ci:********@github.com/org/repo.git"
        );
    }

    #[test]
    fn saving_a_masked_config_keeps_everything_that_was_masked() {
        let stored = json!({
            "bucket": "artifacts",
            "pat": "ghp_exampleexample",
            "pin": 482913,
            "url": "https://store.example.com/c?sv=2022&sig=abc123&sslmode=require",
            "headers": { "X-Api-Key": "k-123" },
            "extra": { "nested": ["one-secret", "two-secret"] },
        });
        let mut edited = stored.clone();
        mask_config(&mut edited);
        assert_ne!(edited, stored);
        edited["bucket"] = json!("artifacts-v2");

        restore_masked(&mut edited, &stored);

        let mut expected = stored.clone();
        expected["bucket"] = json!("artifacts-v2");
        // Query order and encoding are the URL library's; compare the parameters
        let params = |v: &Value| {
            let mut pairs: Vec<(String, String)> = url::Url::parse(v["url"].as_str().unwrap())
                .unwrap()
                .query_pairs()
                .map(|(k, v)| (k.into_owned(), v.into_owned()))
                .collect();
            pairs.sort();
            pairs
        };
        assert_eq!(params(&edited), params(&expected));
        edited["url"] = json!(null);
        expected["url"] = json!(null);
        assert_eq!(edited, expected);
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

    #[test]
    fn encoded_secrets_are_scrubbed_too() {
        use base64::Engine;
        let secret = "hunter2-correct-horse";
        let redactor = RunRedactor::new(vec![secret.to_string()], &json!({}), &json!({}), None);
        let b64 = base64::engine::general_purpose::STANDARD;

        for (label, text) in [
            ("base64", b64.encode(secret)),
            (
                "url-safe base64",
                base64::engine::general_purpose::URL_SAFE.encode(secret),
            ),
            (
                "percent",
                urlencoding::encode(&format!("{secret} x")).into_owned(),
            ),
            ("hex", hex::encode(secret)),
        ] {
            let out = redactor.text(&format!("value: {text}"));
            assert!(out.contains(MASK), "{label}: {out}");
        }

        // Part of a longer encoding, at each alignment: an HTTP Basic credential
        for user in ["a", "ab", "abc"] {
            let basic = b64.encode(format!("{user}:{secret}"));
            let out = redactor.text(&format!("Authorization: Basic {basic}"));
            assert!(out.contains(MASK), "user {user:?}: {out}");
            assert!(!out.contains(&basic), "user {user:?}: {out}");
        }

        // JSON-escaped
        let quoted = RunRedactor::new(
            vec!["pa\"ss\nword".to_string()],
            &json!({}),
            &json!({}),
            None,
        );
        let out = quoted.text(r#"{"password":"pa\"ss\nword"}"#);
        assert!(out.contains(MASK), "{out}");
    }

    #[test]
    fn short_secrets_do_not_mask_their_encodings_everywhere() {
        // "abcd" encodes to 8 characters at most; only exact forms are kept
        let redactor = RunRedactor::new(vec!["abcd".to_string()], &json!({}), &json!({}), None);
        assert_eq!(redactor.text("YWJj is not it"), "YWJj is not it");
    }
}
