use anyhow::Result;
use quick_xml::de::from_str;
use serde::Deserialize;
use serde_json::Value;
use stormchaser_model::{TestCase, TestCaseStatus, TestSummary};
use uuid::Uuid;

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct TestCaseXml {
    #[serde(rename = "@name")]
    name: String,
    #[serde(rename = "@classname")]
    classname: Option<String>,
    #[serde(rename = "@time")]
    time: Option<f64>,
    failure: Option<TestFailureXml>,
    error: Option<TestFailureXml>,
    skipped: Option<Value>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct TestFailureXml {
    #[serde(rename = "@message")]
    message: Option<String>,
    #[serde(rename = "$value")]
    content: Option<String>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct TestSuite {
    #[serde(rename = "@name")]
    name: Option<String>,
    #[serde(rename = "@tests")]
    tests: Option<i32>,
    #[serde(rename = "@failures")]
    failures: Option<i32>,
    #[serde(rename = "@errors")]
    errors: Option<i32>,
    #[serde(rename = "@skipped")]
    skipped: Option<i32>,
    #[serde(rename = "@time")]
    time: Option<f64>,
    #[serde(rename = "testcase", default)]
    testcases: Vec<TestCaseXml>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct TestSuites {
    #[serde(rename = "testsuite", default)]
    testsuites: Vec<TestSuite>,
    #[serde(rename = "@tests")]
    tests: Option<i32>,
    #[serde(rename = "@failures")]
    failures: Option<i32>,
    #[serde(rename = "@errors")]
    errors: Option<i32>,
    #[serde(rename = "@time")]
    time: Option<f64>,
}

pub fn parse_junit(
    content: &str,
    report_name: &str,
    run_id: Uuid,
    step_id: Uuid,
) -> Result<(TestSummary, Vec<TestCase>)> {
    let mut summary = TestSummary {
        id: Uuid::new_v4(),
        run_id,
        step_instance_id: step_id,
        report_name: report_name.to_string(),
        ..Default::default()
    };
    let mut test_cases = Vec::new();

    let suites = if content.contains("<testsuites") {
        from_str::<TestSuites>(content).ok().map(|s| s.testsuites)
    } else if content.contains("<testsuite") {
        from_str::<TestSuite>(content).ok().map(|s| vec![s])
    } else {
        None
    };

    if let Some(suites) = suites {
        for suite in suites {
            let suite_name = suite.name.clone();
            summary.total_tests += suite.tests.unwrap_or(suite.testcases.len() as i32);
            summary.failed += suite.failures.unwrap_or(0);
            summary.errors += suite.errors.unwrap_or(0);
            summary.skipped += suite.skipped.unwrap_or(0);
            summary.duration_ms += (suite.time.unwrap_or(0.0) * 1000.0) as i64;

            for tc in suite.testcases {
                let status = if tc.failure.is_some() {
                    TestCaseStatus::Failed
                } else if tc.error.is_some() {
                    TestCaseStatus::Error
                } else if tc.skipped.is_some() {
                    TestCaseStatus::Skipped
                } else {
                    TestCaseStatus::Passed
                };

                let message = tc
                    .failure
                    .as_ref()
                    .or(tc.error.as_ref())
                    .and_then(|f| f.message.clone().or_else(|| f.content.clone()));

                test_cases.push(TestCase {
                    id: Uuid::new_v4(),
                    run_id,
                    step_instance_id: step_id,
                    report_name: report_name.to_string(),
                    test_suite: suite_name.clone(),
                    test_case: tc.name,
                    status,
                    duration_ms: tc.time.map(|t| (t * 1000.0) as i64),
                    message,
                    created_at: chrono::Utc::now(),
                });
            }
        }
        summary.passed = summary.total_tests - summary.failed - summary.errors - summary.skipped;
    }

    Ok((summary, test_cases))
}

pub fn aggregate_summaries(summaries: &[TestSummary]) -> Option<TestSummary> {
    if summaries.is_empty() {
        return None;
    }

    let mut first = summaries[0].clone();
    for s in &summaries[1..] {
        first.total_tests += s.total_tests;
        first.passed += s.passed;
        first.failed += s.failed;
        first.skipped += s.skipped;
        first.errors += s.errors;
        first.duration_ms += s.duration_ms;
    }
    Some(first)
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[test]
    fn test_parse_junit_suites() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<testsuites tests="3" failures="1" errors="1" time="1.5">
    <testsuite name="suite1" tests="2" failures="1" errors="0" skipped="0" time="1.0">
        <testcase name="test1" classname="c1" time="0.5"/>
        <testcase name="test2" classname="c1" time="0.5">
            <failure message="failed">detail</failure>
        </testcase>
    </testsuite>
    <testsuite name="suite2" tests="1" failures="0" errors="1" skipped="0" time="0.5">
        <testcase name="test3" classname="c2" time="0.5">
            <error message="error">detail</error>
        </testcase>
    </testsuite>
</testsuites>"#;
        let run_id = Uuid::new_v4();
        let step_id = Uuid::new_v4();
        let (summary, cases) = parse_junit(xml, "test-report", run_id, step_id).unwrap();

        assert_eq!(summary.total_tests, 3);
        assert_eq!(summary.failed, 1);
        assert_eq!(summary.errors, 1);
        assert_eq!(summary.passed, 1);
        assert_eq!(summary.duration_ms, 1500);
        assert_eq!(cases.len(), 3);
        assert_eq!(cases[0].status, TestCaseStatus::Passed);
        assert_eq!(cases[1].status, TestCaseStatus::Failed);
        assert_eq!(cases[2].status, TestCaseStatus::Error);
    }

    #[test]
    fn test_parse_junit_single_suite() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<testsuite name="suite1" tests="2" failures="0" errors="0" skipped="1" time="0.8">
    <testcase name="test1" classname="c1" time="0.4"/>
    <testcase name="test2" classname="c1" time="0.4">
        <skipped/>
    </testcase>
</testsuite>"#;
        let run_id = Uuid::new_v4();
        let step_id = Uuid::new_v4();
        let (summary, cases) = parse_junit(xml, "test-report", run_id, step_id).unwrap();

        assert_eq!(summary.total_tests, 2);
        assert_eq!(summary.skipped, 1);
        assert_eq!(summary.passed, 1);
        assert_eq!(summary.duration_ms, 800);
        assert_eq!(cases.len(), 2);
        assert_eq!(cases[1].status, TestCaseStatus::Skipped);
    }

    #[test]
    fn test_aggregate_summaries() {
        let s1 = TestSummary {
            total_tests: 10,
            passed: 8,
            failed: 2,
            duration_ms: 1000,
            ..Default::default()
        };
        let s2 = TestSummary {
            total_tests: 5,
            passed: 4,
            failed: 1,
            duration_ms: 500,
            ..Default::default()
        };
        let agg = aggregate_summaries(&[s1, s2]).unwrap();
        assert_eq!(agg.total_tests, 15);
        assert_eq!(agg.passed, 12);
        assert_eq!(agg.failed, 3);
        assert_eq!(agg.duration_ms, 1500);
    }
}
