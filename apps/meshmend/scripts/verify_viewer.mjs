import { spawn } from "node:child_process";
import { mkdirSync } from "node:fs";
import { resolve } from "node:path";
import { PNG } from "pngjs";
import { chromium } from "playwright";

const root = resolve(import.meta.dirname, "..");
const stlPath = resolve(root, process.argv[2] ?? "../../rose/raw.stl");
const outDir = resolve(root, "verification");
const url = "http://127.0.0.1:1420/";
const viewports = [
  { name: "desktop", width: 1280, height: 860 },
  { name: "compact", width: 900, height: 700 },
];

mkdirSync(outDir, { recursive: true });

const server = spawn("npm", ["run", "dev", "--", "--host", "127.0.0.1"], {
  cwd: root,
  stdio: ["ignore", "pipe", "pipe"],
});

try {
  await waitForServer(url);
  const browser = await chromium.launch({
    headless: true,
    args: ["--use-gl=swiftshader"],
  });
  const results = [];

  for (const viewport of viewports) {
    const page = await browser.newPage({ viewport });
    await page.goto(url, { waitUntil: "networkidle" });
    await page.setInputFiles("#stl-file", stlPath);
    await page.waitForFunction(
      () => document.querySelector("#stat-triangles")?.textContent !== "-",
      undefined,
      { timeout: 120000 },
    );
    await page.waitForTimeout(1000);

    const canvasBox = await page.locator("canvas").boundingBox();
    if (!canvasBox) {
      throw new Error("Canvas was not rendered");
    }

    const loadedScreenshot = await page.screenshot({
      path: `${outDir}/${viewport.name}-loaded.png`,
      fullPage: true,
    });

    await page.mouse.move(
      canvasBox.x + canvasBox.width * 0.5,
      canvasBox.y + canvasBox.height * 0.5,
    );
    await page.mouse.down();
    await page.mouse.move(
      canvasBox.x + canvasBox.width * 0.72,
      canvasBox.y + canvasBox.height * 0.42,
      { steps: 12 },
    );
    await page.mouse.up();
    await page.waitForTimeout(500);

    const orbitedScreenshot = await page.screenshot({
      path: `${outDir}/${viewport.name}-orbited.png`,
      fullPage: true,
    });

    const pixelStats = compareCanvasPixels(
      loadedScreenshot,
      orbitedScreenshot,
      canvasBox,
    );

    if (pixelStats.loadedStdDev < 5) {
      throw new Error(`${viewport.name} canvas appears blank`);
    }

    if (pixelStats.diffMean < 1) {
      throw new Error(`${viewport.name} canvas did not change after orbit`);
    }

    results.push({
      viewport,
      file: await page.textContent("#stat-file"),
      triangles: await page.textContent("#stat-triangles"),
      vertices: await page.textContent("#stat-vertices"),
      bounds: await page.textContent("#stat-bounds"),
      canvas: canvasBox,
      pixelStats,
    });

    await page.close();
  }

  await browser.close();
  console.log(JSON.stringify(results, null, 2));
} finally {
  server.kill();
}

async function waitForServer(targetUrl) {
  const deadline = Date.now() + 30000;

  while (Date.now() < deadline) {
    try {
      const response = await fetch(targetUrl);
      if (response.ok) {
        return;
      }
    } catch {
      await delay(250);
    }
  }

  throw new Error(`Timed out waiting for ${targetUrl}`);
}

function compareCanvasPixels(loadedBuffer, orbitedBuffer, canvasBox) {
  const loaded = PNG.sync.read(loadedBuffer);
  const orbited = PNG.sync.read(orbitedBuffer);
  const crop = {
    x: Math.max(0, Math.floor(canvasBox.x)),
    y: Math.max(0, Math.floor(canvasBox.y)),
    width: Math.floor(canvasBox.width),
    height: Math.floor(canvasBox.height),
  };

  let count = 0;
  let sum = 0;
  let sumSquares = 0;
  let diffSum = 0;

  for (let y = crop.y; y < crop.y + crop.height; y += 1) {
    for (let x = crop.x; x < crop.x + crop.width; x += 1) {
      const index = (y * loaded.width + x) * 4;
      const luminance =
        0.2126 * loaded.data[index] +
        0.7152 * loaded.data[index + 1] +
        0.0722 * loaded.data[index + 2];
      const changed =
        Math.abs(loaded.data[index] - orbited.data[index]) +
        Math.abs(loaded.data[index + 1] - orbited.data[index + 1]) +
        Math.abs(loaded.data[index + 2] - orbited.data[index + 2]);

      count += 1;
      sum += luminance;
      sumSquares += luminance * luminance;
      diffSum += changed / 3;
    }
  }

  const mean = sum / count;
  const variance = sumSquares / count - mean * mean;

  return {
    loadedMean: Number(mean.toFixed(2)),
    loadedStdDev: Number(Math.sqrt(Math.max(variance, 0)).toFixed(2)),
    diffMean: Number((diffSum / count).toFixed(2)),
  };
}

function delay(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}
