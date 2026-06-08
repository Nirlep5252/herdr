import { chromium } from "playwright";
import { spawn } from "node:child_process";
import fs from "node:fs/promises";
import path from "node:path";
import process from "node:process";

const root = path.resolve(new URL("..", import.meta.url).pathname);
const artifacts = path.join(root, "artifacts");
const webmOutput = path.join(artifacts, "herdr-desktop-demo.webm");
const mp4Output = path.join(artifacts, "herdr-desktop-demo.mp4");
const publicArtifactOutput = "/opt/cursor/artifacts/herdr-desktop-demo.mp4";

await fs.mkdir(artifacts, { recursive: true });
await fs.mkdir(path.dirname(publicArtifactOutput), { recursive: true });
await fs.rm(webmOutput, { force: true });
await fs.rm(mp4Output, { force: true });
await fs.rm(publicArtifactOutput, { force: true });

const server = spawn(
  process.platform === "win32" ? "npx.cmd" : "npx",
  ["vite", "--host", "127.0.0.1", "--port", "4173"],
  {
    cwd: root,
    env: { ...process.env, HERDR_DESKTOP_DEMO: "1" },
    stdio: ["ignore", "pipe", "pipe"],
  },
);

try {
  await waitForServer("http://127.0.0.1:4173");
  const browser = await chromium.launch();
  const context = await browser.newContext({
    viewport: { width: 1320, height: 860 },
    recordVideo: {
      dir: artifacts,
      size: { width: 1320, height: 860 },
    },
  });
  const page = await context.newPage();
  await page.goto("http://127.0.0.1:4173", { waitUntil: "networkidle" });
  await page.waitForTimeout(1200);
  await page.getByText("codex").first().click();
  await page.waitForTimeout(900);
  await page.keyboard.press(process.platform === "darwin" ? "Meta+K" : "Control+K");
  await page.waitForTimeout(400);
  await page.keyboard.type("pi");
  await page.waitForTimeout(700);
  await page.keyboard.press("Escape");
  await page.waitForTimeout(900);
  await page.keyboard.press(process.platform === "darwin" ? "Meta+Shift+R" : "Control+Shift+R");
  await page.waitForTimeout(900);
  const video = page.video();
  await context.close();
  await browser.close();

  const videoPath = await video?.path();
  if (!videoPath) {
    throw new Error("Playwright did not produce a video");
  }
  await fs.rename(videoPath, webmOutput);
  await convertToMp4(webmOutput, mp4Output);
  await fs.copyFile(mp4Output, publicArtifactOutput);
  console.log(publicArtifactOutput);
} finally {
  server.kill();
}

async function convertToMp4(input, output) {
  await new Promise((resolve, reject) => {
    const child = spawn("ffmpeg", [
      "-y",
      "-i",
      input,
      "-c:v",
      "libx264",
      "-pix_fmt",
      "yuv420p",
      "-movflags",
      "+faststart",
      output,
    ]);
    let stderr = "";
    child.stderr.on("data", (chunk) => {
      stderr += chunk.toString("utf8");
    });
    child.on("error", reject);
    child.on("close", (code) => {
      if (code === 0) {
        resolve();
      } else {
        reject(new Error(stderr.trim() || `ffmpeg exited with ${code}`));
      }
    });
  });
}

async function waitForServer(url) {
  const started = Date.now();
  while (Date.now() - started < 15000) {
    try {
      const response = await fetch(url);
      if (response.ok) {
        return;
      }
    } catch {
      // keep polling
    }
    await new Promise((resolve) => setTimeout(resolve, 250));
  }

  let stderr = "";
  server.stderr.on("data", (chunk) => {
    stderr += chunk.toString("utf8");
  });
  throw new Error(`Vite demo server did not start. ${stderr}`);
}
