import { test, expect } from '@playwright/test';

// Browser-driven test for the Strava-style leaderboard feature:
// the Pörssi cross-track view + the per-track period selector.
// Fixtures live in e2e/seed.sql (3 tracks, 3 users, 6 historical runs).

test('Pörssi link navigates to the leaderboard view and shows seeded entries', async ({ page }) => {
  const consoleErrors: string[] = [];
  page.on('console', msg => { if (msg.type() === 'error') consoleErrors.push(msg.text()); });

  await page.goto('/');

  // Wait for the track list to be present so the app is fully initialised.
  await expect(page.locator('.track-card').first()).toBeVisible({ timeout: 15_000 });

  // Click the Pörssi link.
  await page.locator('#nav-leaderboard').click();
  await expect(page).toHaveURL(/#\/leaderboard$/);

  // Leaderboard table appears with the seeded entries. Bob holds the overall
  // best at 58.40 — that's the row we anchor on. The smoke step that runs
  // before Playwright (e2e/smoke.sh) registers a one-off user and logs a 75.4
  // run, so the row count is non-deterministic; only the seeded ordering is.
  const lbTable = page.locator('.lb-table');
  await expect(lbTable).toBeVisible({ timeout: 10_000 });
  const rows = lbTable.locator('tbody tr');
  // Wait for at least the 3 seeded rows to be present, without asserting an
  // exact count.
  await expect(rows.nth(2)).toBeVisible({ timeout: 10_000 });
  await expect(rows.first()).toContainText('E2E Bob');
  await expect(rows.first()).toContainText('58.40');

  // Period buttons are present and "Kaikki" is the active default.
  const periodBtns = page.locator('#lb-period-selector button');
  await expect(periodBtns).toHaveCount(3);
  await expect(periodBtns.filter({ hasText: 'Kaikki' })).toHaveClass(/active/);

  expect(consoleErrors, `console.error: ${consoleErrors.join(' | ')}`).toHaveLength(0);
});

test('Leaderboard period selector switches to current-year scope', async ({ page }) => {
  await page.goto('/#/leaderboard');
  await expect(page.locator('.lb-table, .lb-empty')).toBeVisible({ timeout: 15_000 });

  // Seeded runs are all in 2025 — clicking "Tämä vuosi" should either yield an
  // empty state (when "now" is in a later year) or the same data (when "now"
  // is still in 2025). Either way the request must succeed and the UI must
  // update its active state without throwing.
  await page.locator('#lb-period-selector button[data-period="year"]').click();
  await expect(page.locator('#lb-period-selector button[data-period="year"]'))
    .toHaveClass(/active/);
  // Either a table or the empty-state element renders — never an unhandled
  // error message.
  await expect(page.locator('.lb-table, .lb-empty')).toBeVisible();
});

test('Clicking a leaderboard track link navigates back to the map detail view', async ({ page }) => {
  await page.goto('/#/leaderboard');
  await expect(page.locator('.lb-table tbody tr').first()).toBeVisible({ timeout: 15_000 });

  // First row's track link.
  await page.locator('.lb-table .lb-track-link').first().click();

  // Hash returns to '#/' and the detail header shows the track name.
  await expect(page).toHaveURL(/\/(?:#\/)?$/);
  await expect(page.locator('#detail-title')).toHaveText(/.+/, { timeout: 10_000 });
});

test('Suomen ennätys banner shows the seeded open-class records on the Pörssi view', async ({ page }) => {
  // Phase 3: GET /api/finnish-records returns OPEN_M (45.49 Kukkoaho 1972)
  // and OPEN_N (50.14 Salin 1974). With no category filter the banner must
  // surface both rows above the table.
  await page.goto('/#/leaderboard');
  const banner = page.locator('#lb-finnish-records');
  await expect(banner).toContainText('Markku Kukkoaho', { timeout: 15_000 });
  await expect(banner).toContainText('Riitta Salin');
  await expect(banner).toContainText('45.49');

  // Selecting a men's masters band keeps the men's open record and drops the
  // women's row — there is no curated M50 record in the seed, so only one row
  // should be visible.
  await page.locator('#lb-category-select').selectOption('M50');
  await expect(banner).toContainText('Markku Kukkoaho');
  await expect(banner).not.toContainText('Riitta Salin');
});

test('Category dropdown filters leaderboard to a single WMA band', async ({ page }) => {
  // Seed: Alice = N40 (born 1984), Bob = M50 (born 1974), Carol no profile.
  // The category options are populated client-side from a fixed list.
  await page.goto('/#/leaderboard');
  await expect(page.locator('.lb-table tbody tr').first()).toBeVisible({ timeout: 15_000 });

  const sel = page.locator('#lb-category-select');
  await expect(sel).toBeVisible();
  await sel.selectOption('M50');

  // Bob is the only seeded user in M50.
  const rows = page.locator('.lb-table tbody tr');
  await expect(rows).toHaveCount(1, { timeout: 5_000 });
  await expect(rows.first()).toContainText('E2E Bob');

  // Switch to a band with no seeded users — empty state should appear.
  await sel.selectOption('N90');
  await expect(page.locator('.lb-empty')).toBeVisible();
});

test('Per-track records panel exposes a working period selector', async ({ page }) => {
  await page.goto('/');
  const firstCard = page.locator('.track-card').first();
  await expect(firstCard).toBeVisible({ timeout: 15_000 });
  await firstCard.click();

  // Detail panel + records section rendered.
  await expect(page.locator('#detail-title')).toHaveText(/.+/, { timeout: 10_000 });
  const selector = page.locator('#records-period-selector');
  await expect(selector).toBeVisible();
  await expect(selector.locator('button')).toHaveCount(3);
  await expect(selector.locator('button[data-period="all"]')).toHaveClass(/active/);

  // Switch to "Tämä kuukausi" — the request must complete without console
  // errors and the active class must move.
  await selector.locator('button[data-period="month"]').click();
  await expect(selector.locator('button[data-period="month"]')).toHaveClass(/active/);
  // Either a records table or the empty-period notice should render.
  await expect(
    page.locator('#records-body .records-table, #records-body .records-empty-period, #records-body .no-records')
  ).toBeVisible({ timeout: 5_000 });
});
