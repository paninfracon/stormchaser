import { test, expect } from '@playwright/test';

test.describe('Security headers and logout', () => {
  test('pages carry the security headers and a policy with a fresh nonce', async ({ request }) => {
    const first = await request.get('/');
    const headers = first.headers();
    expect(headers['x-content-type-options']).toBe('nosniff');
    expect(headers['x-frame-options']).toBe('DENY');
    expect(headers['referrer-policy']).toBe('same-origin');

    const policy = headers['content-security-policy'];
    expect(policy).toContain("frame-ancestors 'none'");
    expect(policy).toContain("object-src 'none'");
    const nonce = /script-src [^;]*'nonce-([^']+)'/.exec(policy)?.[1];
    expect(nonce).toBeTruthy();
    // The inline scripts in the page carry the nonce the header names
    expect(await first.text()).toContain(`nonce="${nonce}"`);

    const second = await request.get('/');
    const secondNonce = /'nonce-([^']+)'/.exec(second.headers()['content-security-policy'])?.[1];
    expect(secondNonce).not.toBe(nonce);
  });

  test('the policy does not break the page', async ({ page }) => {
    const violations: string[] = [];
    page.on('console', (message) => {
      if (message.text().includes('Content Security Policy')) {
        violations.push(message.text());
      }
    });
    page.on('pageerror', (error) => violations.push(error.message));

    await page.goto('/');
    // Hydration ran: the button responds, which needs the inline script and the WASM
    const themeButton = page.locator('button', { hasText: /Mode/ });
    const initialText = await themeButton.innerText();
    await themeButton.click();
    await expect(themeButton).not.toHaveText(initialText);

    await page.click('text=Backends');
    await expect(page).toHaveURL(/\/backends/);
    expect(violations).toEqual([]);
  });

  test('an injected inline script does not run', async ({ page }) => {
    await page.goto('/');
    await page.evaluate(() => {
      const script = document.createElement('script');
      script.textContent = 'window.__injected = true';
      document.body.appendChild(script);
    });
    expect(await page.evaluate(() => (window as any).__injected)).toBeUndefined();
  });

  test('logout needs a POST from this site', async ({ page, request }) => {
    // A link, an image or a prefetch cannot log anyone out
    expect((await request.get('/auth/logout')).status()).toBe(405);
    // Nor can a form on another site
    const crossSite = await request.post('/auth/logout', {
      headers: { 'Sec-Fetch-Site': 'cross-site' },
      maxRedirects: 0,
    });
    expect(crossSite.status()).toBe(403);

    await page.goto('/');
    await page.locator('button', { hasText: 'Logout' }).click();
    await expect(page).toHaveURL(/\/$/);
    await expect(page.locator('text=Login with Dex')).toBeVisible();
  });
});
