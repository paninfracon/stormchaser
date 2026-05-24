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
});
