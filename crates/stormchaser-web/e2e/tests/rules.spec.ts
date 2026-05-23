import { test, expect } from '@playwright/test';

test.describe('Rules', () => {
  test('should display the rules page', async ({ page }) => {
    await page.goto('/rules');

    await expect(page.locator('h2', { hasText: 'Event Rules' })).toBeVisible();
    await expect(page.locator('text=No event rules configured.').or(page.locator('table'))).toBeVisible();
  });

  test('should open Create Rule modal', async ({ page }) => {
    await page.goto('/rules');

    await expect(async () => {
      await page.locator('button', { hasText: 'Create Rule' }).click();
      await expect(page.locator('.modal-content')).toBeVisible({ timeout: 1000 });
    }).toPass();

      await expect(page.locator('h2', { hasText: 'Create Event Rule' })).toBeVisible();

    // Close modal
    await page.locator('button[title="Close"]').click();
    await expect(page.locator('.modal-content')).not.toBeVisible();
  });
});
