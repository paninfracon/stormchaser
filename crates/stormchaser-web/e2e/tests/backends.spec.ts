import { test, expect } from '@playwright/test';

test.describe('Backend Connections', () => {
  test('should display the backends page and empty state or table', async ({ page }) => {
    await page.goto('/backends');
    await expect(page.locator('h2', { hasText: 'Backend Connections' })).toBeVisible();
    await expect(page.locator('text=No backend connections found.').or(page.locator('table'))).toBeVisible();
  });

  test('should open Create Backend modal, fill inputs, and test connection successfully', async ({ page }) => {
    // Intercept API requests based on URL
    await page.route('**/api/*', async route => {
      const url = route.request().url();
      if (url.includes('test_connection') || url.includes('TestConnection')) {
        await route.fulfill({
          status: 200,
          contentType: 'application/json',
          body: JSON.stringify([true, "Connection successful!"]) // leptos returns tuple as array
        });
      } else {
        await route.continue();
      }
    });

    await page.goto('/backends');

    await expect(async () => {
      await page.locator('button', { hasText: 'Create Connection' }).click();
      await expect(page.locator('.modal-content')).toBeVisible({ timeout: 1000 });
    }).toPass();

    await page.locator('.form-group').filter({ hasText: 'Name *' }).locator('input').fill('Test Git Connection');
    await page.locator('.form-group').filter({ hasText: 'Connection Type *' }).locator('select').selectOption('git');
    await page.locator('input[placeholder="https://github.com/..."]').fill('https://github.com/test/test');

    await page.locator('button', { hasText: 'Test Connection' }).click();

    await expect(page.getByText('Connection successful!')).toBeVisible({ timeout: 5000 });

    await page.locator('button[title="Close"]').click();
  });
});
