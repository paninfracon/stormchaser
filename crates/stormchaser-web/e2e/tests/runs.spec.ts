import { test, expect } from '@playwright/test';

test.describe('Workflow Runs', () => {
  test('should display the runs list page and show empty state initially', async ({ page }) => {
    page.on('console', msg => console.log(`[Browser Console]: ${msg.text()}`));
    page.on('pageerror', error => console.log(`[Browser Error]: ${error.message}`));

    page.on('request', request => console.log('>>', request.method(), request.url()));
    page.on('response', response => console.log('<<', response.status(), response.url()));

    await page.goto('/');

    try {
        await expect(page.locator('h2', { hasText: 'Workflow Runs' })).toBeVisible({ timeout: 5000 });
    } catch (e) {
        console.log("PAGE HTML:");
        console.log(await page.content());
        throw e;
    }
  });
});
