// Screenshots of the UI in mock mode at the default window size (AGENTS.md §6).
// Usage: node scripts/screenshots.mjs <out-dir> [scenario ...]; the dev server must run.
import { chromium } from 'playwright-core';
import { existsSync, mkdirSync, readdirSync } from 'node:fs';
import { homedir } from 'node:os';
import { join } from 'node:path';

const base = process.env.MACAFON_UI_URL ?? 'http://127.0.0.1:1420/';
const out = process.argv[2] ?? '.artifacts/screenshots';
const only = new Set(process.argv.slice(3));
mkdirSync(out, { recursive: true });

/** Chromium from the Playwright cache unless CHROMIUM is set. */
function executable() {
  if (process.env.CHROMIUM) return process.env.CHROMIUM;
  const cache = join(homedir(), '.cache/ms-playwright');
  const dir = existsSync(cache) ? readdirSync(cache).find((name) => name.startsWith('chromium-')) : undefined;
  if (!dir) throw new Error('no Chromium found; set CHROMIUM');
  return join(cache, dir, 'chrome-linux64/chrome');
}

const scenarios = {
  microphone: async (page) => {
    await page.goto(base);
    await page.waitForSelector('.chain');
    await page.waitForTimeout(1500);
  },
  advanced: async (page) => {
    await page.goto(base);
    await page.waitForSelector('.chain');
    await page.locator('section[aria-label="Compressor"] .advanced-toggle').click();
    await page.locator('section[aria-label="Compressor"] [role="slider"]').nth(1).focus();
    await page.keyboard.press('PageDown');
    await page.waitForTimeout(1200);
  },
  eq: async (page) => {
    await page.goto(base);
    await page.waitForSelector('.chain');
    await page.locator('section[aria-label="EQ"] .expand').click();
    await page.locator('.point').nth(3).focus();
    for (let i = 0; i < 6; i++) await page.keyboard.press('ArrowUp');
    await page.waitForTimeout(900);
  },
  calibration: async (page) => {
    await page.goto(base);
    await page.waitForSelector('.chain');
    await page.getByRole('button', { name: 'Calibrate' }).first().click();
    await page.getByRole('button', { name: 'Start' }).click();
    await page.waitForTimeout(6200);
  },
  'calibration-result': async (page) => {
    await page.goto(base);
    await page.waitForSelector('.chain');
    await page.getByRole('button', { name: 'Calibrate' }).first().click();
    await page.getByRole('button', { name: 'Start' }).click();
    await page.waitForTimeout(10_800);
  },
  ab: async (page) => {
    await page.goto(base);
    await page.waitForSelector('.chain');
    await page.locator('.test .rec').click();
    await page.waitForTimeout(2300 + 5200);
    await page.locator('.test .rec').click();
    await page.getByRole('button', { name: 'Loop' }).click();
    await page.waitForTimeout(1600);
  },
  disconnected: async (page) => {
    await page.goto(`${base}?mic=disconnected`);
    await page.waitForSelector('.chain');
    await page.waitForTimeout(800);
  },
  bypass: async (page) => {
    await page.goto(base);
    await page.waitForSelector('.chain');
    await page.getByRole('button', { name: 'Bypass', exact: true }).click();
    await page.waitForTimeout(800);
  },
  'quit-confirm': async (page) => {
    await page.goto(`${base}?screen=settings`);
    await page.waitForSelector('section');
    await page.getByRole('button', { name: 'Quit Macafon' }).click();
    await page.waitForTimeout(500);
  },
  muted: async (page) => {
    await page.goto(base);
    await page.waitForSelector('.chain');
    await page.getByRole('button', { name: 'Mute' }).click();
    await page.waitForTimeout(1200);
  },
  settings: async (page) => {
    await page.goto(`${base}?screen=settings`);
    await page.waitForSelector('section');
    await page.waitForTimeout(600);
  },
  unavailable: async (page) => {
    await page.goto(`${base}?daemon=down`);
    await page.waitForSelector('.unavailable');
    await page.waitForTimeout(300);
  },
};

const browser = await chromium.launch({ executablePath: executable() });
const context = await browser.newContext({ viewport: { width: 1180, height: 760 }, deviceScaleFactor: 1 });
const date = new Date().toISOString().slice(0, 10);
for (const [name, run] of Object.entries(scenarios)) {
  if (only.size && !only.has(name)) continue;
  const page = await context.newPage();
  const errors = [];
  page.on('pageerror', (error) => errors.push(error.message));
  page.on('console', (message) => message.type() === 'error' && errors.push(message.text()));
  await run(page);
  const file = join(out, `${date}-ui-${name}.png`);
  await page.screenshot({ path: file });
  console.log(file, errors.length ? `errors: ${errors.join(' | ')}` : '');
  await page.close();
}
await browser.close();
