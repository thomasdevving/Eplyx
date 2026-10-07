import { test, expect } from '@playwright/test';

test('fragment identifiers are decoded as IDs, not CSS selectors', async ({ page }) => {
  const errors = [];
  page.on('pageerror', error => errors.push(error.message));
  await page.goto('/cli#[');
  await expect(page.getByRole('heading', { level: 1 })).toBeVisible();
  await page.getByRole('navigation', { name: 'Primary navigation' }).getByRole('link', { name: 'Start here' }).click();
  await expect(page).toHaveURL(/\/start$/);
  expect(errors).toEqual([]);
});

test('client navigation preserves the query string', async ({ page }) => {
  await page.goto('/');
  const link = page.getByRole('navigation', { name: 'Primary navigation' }).getByRole('link', { name: 'Start here' });
  await link.evaluate(element => { element.href = '/start?source=review#migration'; });
  await link.click();
  await expect(page).toHaveURL(/\/start\?source=review#migration$/);
  await expect(page.locator('#start-migration')).toBeVisible();
});
