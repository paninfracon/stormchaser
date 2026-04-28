use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Serialize, Deserialize, Clone, sqlx::FromRow)]
pub struct TestReport {
    pub id: Uuid,
    pub run_id: Uuid,
    pub step_instance_id: Uuid,
    pub report_name: String,
    pub file_name: String,
    pub format: String,
    pub content: Option<String>,
    pub backend_id: Option<Uuid>,
    pub remote_path: Option<String>,
    pub checksum: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Serialize, Deserialize, Clone, sqlx::FromRow, Default)]
pub struct TestSummary {
    pub id: Uuid,
    pub run_id: Uuid,
    pub step_instance_id: Uuid,
    pub report_name: String,
    pub total_tests: i32,
    pub passed: i32,
    pub failed: i32,
    pub skipped: i32,
    pub errors: i32,
    pub duration_ms: i64,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Serialize, Deserialize, Clone, sqlx::Type, PartialEq, Eq)]
#[sqlx(type_name = "test_case_status", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum TestCaseStatus {
    Passed,
    Failed,
    Skipped,
    Error,
}

#[derive(Debug, Serialize, Deserialize, Clone, sqlx::FromRow)]
pub struct TestCase {
    pub id: Uuid,
    pub run_id: Uuid,
    pub step_instance_id: Uuid,
    pub report_name: String,
    pub test_suite: Option<String>,
    pub test_case: String,
    pub status: TestCaseStatus,
    pub duration_ms: Option<i64>,
    pub message: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}
