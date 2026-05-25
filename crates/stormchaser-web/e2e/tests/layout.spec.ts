import { test, expect } from '@playwright/test';

test.describe('Layout and Navigation', () => {
  test('should display the header and title', async ({ page }) => {
    await page.goto('/');
    await expect(page.locator('h1')).toHaveText('Stormchaser');
    await expect(page.locator('text=GitHub')).toBeVisible();
  });

  test('should toggle theme', async ({ page }) => {
    await page.goto('/');
    const themeButton = page.locator('button', { hasText: /Mode/ });

    // Default should be light mode or dark mode depending on system, but let's just click it
    const initialText = await themeButton.innerText();
    await themeButton.click();

    const newText = await themeButton.innerText();
    expect(newText).not.toBe(initialText);
  });

  test('should navigate to all main tabs', async ({ page }) => {
    await page.goto('/');

    // Check initial tab
    await expect(page.locator('.tab-link', { hasText: 'Runs' })).toBeVisible();

    // Navigate to Backends
    await page.click('text=Backends');
    await expect(page).toHaveURL(/\/backends/);

    // Navigate to Webhooks
    await page.click('text=Webhooks');
    await expect(page).toHaveURL(/\/webhooks/);

    // Navigate to Rules
    await page.click('text=Rules');
    await expect(page).toHaveURL(/\/rules/);

    // Navigate to Cron
    await page.click('text=Cron');
    await expect(page).toHaveURL(/\/cron/);
  });
});
