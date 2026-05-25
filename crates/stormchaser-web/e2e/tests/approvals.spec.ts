import { test, expect } from '@playwright/test';

test.describe('Approvals Tab', () => {
  test('should display running workflows', async ({ page }) => {
    page.on('console', msg => console.log('BROWSER LOG:', msg.text()));
    await page.route('**/api/*', async route => {
      const url = route.request().url();
      if (url.includes('fetch_workflow_runs') || url.includes('FetchWorkflowRuns')) {
        await route.fulfill({
          status: 200,
          contentType: 'application/json',
          body: JSON.stringify([{
            id: "12345678-1234-1234-1234-123456789012",
            workflow_name: "approval-test",
            initiating_user: "admin",
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
      } else {
        await route.continue();
      }
    });

    await page.goto('/');
    await page.locator('a', { hasText: 'Approvals' }).click();

    await expect(page.locator('text=approval-test')).toBeVisible();
    await expect(page.locator('text=Running')).toBeVisible();
  });
});
