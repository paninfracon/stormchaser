//! Operator allow-lists for what the engine does with its own AWS identity.
//!
//! Role ARNs and Lambda function names come from workflows and connections, which any
//! API user can write. Without a limit, the engine would assume any role that trusts it,
//! or invoke any function it can reach, on a workflow author's say-so.

/// Comma-separated role ARNs the engine may assume. A trailing `*` matches a prefix.
/// Empty by default, which refuses every assume-role.
pub const ALLOWED_ROLE_ARNS_ENV: &str = "STORMCHASER_AWS_ALLOWED_ROLE_ARNS";

/// Comma-separated Lambda function names or ARNs the engine may invoke with its own
/// credentials. A trailing `*` matches a prefix. Empty by default.
pub const ALLOWED_LAMBDA_FUNCTIONS_ENV: &str = "STORMCHASER_AWS_ALLOWED_LAMBDA_FUNCTIONS";

fn is_allowed(allow_list: &str, value: &str) -> bool {
    allow_list
        .split(',')
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .any(|entry| match entry.strip_suffix('*') {
            Some(prefix) => value.starts_with(prefix),
            None => entry == value,
        })
}

/// Refuses a role ARN the operator has not allow-listed.
pub fn ensure_role_allowed(role_arn: &str) -> anyhow::Result<()> {
    let allow_list = std::env::var(ALLOWED_ROLE_ARNS_ENV).unwrap_or_default();
    if is_allowed(&allow_list, role_arn) {
        Ok(())
    } else {
        anyhow::bail!(
            "AWS role {} is not in {}; the engine will not assume it",
            role_arn,
            ALLOWED_ROLE_ARNS_ENV
        )
    }
}

/// Refuses a Lambda function the operator has not allow-listed for the engine's own
/// credentials. A function invoked through an allow-listed role is governed by that role.
pub fn ensure_lambda_allowed(function_name: &str) -> anyhow::Result<()> {
    let allow_list = std::env::var(ALLOWED_LAMBDA_FUNCTIONS_ENV).unwrap_or_default();
    if is_allowed(&allow_list, function_name) {
        Ok(())
    } else {
        anyhow::bail!(
            "Lambda function {} is not in {}; the engine will not invoke it with its own credentials",
            function_name,
            ALLOWED_LAMBDA_FUNCTIONS_ENV
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_list_allows_nothing() {
        assert!(!is_allowed("", "arn:aws:iam::111111111111:role/deploy"));
        assert!(!is_allowed(" , ", "arn:aws:iam::111111111111:role/deploy"));
    }

    #[test]
    fn entries_match_exactly_or_by_prefix() {
        let list =
            "arn:aws:iam::111111111111:role/deploy, arn:aws:iam::222222222222:role/stormchaser-*";
        assert!(is_allowed(list, "arn:aws:iam::111111111111:role/deploy"));
        assert!(is_allowed(
            list,
            "arn:aws:iam::222222222222:role/stormchaser-tf"
        ));
        assert!(!is_allowed(
            list,
            "arn:aws:iam::111111111111:role/deploy-admin"
        ));
        assert!(!is_allowed(
            list,
            "arn:aws:iam::333333333333:role/stormchaser-tf"
        ));
        assert!(!is_allowed(list, "arn:aws:iam::222222222222:role/admin"));
    }

    #[test]
    fn a_role_is_refused_unless_listed() {
        // The variable is unset in the test environment.
        let err = ensure_role_allowed("arn:aws:iam::111111111111:role/anything").unwrap_err();
        assert!(err.to_string().contains(ALLOWED_ROLE_ARNS_ENV));
        let err = ensure_lambda_allowed("billing-export").unwrap_err();
        assert!(err.to_string().contains(ALLOWED_LAMBDA_FUNCTIONS_ENV));
    }
}
