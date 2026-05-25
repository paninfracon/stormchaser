import { test, expect } from '@playwright/test';

test.describe('Workflow Runs', () => {
  test('should render git workflow inputs successfully', async ({ page }) => {

    await page.route('**/api/*', async route => {
      const url = route.request().url();
      if (url.includes('fetch_connections') || url.includes('FetchConnections')) {
        await route.fulfill({
          status: 200,
          contentType: 'application/json',
          body: JSON.stringify([{
              id: "58d8f2fc-aed9-4242-8eb9-fe8323fa25d1",
              name: "My Git Repo",
              description: null,
              connection_type: "git",
              config: { repo_url: "https://github.com/example/repo" },
              encrypted_credentials: null,
              aws_assume_role_arn: null,
              is_default_sfs: false,
              ca_cert: null,
              client_cert: null,
              client_key: null,
              created_at: "2024-01-01T00:00:00Z",
              updated_at: "2024-01-01T00:00:00Z"
          }])
        });
      } else if (url.includes('fetch_dsl_from_git') || url.includes('FetchDslFromGit')) {
        await route.fulfill({
          status: 200,
          contentType: 'application/json',
          body: JSON.stringify("mock dsl content")
        });
      } else if (url.includes('parse_dsl') || url.includes('ParseDsl')) {
        await route.fulfill({
          status: 200,
          contentType: 'application/json',
          body: JSON.stringify({
              inputs_schema: {
                  type: "object",
                  properties: {
                      message: { type: "string" }
                  }
              },
              inputs: {},
              queries: [],
              inputs_view: null
          })
        });
      } else {
          await route.continue();
      }
    });

    await page.goto('/');

    await expect(async () => {
      await page.locator('button', { hasText: 'Create Run' }).click();
      await expect(page.locator('.modal-content')).toBeVisible({ timeout: 1000 });
    }).toPass();

    await page.locator('button', { hasText: 'From Git' }).click();

    // Select the git repo
    const select = page.locator('select.input-field').first();
    await expect(select).toBeVisible();

    // Force select
    await select.selectOption({ label: 'My Git Repo' });

    // Fill in workflow path
    await page.locator('input[placeholder=".stormchaser/workflows/main.storm"]').fill('test.storm');

    // Fill in git ref
    await page.locator('input[placeholder="main"]').fill('main');

    // Click Load Form
    await page.locator('button', { hasText: 'Load Form' }).click();

    // Verify it loads inputs (SchemaForm renders "Message" instead of "message (string)")
    await expect(page.locator('label', { hasText: 'Message' })).toBeVisible({ timeout: 5000 });
  });

  test('should render run details and handle actions', async ({ page }) => {
    let approveCalled = false;
    let deleteCalled = false;

    await page.route('**/api/*', async route => {
      const url = route.request().url();
      if (url.includes('fetch_workflow_runs') || url.includes('FetchWorkflowRuns')) {
        await route.fulfill({
          status: 200,
          contentType: 'application/json',
          body: JSON.stringify([{
            id: "11111111-1111-1111-1111-111111111111",
            workflow_name: "test-actions",
            initiating_user: "u",
            repo_url: "https://github.com/example/repo",
            workflow_path: "main.storm",
            git_ref: "main",
            version: 1,
            status: "running",
            created_at: "2024-01-01T00:00:00Z",
            updated_at: "2024-01-01T00:00:00Z",
            started_resolving_at: null,
            started_at: "2024-01-01T00:00:00Z",
            finished_at: null,
            error: null,
            inputs: {},
            secrets: {}
          }])
        });
      } else if (url.includes('fetch_workflow_run_detail') || url.includes('FetchWorkflowRunDetail')) {
        await route.fulfill({
          status: 200,
          contentType: 'application/json',
          body: JSON.stringify({
            detail: {
              id: "11111111-1111-1111-1111-111111111111",
              workflow_name: "test-actions",
              initiating_user: "u",
              repo_url: "https://github.com/example/repo",
              workflow_path: "main.storm",
              git_ref: "main",
              version: 1,
              status: "running",
              created_at: "2024-01-01T00:00:00Z",
              updated_at: "2024-01-01T00:00:00Z",
              started_resolving_at: null,
              started_at: "2024-01-01T00:00:00Z",
              finished_at: null,
              error: null,
              inputs: {},
              secrets: {}
            },
            steps: [{
              instance: {
                id: "step-123",
                step_name: "approval-step",
                status: "waiting_for_event",
                requires_approval: true,
                depends_on: [],
                started_at: "2024-01-01T00:01:00Z",
                finished_at: null
              },
              outputs: [],
              history: [],
              logs: []
            }],
            artifacts: [],
            test_summaries: [],
            test_cases: []
          })
        });
      } else if (url.includes('approve_step') || url.includes('ApproveStep')) {
        approveCalled = true;
        await route.fulfill({ status: 200, body: JSON.stringify(null) });
      } else if (url.includes('delete_workflow_run') || url.includes('DeleteWorkflowRun')) {
        deleteCalled = true;
        await route.fulfill({ status: 200, body: JSON.stringify(null) });
      } else {
        await route.continue();
      }
    });

    await page.goto('/approvals');
    await page.locator('a', { hasText: 'Runs' }).click();

    // Click on the run to view details
    await page.locator('text=test-actions').click();

    // Verify step shows up
    await expect(page.locator('text=approval-step')).toBeVisible();

    // Verify Approve and Reject buttons appear
    await expect(page.locator('button', { hasText: 'Approve' })).toBeVisible();
    await expect(page.locator('button', { hasText: 'Reject' })).toBeVisible();

    // Click Approve
    await page.locator('button', { hasText: 'Approve' }).click();
    expect(approveCalled).toBeTruthy();

    // Verify Delete Run button
    await expect(page.locator('button', { hasText: 'Delete Run' })).toBeVisible();
    await page.locator('button', { hasText: 'Delete Run' }).click();
    expect(deleteCalled).toBeTruthy();
  });

  test('should sort workflow runs in the table', async ({ page }) => {
    await page.route('**/api/*', async route => {
      const url = route.request().url();
      if (url.includes('fetch_workflow_runs') || url.includes('FetchWorkflowRuns')) {
        await route.fulfill({
          status: 200,
          contentType: 'application/json',
          body: JSON.stringify([
            {
              id: "1",
              workflow_name: "Z-workflow",
              initiating_user: "alice",
              repo_url: "",
              workflow_path: "",
              git_ref: "",
              version: 1,
              status: "running",
              created_at: "2024-01-02T00:00:00Z",
              updated_at: "2024-01-02T00:00:00Z",
              started_resolving_at: null,
              started_at: null,
              finished_at: null,
              error: null,
              inputs: {},
              secrets: {}
            },
            {
              id: "2",
              workflow_name: "A-workflow",
              initiating_user: "bob",
              repo_url: "",
              workflow_path: "",
              git_ref: "",
              version: 1,
              status: "running",
              created_at: "2024-01-01T00:00:00Z",
              updated_at: "2024-01-01T00:00:00Z",
              started_resolving_at: null,
              started_at: null,
              finished_at: null,
              error: null,
              inputs: {},
              secrets: {}
            }
          ])
        });
      } else {
        await route.continue();
      }
    });

    await page.goto('/approvals');
    await page.locator('a', { hasText: 'Runs' }).click();

    // Default sort is Created descending, so Z-workflow (2024-01-02) is first, A-workflow (2024-01-01) is second
    await expect(page.locator('tbody tr').nth(0)).toContainText('Z-workflow');
    await expect(page.locator('tbody tr').nth(1)).toContainText('A-workflow');

    // Click "Name" to sort ascending
    await page.locator('th', { hasText: 'Name' }).click();
    await expect(page.locator('tbody tr').nth(0)).toContainText('A-workflow');
    await expect(page.locator('tbody tr').nth(1)).toContainText('Z-workflow');

    // Click "Name" again to sort descending
    await page.locator('th', { hasText: 'Name' }).click();
    await expect(page.locator('tbody tr').nth(0)).toContainText('Z-workflow');
    await expect(page.locator('tbody tr').nth(1)).toContainText('A-workflow');
  });
});
