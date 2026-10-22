//! Every step type Stormchaser knows, and who carries a step of that type out.
//!
//! The engine, the runners, the schema generator and the docs all name step types; this
//! list is the one they agree on. A step whose type is not here can still run, if a
//! WASM module is registered for its type.

/// Who carries out a step of a given type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Executor {
    /// The engine runs it itself.
    Engine,
    /// A runner runs it as a container or a Kubernetes Job.
    Runner,
    /// The engine turns it into a step of another type when it dispatches it
    /// (`GitCheckout` and the Terraform steps become `RunContainer` steps).
    Rewritten,
    /// It waits for a person or an event, and is never dispatched.
    Waits,
    /// Parsed, but nothing runs it yet.
    Planned,
}

/// A step type and who carries it out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StepType {
    /// The name a workflow gives it: `step "name" "<type>"`.
    pub name: &'static str,
    /// Who carries it out.
    pub executor: Executor,
}

impl StepType {
    /// True for steps that wait for a person or an event instead of being dispatched.
    pub fn waits(&self) -> bool {
        self.executor == Executor::Waits
    }
}

const fn t(name: &'static str, executor: Executor) -> StepType {
    StepType { name, executor }
}

/// Every built-in step type.
pub const STEP_TYPES: &[StepType] = &[
    t("RunContainer", Executor::Runner),
    t("RunK8sJob", Executor::Runner),
    t("GitCheckout", Executor::Rewritten),
    t("TerraformPlan", Executor::Rewritten),
    t("TerraformApply", Executor::Rewritten),
    t("TerraformApproval", Executor::Rewritten),
    t("RestApi", Executor::Engine),
    t("SqlExecute", Executor::Engine),
    t("JQ", Executor::Engine),
    t("JinjaRender", Executor::Engine),
    t("WebhookInvoke", Executor::Engine),
    t("Wasm", Executor::Engine),
    t("LambdaInvoke", Executor::Engine),
    t("EmailSend", Executor::Engine),
    t("TestReportEmail", Executor::Engine),
    t("SlackMessage", Executor::Engine),
    t("TeamsMessage", Executor::Engine),
    t("Approval", Executor::Waits),
    t("Wait", Executor::Waits),
    t("Parallel", Executor::Planned),
    t("Sequential", Executor::Planned),
];

/// The built-in step type named `name`, if there is one. Names are matched exactly.
pub fn step_type(name: &str) -> Option<&'static StepType> {
    STEP_TYPES.iter().find(|t| t.name == name)
}

/// True when `name` is a built-in step type in any letter case, so that a registered
/// WASM module cannot take it over by spelling it differently.
pub fn is_builtin(name: &str) -> bool {
    STEP_TYPES.iter().any(|t| t.name.eq_ignore_ascii_case(name))
}

/// True for steps that wait for a person or an event instead of being dispatched.
pub fn waits(name: &str) -> bool {
    step_type(name).is_some_and(StepType::waits)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_type_is_listed_once() {
        for (i, a) in STEP_TYPES.iter().enumerate() {
            assert!(
                !STEP_TYPES[i + 1..]
                    .iter()
                    .any(|b| b.name.eq_ignore_ascii_case(a.name)),
                "{} is listed twice",
                a.name
            );
        }
    }

    #[test]
    fn lookups() {
        assert_eq!(step_type("JQ").map(|t| t.executor), Some(Executor::Engine));
        assert!(step_type("jq").is_none());
        assert!(is_builtin("jq"));
        assert!(waits("Approval") && waits("Wait") && !waits("RunContainer"));
        assert!(step_type("WaitEvent").is_none());
    }
}
