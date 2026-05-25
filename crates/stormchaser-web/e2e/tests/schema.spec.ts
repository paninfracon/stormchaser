import { test, expect } from '@playwright/test';

test.describe('Schema Linter', () => {
  test('should parse and display valid DSL', async ({ page }) => {
    await page.route('**/api/*', async route => {
      const url = route.request().url();
      if (url.includes('parse_dsl') || url.includes('ParseDsl')) {
        await route.fulfill({
          status: 200,
          contentType: 'application/json',
          body: JSON.stringify({
            inputs_schema: { type: "object" },
            inputs: {},
            queries: [],
            inputs_view: null
          })
        });
      } else {
        await route.continue();
      }
    });

    await page.goto('/schema');

    const textarea = page.locator('textarea');
    await textarea.fill('workflow "test" {}');

    await page.locator('button', { hasText: 'Lint' }).click();

    await expect(page.locator('text=DSL is valid!')).toBeVisible();
  });

  test('should handle validation errors', async ({ page }) => {
    await page.route('**/api/*', async route => {
      const url = route.request().url();
      if (url.includes('parse_dsl') || url.includes('ParseDsl')) {
        await route.fulfill({
          status: 500,
          contentType: 'application/json',
          body: JSON.stringify({ error: "Invalid syntax" }) // ServerFnError serialization
        });
      } else {
        await route.continue();
      }
    });

    await page.goto('/schema');

    const textarea = page.locator('textarea');
    await textarea.fill('invalid syntax');

    await page.locator('button', { hasText: 'Lint' }).click();

    await expect(page.locator('text=Validation Error:')).toBeVisible();
  });
});
