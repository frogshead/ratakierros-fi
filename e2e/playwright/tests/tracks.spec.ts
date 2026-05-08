import { test, expect } from '@playwright/test';

// Browser-driven test for the regression class that 744b2ff (tower-http 0.4 → 0.5)
// belonged to: cargo test stayed green, but the actual page → API fetch broke.
// We need to see the seed track render in the DOM.
test('home page lists at least one track and opens its detail panel', async ({ page }) => {
  const consoleErrors: string[] = [];
  const pageErrors: Error[] = [];

  page.on('console', msg => {
    if (msg.type() === 'error') consoleErrors.push(msg.text());
  });
  page.on('pageerror', err => {
    pageErrors.push(err);
  });

  await page.goto('/');

  // Geolocation is denied by default in CI Chromium → app falls back to loadTracks()
  // which fetches the full list. Wait for at least one track-card to appear.
  const firstCard = page.locator('.track-card').first();
  await expect(firstCard).toBeVisible({ timeout: 15_000 });

  // The seed includes a Helsinki fixture; verify it actually rendered (proves
  // the data round-tripped DB → /api/tracks → fetch → render).
  await expect(page.locator('.track-card-name', { hasText: /helsinki/i }).first())
    .toBeVisible();

  // Click → detail panel populates.
  await firstCard.click();
  await expect(page.locator('#detail-title')).toHaveText(/.+/, { timeout: 10_000 });

  // No console / page errors should have fired during the flow. A strict-mode
  // CORS rejection or a thrown render error would land here.
  expect(pageErrors, `pageerror: ${pageErrors.map(e => e.message).join(' | ')}`)
    .toHaveLength(0);
  expect(consoleErrors, `console.error: ${consoleErrors.join(' | ')}`)
    .toHaveLength(0);
});
