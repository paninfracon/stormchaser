import { test, expect } from '@playwright/test';

test.describe('Backend Connections', () => {
  test('should display the backends page and empty state or table', async ({ page }) => {
    await page.goto('/backends');

    await expect(page.locator('h2', { hasText: 'Backend Connections' })).toBeVisible();

    // Check for empty state or table
    await expect(page.locator('text=No backend connections found.').or(page.locator('table'))).toBeVisible();
  });

  test('should open Create Backend modal', async ({ page }) => {
    await page.goto('/backends');

    await expect(async () => {
      await page.locator('button', { hasText: 'Create Connection' }).click();
      await expect(page.locator('.modal-content')).toBeVisible({ timeout: 1000 });
    }).toPass();

    await expect(page.locator('h2', { hasText: 'Create Backend Connection' })).toBeVisible();

    // Close modal
    await page.locator('button[title="Close"]').click();
    await expect(page.locator('.modal-content')).not.toBeVisible();
  });
});
