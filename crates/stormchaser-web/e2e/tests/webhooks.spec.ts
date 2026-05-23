import { test, expect } from '@playwright/test';

test.describe('Webhooks', () => {
  test('should display the webhooks page', async ({ page }) => {
    await page.goto('/webhooks');

    await expect(page.locator('h2', { hasText: 'Webhooks' })).toBeVisible();
    await expect(page.locator('text=No webhooks configured.').or(page.locator('table'))).toBeVisible();
  });

  test('should open Create Webhook modal', async ({ page }) => {
    await page.goto('/webhooks');

    await expect(async () => {
      await page.locator('button', { hasText: 'Create Webhook' }).click();
      await expect(page.locator('.modal-content')).toBeVisible({ timeout: 1000 });
    }).toPass();

    await expect(page.locator('h2', { hasText: 'Create Webhook' })).toBeVisible();

    // Close modal
    await page.locator('button[title="Close"]').click();
    await expect(page.locator('.modal-content')).not.toBeVisible();
  });
});
