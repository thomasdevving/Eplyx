// Browser footage recorder: the real page, recorded through the Chrome
// DevTools screencast. Every compositor frame is stored as a JPEG with its
// timestamp, and every pointer/keyboard action is logged so the film can draw
// the cursor over the footage. Nothing on the page is altered: no overlay is
// injected; the recorded pixels are the product as served.
import { chromium } from '@playwright/test';
import { mkdir, writeFile, rm } from 'node:fs/promises';
import { join } from 'node:path';

export const CHROME = process.env.EPLYX_CHROME || '/opt/pw-browsers/chromium-1194/chrome-linux/chrome';

export async function launch() {
  return chromium.launch({
    executablePath: CHROME,
    args: ['--enable-unsafe-swiftshader', '--use-angle=swiftshader', '--disable-gpu-vsync', '--hide-scrollbars'],
  });
}

export class Clip {
  constructor(browser, dir, name, { width = 1440, height = 900, scale = 1.5, storageState, quality = 90, extraContext = {} } = {}) {
    Object.assign(this, { browser, dir, name, width, height, scale, storageState, quality, extraContext });
    this.frames = [];
    this.events = [];
    this.pointer = { x: width * 0.72, y: height * 0.78 };
  }
  async open() {
    this.out = join(this.dir, this.name);
    await rm(this.out, { recursive: true, force: true });
    await mkdir(this.out, { recursive: true });
    this.context = await this.browser.newContext({
      viewport: { width: this.width, height: this.height },
      deviceScaleFactor: this.scale,
      ...(this.storageState ? { storageState: this.storageState } : {}),
      ...this.extraContext,
    });
    // Only loopback services are part of the demo; anything else is refused.
    await this.context.route('**/*', r => {
      const host = new URL(r.request().url()).hostname;
      return ['127.0.0.1', 'localhost'].includes(host) ? r.continue() : r.abort();
    });
    this.page = await this.context.newPage();
    this.page.setDefaultTimeout(60000);
    this.page.on('pageerror', e => console.log(`[${this.name}] pageerror ${e.message}`));
    this.cdp = await this.context.newCDPSession(this.page);
    this.started = null;
    let n = 0;
    this.cdp.on('Page.screencastFrame', async f => {
      // metadata.timestamp is the frame swap time in epoch seconds, the same
      // clock as the action log, so frames and pointer events share one axis.
      const ts = f.metadata.timestamp;
      const file = `${String(n++).padStart(5, '0')}.jpg`;
      this.frames.push({ t: +(ts - this.wallStart / 1000).toFixed(4), file });
      this.cdp.send('Page.screencastFrameAck', { sessionId: f.sessionId }).catch(() => {});
      await writeFile(join(this.out, file), Buffer.from(f.data, 'base64'));
    });
    return this.page;
  }
  async start() {
    this.wallStart = Date.now();
    await this.cdp.send('Page.startScreencast', {
      format: 'jpeg', quality: this.quality,
      maxWidth: Math.round(this.width * this.scale), maxHeight: Math.round(this.height * this.scale),
      everyNthFrame: 1,
    });
    await this.page.waitForTimeout(300);
  }
  now() { return (Date.now() - this.wallStart) / 1000; }
  mark(label, extra = {}) {
    const t = +this.now().toFixed(3);
    this.events.push({ t, type: 'mark', label, ...extra });
    console.log(`[${this.name}] ${t.toFixed(2)} ${label}`);
  }
  async wait(ms) { await this.page.waitForTimeout(ms); }
  // Eased pointer travel, logged per step for the film's cursor layer.
  async moveTo(x, y, ms = 650) {
    const from = { ...this.pointer }, steps = Math.max(8, Math.round(ms / 16));
    for (let i = 1; i <= steps; i++) {
      const p = i / steps, e = p < .5 ? 4 * p * p * p : 1 - Math.pow(-2 * p + 2, 3) / 2;
      const nx = from.x + (x - from.x) * e, ny = from.y + (y - from.y) * e;
      await this.page.mouse.move(nx, ny);
      this.events.push({ t: +this.now().toFixed(3), type: 'move', x: +nx.toFixed(1), y: +ny.toFixed(1) });
      await this.page.waitForTimeout(ms / steps);
    }
    this.pointer = { x, y };
  }
  async pointAt(locator, ms) {
    await locator.scrollIntoViewIfNeeded();
    const b = await locator.boundingBox();
    if (!b) throw new Error(`[${this.name}] no box for target`);
    await this.moveTo(b.x + b.width / 2, b.y + b.height / 2, ms);
    return b;
  }
  async click(locator, label, ms) {
    await this.pointAt(locator, ms);
    this.events.push({ t: +this.now().toFixed(3), type: 'down', ...this.pointer });
    if (label) this.mark(label);
    await this.page.mouse.down(); await this.page.waitForTimeout(70); await this.page.mouse.up();
    this.events.push({ t: +this.now().toFixed(3), type: 'up', ...this.pointer });
    await this.page.waitForTimeout(250);
  }
  async type(locator, text, label, delay = 38) {
    await this.click(locator);
    if (label) this.mark(label);
    await locator.pressSequentially(text, { delay });
  }
  async scrollTo(locatorOrY, label, offset = 110) {
    if (label) this.mark(label);
    if (typeof locatorOrY === 'number') {
      await this.page.evaluate(y => window.scrollTo({ top: y, behavior: 'smooth' }), locatorOrY);
    } else {
      await locatorOrY.first().evaluate((el, off) => window.scrollTo({ top: el.getBoundingClientRect().top + scrollY - off, behavior: 'smooth' }), offset);
    }
    await this.page.waitForTimeout(1100);
  }
  async wheel(dy, label, ms = 900) {
    if (label) this.mark(label);
    const steps = Math.round(ms / 30);
    for (let i = 0; i < steps; i++) { await this.page.mouse.wheel(0, dy / steps); await this.page.waitForTimeout(30); }
  }
  async close(meta = {}) {
    await this.page.waitForTimeout(400);
    await this.cdp.send('Page.stopScreencast').catch(() => {});
    await this.page.waitForTimeout(300);
    const log = {
      name: this.name, viewport: [this.width, this.height], scale: this.scale,
      frame_size: [Math.round(this.width * this.scale), Math.round(this.height * this.scale)],
      duration: this.frames.at(-1)?.t ?? 0,
      frames: this.frames, events: this.events, url: this.page.url(), ...meta,
    };
    await writeFile(join(this.out, 'clip.json'), JSON.stringify(log));
    await this.context.close();
    console.log(`[${this.name}] saved ${this.frames.length} frames, ${log.duration.toFixed(1)}s`);
    return log;
  }
}
