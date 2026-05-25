import { test, expect } from '@playwright/test';

test.describe('Cron Schedules', () => {
  test('should display the cron page', async ({ page }) => {
    await page.goto('/cron');

    await expect(page.locator('h2', { hasText: 'Cron Workflows' })).toBeVisible();
    await expect(page.locator('text=No cron workflows configured.').or(page.locator('table'))).toBeVisible();
  });

  test('should open Create Cron modal', async ({ page }) => {
    await page.goto('/cron');

    await expect(async () => {
      await page.locator('button', { hasText: 'Create Cron' }).click();
      await expect(page.locator('.modal-content')).toBeVisible({ timeout: 1000 });
    }).toPass();

      await expect(page.locator('h2', { hasText: 'Create Cron Workflow' })).toBeVisible();

    // Close modal
    await page.locator('button[title="Close"]').click();
    await expect(page.locator('.modal-content')).not.toBeVisible();
  });
});
