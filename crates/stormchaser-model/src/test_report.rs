//! Test reporting and summary models for workflow execution.

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

/// Details of a single test report associated with a step instance.
#[derive(Debug, Serialize, Deserialize, Clone, sqlx::FromRow, ToSchema)]
pub struct TestReport {
    /// Unique identifier for the test report.
    pub id: Uuid,
    /// Associated workflow run ID.
    pub run_id: Uuid,
    /// Associated step instance ID.
    pub step_instance_id: Uuid,
    /// Logical name of the report.
    pub report_name: String,
    /// Original file name of the report.
    pub file_name: String,
    /// Format of the report (e.g., 'junit', 'clover').
    pub format: String,
    /// Raw content of the report, if small enough to be inlined.
    pub content: Option<String>,
    /// Storage backend ID where the full report is stored.
    pub backend_id: Option<Uuid>,
    /// Path to the report in the remote storage backend.
    pub remote_path: Option<String>,
    /// Checksum of the report content.
    pub checksum: String,
    /// Timestamp when the report was recorded.
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// Aggregated summary of test results for a specific report.
#[derive(Debug, Serialize, Deserialize, Clone, sqlx::FromRow, Default, ToSchema)]
pub struct TestSummary {
    /// Unique identifier for the summary.
    pub id: Uuid,
    /// Associated workflow run ID.
    pub run_id: Uuid,
    /// Associated step instance ID.
    pub step_instance_id: Uuid,
    /// Logical name of the report.
    pub report_name: String,
    /// Total number of tests executed.
    pub total_tests: i32,
    /// Number of tests that passed.
    pub passed: i32,
    /// Number of tests that failed.
    pub failed: i32,
    /// Number of tests that were skipped.
    pub skipped: i32,
    /// Number of tests that encountered an error.
    pub errors: i32,
    /// Total duration of the test suite execution in milliseconds.
    pub duration_ms: i64,
    /// Timestamp when the summary was generated.
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// Status of an individual test case.
#[derive(Debug, Serialize, Deserialize, Clone, sqlx::Type, PartialEq, Eq, ToSchema)]
#[sqlx(type_name = "test_case_status", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum TestCaseStatus {
    /// The test case passed successfully.
    Passed,
    /// The test case failed.
    Failed,
    /// The test case was skipped.
    Skipped,
    /// The test case encountered an unexpected error.
    Error,
}

/// Details of an individual test case execution.
#[derive(Debug, Serialize, Deserialize, Clone, sqlx::FromRow, ToSchema)]
pub struct TestCase {
    /// Unique identifier for the test case record.
    pub id: Uuid,
    /// Associated workflow run ID.
    pub run_id: Uuid,
    /// Associated step instance ID.
    pub step_instance_id: Uuid,
    /// Logical name of the report containing this test case.
    pub report_name: String,
    /// Name of the test suite this case belongs to.
    pub test_suite: Option<String>,
    /// Name of the test case.
    pub test_case: String,
    /// Final status of the test case execution.
    pub status: TestCaseStatus,
    /// Duration of the test case execution in milliseconds.
    pub duration_ms: Option<i64>,
    /// Optional error or failure message associated with the test case.
    pub message: Option<String>,
    /// Timestamp when the test case record was created.
    pub created_at: chrono::DateTime<chrono::Utc>,
}
