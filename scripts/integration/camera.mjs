#!/usr/bin/env node
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { spawn, spawnSync } from "node:child_process";
import { selectIntegrationSimulator } from "./simulator-selection.mjs";

const root = path.resolve(new URL("../..", import.meta.url).pathname);
const simdeck = path.join(root, "build", "simdeck");
const tempRoot = fs.mkdtempSync(path.join(os.tmpdir(), "simdeck-camera-it-"));
const executable = "SimDeckCameraFixture";
const bundleId = "dev.nativescript.simdeck.integration.camera";
const minimumIosVersion = "15.0";
const verbose = process.env.SIMDECK_INTEGRATION_VERBOSE === "1";
const showSimulator = process.env.SIMDECK_INTEGRATION_SHOW_SIMULATOR === "1";
const keepSimulator = process.env.SIMDECK_INTEGRATION_KEEP_SIMULATOR === "1";
const serverUrl = new URL(
  process.env.SIMDECK_SERVER_URL ?? "http://127.0.0.1:4310",
);
const chromePath =
  process.env.CHROME_PATH ??
  "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome";
const chromeDebugPort = Number(
  process.env.SIMDECK_CAMERA_BENCHMARK_CHROME_PORT ?? "9341",
);
const benchmarkOutputPath = process.env.SIMDECK_CAMERA_BENCHMARK_OUTPUT ?? "";
const commandTimeoutMs = Number(
  process.env.SIMDECK_INTEGRATION_SIMCTL_TIMEOUT_MS ?? "300000",
);

let simulatorUDID = "";
let chromeProcess = null;

process.on("SIGINT", () => {
  cleanup();
  process.exit(130);
});
process.on("SIGTERM", () => {
  cleanup();
  process.exit(143);
});
process.on("exit", cleanup);

main()
  .then(() => {
    cleanup();
    console.log("SimDeck camera integration suite passed");
    process.exit(0);
  })
  .catch((error) => {
    console.error(error?.stack ?? error);
    cleanup();
    process.exit(1);
  });

async function main() {
  if (process.platform !== "darwin") {
    throw new Error("SimDeck camera integration tests require macOS.");
  }
  if (!fs.existsSync(simdeck)) {
    throw new Error(`Missing ${simdeck}. Run npm run build:cli first.`);
  }

  step("select simulator runtime");
  const { runtime, deviceType, sdkVersion } = selectIntegrationSimulator({
    runJson,
    runText,
    timeoutMs: commandTimeoutMs,
  });
  const simulatorName = `SimDeck Camera Integration ${Date.now()}`;
  simulatorUDID = runText(
    "xcrun",
    [
      "simctl",
      "create",
      simulatorName,
      deviceType.identifier,
      runtime.identifier,
    ],
    { timeoutMs: commandTimeoutMs },
  ).trim();
  console.log(
    `created ${simulatorUDID} (${deviceType.name}, ${runtime.version}; iphonesimulator SDK ${sdkVersion})`,
  );

  step("boot simulator");
  runText("xcrun", ["simctl", "boot", simulatorUDID], {
    allowFailure: true,
    timeoutMs: commandTimeoutMs,
  });
  runText("xcrun", ["simctl", "bootstatus", simulatorUDID, "-b"], {
    timeoutMs: 600_000,
  });
  simdeckJson(["boot", simulatorUDID], { timeoutMs: commandTimeoutMs });
  if (showSimulator) {
    runText(
      "open",
      ["-a", "Simulator", "--args", "-CurrentDeviceUDID", simulatorUDID],
      {
        allowFailure: true,
        timeoutMs: 30_000,
      },
    );
  }

  step("build camera fixture app");
  const appPath = buildCameraFixtureApp();
  const imagePath = path.join(tempRoot, "solid-red.bmp");
  const videoPath = path.join(tempRoot, "solid-green.mov");
  writeSolidBmp(imagePath, 32, 24, { r: 255, g: 0, b: 0 });
  writeSolidMov(videoPath, 64, 48, { r: 0, g: 255, b: 0 });

  step("install camera fixture app");
  await retryRunText("xcrun", ["simctl", "install", simulatorUDID, appPath], {
    attempts: 3,
    delayMs: 2_000,
    timeoutMs: commandTimeoutMs,
  });

  step("verify initial camera status");
  const initialStatus = simdeckJson(["camera", "status", simulatorUDID]);
  if (initialStatus.alive !== false) {
    throw new Error(
      `expected daemon camera feed to be stopped: ${JSON.stringify(initialStatus)}`,
    );
  }

  step("launch camera fixture app");
  simdeckJson(["launch", simulatorUDID, bundleId]);
  await waitForMarker(
    "fixture launch",
    (marker) => marker.status === "view-loaded",
  );
  await waitForForegroundBundle(bundleId);

  step("start injected app with image source");
  const startStatus = simdeckJson([
    "camera",
    "start",
    simulatorUDID,
    "--file",
    imagePath,
    "--mirror",
    "off",
  ]);
  assertCameraStatus(startStatus, "image");

  const imageMarker = await waitForMarker(
    "solid red image frames",
    (marker) => {
      return (
        marker.frames > 0 &&
        marker.width === 1280 &&
        marker.height === 720 &&
        marker.avgRed > 180 &&
        marker.avgGreen < 90 &&
        marker.avgBlue < 90
      );
    },
  );
  console.log(
    `received image frames: frames=${imageMarker.frames} rgb=${Math.round(imageMarker.avgRed)},${Math.round(imageMarker.avgGreen)},${Math.round(imageMarker.avgBlue)}`,
  );

  step("switch to static video source");
  const videoStatus = simdeckJson([
    "camera",
    "switch",
    simulatorUDID,
    "--file",
    videoPath,
    "--mirror",
    "off",
  ]);
  assertCameraStatus(videoStatus, "video");

  const videoMarker = await waitForMarker(
    "solid green video frames",
    (marker) => {
      return (
        marker.frames > imageMarker.frames &&
        marker.avgRed < 90 &&
        marker.avgGreen > 180 &&
        marker.avgBlue < 90
      );
    },
  );
  console.log(
    `received video frames: frames=${videoMarker.frames} rgb=${Math.round(videoMarker.avgRed)},${Math.round(videoMarker.avgGreen)},${Math.round(videoMarker.avgBlue)}`,
  );

  step("switch to placeholder source");
  const switchStatus = simdeckJson([
    "camera",
    "switch",
    simulatorUDID,
    "--placeholder",
    "--mirror",
    "off",
  ]);
  assertCameraStatus(switchStatus, "placeholder");

  const placeholderMarker = await waitForMarker(
    "placeholder frames",
    (marker) => {
      return (
        marker.frames > videoMarker.frames &&
        marker.avgRed > 120 &&
        marker.avgGreen > 60 &&
        marker.avgBlue > 60
      );
    },
  );
  console.log(
    `received placeholder frames: frames=${placeholderMarker.frames} rgb=${Math.round(placeholderMarker.avgRed)},${Math.round(placeholderMarker.avgGreen)},${Math.round(placeholderMarker.avgBlue)}`,
  );

  step("stop daemon camera feed");
  const stopStatus = simdeckJson(["camera", "stop", simulatorUDID]);
  if (stopStatus.alive !== false) {
    throw new Error(
      `camera stop did not report alive=false: ${JSON.stringify(stopStatus)}`,
    );
  }

  step("run deterministic browser camera baseline");
  const benchmark = await runBrowserBenchmark();
  console.log(`camera benchmark: ${JSON.stringify(benchmark)}`);
  if (benchmarkOutputPath) {
    fs.mkdirSync(path.dirname(path.resolve(benchmarkOutputPath)), {
      recursive: true,
    });
    fs.writeFileSync(
      path.resolve(benchmarkOutputPath),
      `${JSON.stringify(benchmark, null, 2)}\n`,
    );
  }
}

async function runBrowserBenchmark() {
  if (!fs.existsSync(chromePath)) {
    throw new Error(`Missing Chrome at ${chromePath}.`);
  }
  const chromeProfile = path.join(tempRoot, "chrome-profile");
  chromeProcess = spawn(
    chromePath,
    [
      "--headless=new",
      "--autoplay-policy=no-user-gesture-required",
      `--remote-debugging-port=${chromeDebugPort}`,
      `--user-data-dir=${chromeProfile}`,
      "about:blank",
    ],
    { stdio: ["ignore", "pipe", "pipe"] },
  );
  await waitForChrome();
  const viewerUrl = new URL(serverUrl);
  viewerUrl.searchParams.set("device", simulatorUDID);
  viewerUrl.searchParams.set("cameraBenchmark", "1");
  viewerUrl.searchParams.set("cameraBenchmarkWidth", "1280");
  viewerUrl.searchParams.set("cameraBenchmarkHeight", "720");
  viewerUrl.searchParams.set("cameraBenchmarkFps", "30");
  viewerUrl.searchParams.set("stream", "webrtc");
  const targetResponse = await fetch(
    `http://127.0.0.1:${chromeDebugPort}/json/new?${encodeURIComponent(viewerUrl)}`,
    { method: "PUT" },
  );
  if (!targetResponse.ok) {
    throw new Error(
      `Chrome target creation failed with ${targetResponse.status}: ${await targetResponse.text()}`,
    );
  }
  const target = await targetResponse.json();
  const cdp = await connectCdp(target.webSocketDebuggerUrl);
  try {
    await cdp.send("Runtime.enable");
    await waitForBrowserValue(
      cdp,
      "window.__simdeckCameraBenchmark?.snapshot() ?? null",
      (value) => value?.ready && value.udid === simulatorUDID,
      60_000,
    );
    await waitForBrowserValue(
      cdp,
      `(() => {
        const video = document.querySelector("video.stream-video");
        return video ? {
          readyState: video.readyState,
          width: video.videoWidth,
          height: video.videoHeight,
        } : null;
      })()`,
      (value) => value?.readyState >= 2 && value.width > 0 && value.height > 0,
      90_000,
    );
    const started = await cdp.evaluate(
      "window.__simdeckCameraBenchmark.start()",
      true,
    );
    const firstMarker = await waitForMarker(
      "deterministic browser camera frames",
      (marker) => marker.status === "frame" && marker.frames >= 10,
    );
    const sampleStartedAt = Date.now();
    const firstBrowserSample = await waitForBrowserValue(
      cdp,
      "window.__simdeckCameraBenchmark.snapshot()",
      (value) => value?.transport?.encodedFramesPerSecond > 0,
      30_000,
    );
    const initialCameraStatus = simdeckJson([
      "camera",
      "status",
      simulatorUDID,
    ]);
    const initialMarker = readMarker();
    const initialViewer = await browserViewerSample(cdp);
    const state = await fetchSimulatorState();
    const benchmarkDurationMs = 10_000;
    const metricsStartedAt = Date.now();
    const processStatsPromise = collectProcessStats(
      [
        { name: "server", pid: started.processId },
        {
          name: "simulatorApp",
          pid: state.foregroundApp?.processIdentifier,
        },
      ],
      benchmarkDurationMs,
    );
    const glassToGlass = await collectGlassToGlassSamples(
      cdp,
      benchmarkDurationMs,
    );
    const processStats = await processStatsPromise;
    const finalBrowserSample = await cdp.evaluate(
      "window.__simdeckCameraBenchmark.snapshot()",
    );
    const finalMarker = readMarker();
    const cameraStatus = simdeckJson(["camera", "status", simulatorUDID]);
    const viewer = await browserViewerSample(cdp);
    const metricsDurationMs = Date.now() - metricsStartedAt;
    const stopped = await cdp.evaluate(
      "window.__simdeckCameraBenchmark.stop()",
      true,
    );
    return {
      transport: "webrtc-h264",
      source: { width: 1280, height: 720, framesPerSecond: 30 },
      started,
      stopped,
      sampleDurationMs: Date.now() - sampleStartedAt,
      firstBrowserSample,
      finalBrowserSample,
      glassToGlass,
      rates: {
        observedDurationMs: metricsDurationMs,
        appFramesPerSecond: frameRate(
          initialMarker?.frames,
          finalMarker?.frames,
          metricsDurationMs,
        ),
        decodedFramesPerSecond: frameRate(
          initialCameraStatus.frames,
          cameraStatus.frames,
          metricsDurationMs,
        ),
        presentedFramesPerSecond: frameRate(
          initialViewer?.presentedFrames,
          viewer?.presentedFrames,
          metricsDurationMs,
        ),
      },
      processStats,
      firstMarker,
      finalMarker,
      cameraStatus,
      viewer,
    };
  } finally {
    cdp.close();
  }
}

async function browserViewerSample(cdp) {
  return cdp.evaluate(`(() => {
    const video = document.querySelector("video.stream-video");
    if (!video) return null;
    const quality = video.getVideoPlaybackQuality?.();
    return {
      readyState: video.readyState,
      width: video.videoWidth,
      height: video.videoHeight,
      presentedFrames: quality?.totalVideoFrames ?? 0,
      droppedFrames: quality?.droppedVideoFrames ?? 0,
    };
  })()`);
}

async function fetchSimulatorState() {
  const url = new URL(
    `/api/simulators/${encodeURIComponent(simulatorUDID)}/state`,
    serverUrl,
  );
  const response = await fetch(url);
  if (!response.ok) {
    throw new Error(
      `Simulator state failed with ${response.status}: ${await response.text()}`,
    );
  }
  return response.json();
}

async function collectProcessStats(processes, durationMs) {
  const samples = new Map(
    processes
      .filter((process) => Number.isInteger(process.pid) && process.pid > 0)
      .map((process) => [process.name, []]),
  );
  const deadline = Date.now() + durationMs;
  while (Date.now() < deadline) {
    for (const process of processes) {
      if (!samples.has(process.name)) {
        continue;
      }
      const output = runText(
        "ps",
        ["-o", "%cpu=,rss=", "-p", String(process.pid)],
        { allowFailure: true, timeoutMs: 5_000 },
      ).trim();
      const [cpu, rssKilobytes] = output.split(/\s+/).map(Number);
      if (Number.isFinite(cpu) && Number.isFinite(rssKilobytes)) {
        samples.get(process.name).push({ cpu, rssKilobytes });
      }
    }
    await sleep(500);
  }
  return Object.fromEntries(
    [...samples.entries()].map(([name, values]) => {
      const cpu = values.map((value) => value.cpu).sort((a, b) => a - b);
      const rss = values
        .map((value) => value.rssKilobytes)
        .sort((a, b) => a - b);
      return [
        name,
        {
          samples: values.length,
          averageCpuPercent:
            cpu.reduce((total, value) => total + value, 0) /
            Math.max(1, cpu.length),
          p95CpuPercent: percentile(cpu, 0.95),
          minimumRssBytes: (rss[0] ?? 0) * 1024,
          maximumRssBytes: (rss.at(-1) ?? 0) * 1024,
        },
      ];
    }),
  );
}

function frameRate(initialValue, finalValue, durationMs) {
  if (!Number.isFinite(initialValue) || !Number.isFinite(finalValue)) {
    return null;
  }
  return ((finalValue - initialValue) * 1_000) / durationMs;
}

async function waitForChrome() {
  const deadline = Date.now() + 30_000;
  while (Date.now() < deadline) {
    if (chromeProcess?.exitCode !== null) {
      throw new Error(`Chrome exited before DevTools was ready.`);
    }
    try {
      const response = await fetch(
        `http://127.0.0.1:${chromeDebugPort}/json/version`,
      );
      if (response.ok) {
        return;
      }
    } catch {}
    await sleep(100);
  }
  throw new Error("Timed out waiting for Chrome DevTools.");
}

async function connectCdp(url) {
  const socket = new WebSocket(url);
  await new Promise((resolve, reject) => {
    socket.addEventListener("open", resolve, { once: true });
    socket.addEventListener("error", reject, { once: true });
  });
  let nextId = 1;
  const pending = new Map();
  socket.addEventListener("message", (event) => {
    const message = JSON.parse(event.data);
    const request = pending.get(message.id);
    if (!request) {
      return;
    }
    pending.delete(message.id);
    if (message.error) {
      request.reject(new Error(message.error.message));
    } else {
      request.resolve(message.result);
    }
  });
  return {
    close: () => socket.close(),
    send(method, params = {}) {
      const id = nextId++;
      return new Promise((resolve, reject) => {
        pending.set(id, { resolve, reject });
        socket.send(JSON.stringify({ id, method, params }));
      });
    },
    async evaluate(expression, awaitPromise = false) {
      const result = await this.send("Runtime.evaluate", {
        expression,
        awaitPromise,
        returnByValue: true,
      });
      if (result.exceptionDetails) {
        throw new Error(
          result.exceptionDetails.exception?.description ??
            result.exceptionDetails.text,
        );
      }
      return result.result.value;
    },
  };
}

async function waitForBrowserValue(cdp, expression, predicate, timeoutMs) {
  const deadline = Date.now() + timeoutMs;
  let value;
  while (Date.now() < deadline) {
    value = await cdp.evaluate(expression);
    if (predicate(value)) {
      return value;
    }
    await sleep(100);
  }
  throw new Error(
    `Timed out waiting for browser value from ${expression}: ${JSON.stringify(value)}`,
  );
}

async function collectGlassToGlassSamples(cdp, durationMs) {
  const result = await cdp.evaluate(
    `(() => new Promise((resolve, reject) => {
      const video = document.querySelector("video.stream-video");
      if (!video || typeof video.requestVideoFrameCallback !== "function") {
        reject(new Error("Simulator viewer video is unavailable."));
        return;
      }
      const scale = 3;
      const canvas = document.createElement("canvas");
      canvas.width = Math.max(1, Math.round(video.videoWidth / scale));
      canvas.height = Math.max(1, Math.round(video.videoHeight / scale));
      const context = canvas.getContext("2d", { alpha: false, willReadFrequently: true });
      if (!context) {
        reject(new Error("Unable to create latency sampling canvas."));
        return;
      }
      const markerBits = 48;
      const timestampModulus = 2 ** 32;
      const samples = [];
      let decodeFailures = 0;
      let lastFrame = -1;
      let lastBounds = null;
      let callbackId = 0;
      let stopped = false;
      const numberFromBits = (bits) => bits.reduce(
        (value, bit) => value * 2 + (bit ? 1 : 0),
        0,
      );
      const sampleFrame = () => {
        context.drawImage(video, 0, 0, canvas.width, canvas.height);
        const image = context.getImageData(0, 0, canvas.width, canvas.height);
        const data = image.data;
        let minX = canvas.width;
        let minY = canvas.height;
        let maxX = -1;
        let maxY = -1;
        const startX = Math.floor(canvas.width * 0.3);
        const endX = Math.ceil(canvas.width * 0.7);
        for (let y = Math.floor(canvas.height * 0.04); y < canvas.height * 0.96; y += 1) {
          for (let x = startX; x < endX; x += 1) {
            const offset = (y * canvas.width + x) * 4;
            const red = data[offset];
            const green = data[offset + 1];
            const blue = data[offset + 2];
            if (red > 140 && blue > 140 && green < 155 && red + blue > green * 2.2) {
              minX = Math.min(minX, x);
              minY = Math.min(minY, y);
              maxX = Math.max(maxX, x);
              maxY = Math.max(maxY, y);
            }
          }
        }
        if (maxX < minX || maxY - minY < canvas.height * 0.15) {
          decodeFailures += 1;
          return;
        }
        lastBounds = { minX, minY, maxX, maxY };
        const cell = (maxY - minY + 1) / (markerBits + 2);
        const centerX = Math.round((minX + maxX) / 2);
        const bits = [];
        for (let index = 0; index < markerBits; index += 1) {
          const centerY = Math.max(
            0,
            Math.min(canvas.height - 1, Math.round(minY + cell * (index + 1.5))),
          );
          let luminance = 0;
          let count = 0;
          for (let dy = -1; dy <= 1; dy += 1) {
            for (let dx = -1; dx <= 1; dx += 1) {
              const x = Math.max(0, Math.min(canvas.width - 1, centerX + dx));
              const y = Math.max(0, Math.min(canvas.height - 1, centerY + dy));
              const offset = (y * canvas.width + x) * 4;
              luminance += data[offset] * 0.2126 + data[offset + 1] * 0.7152 + data[offset + 2] * 0.0722;
              count += 1;
            }
          }
          bits.push(luminance / count > 128);
        }
        const frame = numberFromBits(bits.slice(0, 16));
        const timestamp = numberFromBits(bits.slice(16));
        const now = Math.floor(performance.now()) % timestampModulus;
        const latencyMs = (now - timestamp + timestampModulus) % timestampModulus;
        if (frame === lastFrame || latencyMs > 5_000) {
          if (latencyMs > 5_000) decodeFailures += 1;
          return;
        }
        lastFrame = frame;
        samples.push({ frame, latencyMs, timestamp });
      };
      const onFrame = () => {
        if (stopped) return;
        sampleFrame();
        callbackId = video.requestVideoFrameCallback(onFrame);
      };
      callbackId = video.requestVideoFrameCallback(onFrame);
      setTimeout(() => {
        stopped = true;
        video.cancelVideoFrameCallback(callbackId);
        resolve({ samples, decodeFailures, markerBounds: lastBounds });
      }, ${Math.round(durationMs)});
    }))()`,
    true,
  );
  const latencies = result.samples
    .map((sample) => sample.latencyMs)
    .sort((a, b) => a - b);
  if (latencies.length < 30) {
    throw new Error(
      `Camera latency marker produced only ${latencies.length} samples (${result.decodeFailures} decode failures).`,
    );
  }
  return {
    samples: latencies.length,
    decodeFailures: result.decodeFailures,
    markerBounds: result.markerBounds,
    minimumMs: latencies[0],
    medianMs: percentile(latencies, 0.5),
    p95Ms: percentile(latencies, 0.95),
    maximumMs: latencies.at(-1),
  };
}

function percentile(sortedValues, percentileValue) {
  const index = Math.min(
    sortedValues.length - 1,
    Math.max(0, Math.ceil(sortedValues.length * percentileValue) - 1),
  );
  return sortedValues[index];
}

async function waitForForegroundBundle(expectedBundleId) {
  const deadline = Date.now() + 30_000;
  let foregroundApp = null;
  while (Date.now() < deadline) {
    const url = new URL(
      `/api/simulators/${encodeURIComponent(simulatorUDID)}/state`,
      serverUrl,
    );
    const response = await fetch(url);
    if (response.ok) {
      const state = await response.json();
      foregroundApp = state.foregroundApp ?? null;
      if (foregroundApp?.bundleIdentifier === expectedBundleId) {
        return;
      }
    }
    await sleep(250);
  }
  throw new Error(
    `Timed out waiting for foreground app ${expectedBundleId}: ${JSON.stringify(foregroundApp)}`,
  );
}

function buildCameraFixtureApp() {
  const targetArch = process.arch === "arm64" ? "arm64" : "x86_64";
  const appPath = path.join(tempRoot, `${executable}.app`);
  const sourcePath = path.join(tempRoot, `${executable}.m`);
  fs.mkdirSync(appPath, { recursive: true });
  fs.writeFileSync(path.join(appPath, "Info.plist"), fixtureInfoPlist());
  fs.writeFileSync(sourcePath, fixtureSource());
  runText("xcrun", [
    "--sdk",
    "iphonesimulator",
    "clang",
    "-target",
    `${targetArch}-apple-ios${minimumIosVersion}-simulator`,
    "-fobjc-arc",
    "-fmodules",
    "-framework",
    "AVFoundation",
    "-framework",
    "CoreGraphics",
    "-framework",
    "CoreMedia",
    "-framework",
    "CoreVideo",
    "-framework",
    "Foundation",
    "-framework",
    "UIKit",
    sourcePath,
    "-o",
    path.join(appPath, executable),
  ]);
  return appPath;
}

function writeSolidMov(outputPath, width, height, color) {
  const sourcePath = path.join(tempRoot, "WriteSolidMov.m");
  const binaryPath = path.join(tempRoot, "WriteSolidMov");
  fs.writeFileSync(
    sourcePath,
    `#import <AVFoundation/AVFoundation.h>
#import <CoreMedia/CoreMedia.h>
#import <CoreVideo/CoreVideo.h>
#import <Foundation/Foundation.h>

int main(int argc, const char *argv[]) {
  @autoreleasepool {
    if (argc < 2) return 64;
    NSString *path = [NSString stringWithUTF8String:argv[1]];
    [[NSFileManager defaultManager] removeItemAtPath:path error:nil];
    NSURL *url = [NSURL fileURLWithPath:path];
    NSError *error = nil;
    AVAssetWriter *writer = [AVAssetWriter assetWriterWithURL:url fileType:AVFileTypeQuickTimeMovie error:&error];
    if (!writer) {
      fprintf(stderr, "%s\\n", error.localizedDescription.UTF8String);
      return 1;
    }
    NSDictionary *settings = @{
      AVVideoCodecKey: AVVideoCodecTypeH264,
      AVVideoWidthKey: @(${width}),
      AVVideoHeightKey: @(${height}),
    };
    AVAssetWriterInput *input = [AVAssetWriterInput assetWriterInputWithMediaType:AVMediaTypeVideo outputSettings:settings];
    input.expectsMediaDataInRealTime = NO;
    NSDictionary *attributes = @{
      (id)kCVPixelBufferPixelFormatTypeKey: @(kCVPixelFormatType_32BGRA),
      (id)kCVPixelBufferWidthKey: @(${width}),
      (id)kCVPixelBufferHeightKey: @(${height}),
    };
    AVAssetWriterInputPixelBufferAdaptor *adaptor = [AVAssetWriterInputPixelBufferAdaptor assetWriterInputPixelBufferAdaptorWithAssetWriterInput:input sourcePixelBufferAttributes:attributes];
    if (![writer canAddInput:input]) return 2;
    [writer addInput:input];
    if (![writer startWriting]) return 3;
    [writer startSessionAtSourceTime:kCMTimeZero];
    for (int frame = 0; frame < 90; frame += 1) {
      while (!input.readyForMoreMediaData) {
        [NSThread sleepForTimeInterval:0.01];
      }
      CVPixelBufferRef pixelBuffer = NULL;
      CVReturn status = CVPixelBufferPoolCreatePixelBuffer(NULL, adaptor.pixelBufferPool, &pixelBuffer);
      if (status != kCVReturnSuccess || !pixelBuffer) return 4;
      CVPixelBufferLockBaseAddress(pixelBuffer, 0);
      uint8_t *base = (uint8_t *)CVPixelBufferGetBaseAddress(pixelBuffer);
      size_t bytesPerRow = CVPixelBufferGetBytesPerRow(pixelBuffer);
      for (int y = 0; y < ${height}; y += 1) {
        uint8_t *row = base + (size_t)y * bytesPerRow;
        for (int x = 0; x < ${width}; x += 1) {
          uint8_t *pixel = row + (size_t)x * 4;
          pixel[0] = ${color.b};
          pixel[1] = ${color.g};
          pixel[2] = ${color.r};
          pixel[3] = 255;
        }
      }
      CVPixelBufferUnlockBaseAddress(pixelBuffer, 0);
      CMTime presentationTime = CMTimeMake(frame, 30);
      if (![adaptor appendPixelBuffer:pixelBuffer withPresentationTime:presentationTime]) return 5;
      CVPixelBufferRelease(pixelBuffer);
    }
    [input markAsFinished];
    dispatch_semaphore_t semaphore = dispatch_semaphore_create(0);
    [writer finishWritingWithCompletionHandler:^{
      dispatch_semaphore_signal(semaphore);
    }];
    dispatch_semaphore_wait(semaphore, DISPATCH_TIME_FOREVER);
    return writer.status == AVAssetWriterStatusCompleted ? 0 : 6;
  }
}
`,
  );
  runText("clang", [
    "-fobjc-arc",
    "-fmodules",
    "-framework",
    "AVFoundation",
    "-framework",
    "CoreMedia",
    "-framework",
    "CoreVideo",
    "-framework",
    "Foundation",
    sourcePath,
    "-o",
    binaryPath,
  ]);
  runText(binaryPath, [outputPath]);
}

function fixtureInfoPlist() {
  return `<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleDevelopmentRegion</key><string>en</string>
  <key>CFBundleExecutable</key><string>${executable}</string>
  <key>CFBundleIdentifier</key><string>${bundleId}</string>
  <key>CFBundleInfoDictionaryVersion</key><string>6.0</string>
  <key>CFBundleName</key><string>${executable}</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>1.0</string>
  <key>CFBundleVersion</key><string>1</string>
  <key>CFBundleURLTypes</key>
  <array>
    <dict>
      <key>CFBundleURLName</key><string>${executable}</string>
      <key>CFBundleURLSchemes</key>
      <array><string>simdeck-camera-fixture</string></array>
    </dict>
  </array>
  <key>LSRequiresIPhoneOS</key><true/>
  <key>MinimumOSVersion</key><string>${minimumIosVersion}</string>
  <key>NSCameraUsageDescription</key><string>Camera fixture validates SimDeck camera simulation.</string>
  <key>UIDeviceFamily</key><array><integer>1</integer></array>
  <key>UIApplicationSceneManifest</key>
  <dict>
    <key>UIApplicationSupportsMultipleScenes</key><false/>
    <key>UISceneConfigurations</key>
    <dict>
      <key>UIWindowSceneSessionRoleApplication</key>
      <array>
        <dict>
          <key>UISceneConfigurationName</key><string>Default Configuration</string>
          <key>UISceneDelegateClassName</key><string>SceneDelegate</string>
        </dict>
      </array>
    </dict>
  </dict>
</dict>
</plist>
`;
}

function fixtureSource() {
  return `#import <AVFoundation/AVFoundation.h>
#import <CoreMedia/CoreMedia.h>
#import <CoreVideo/CoreVideo.h>
#import <UIKit/UIKit.h>
#import <objc/runtime.h>

@interface CameraViewController : UIViewController <AVCaptureVideoDataOutputSampleBufferDelegate>
@property (nonatomic, strong) AVCaptureSession *session;
@property (nonatomic, strong) AVCaptureVideoPreviewLayer *previewLayer;
@property (nonatomic, strong) UILabel *statusLabel;
@property (nonatomic) NSInteger frames;
@end

@implementation CameraViewController

- (void)viewDidLoad {
  [super viewDidLoad];
  self.view.backgroundColor = UIColor.systemBackgroundColor;
  self.statusLabel = [[UILabel alloc] init];
  self.statusLabel.text = @"Camera Starting";
  self.statusLabel.textAlignment = NSTextAlignmentCenter;
  self.statusLabel.numberOfLines = 0;
  self.statusLabel.font = [UIFont preferredFontForTextStyle:UIFontTextStyleHeadline];
  self.statusLabel.accessibilityIdentifier = @"camera.status";
  self.statusLabel.translatesAutoresizingMaskIntoConstraints = NO;
  [self.view addSubview:self.statusLabel];
  [NSLayoutConstraint activateConstraints:@[
    [self.statusLabel.centerXAnchor constraintEqualToAnchor:self.view.centerXAnchor],
    [self.statusLabel.centerYAnchor constraintEqualToAnchor:self.view.centerYAnchor],
    [self.statusLabel.leadingAnchor constraintGreaterThanOrEqualToAnchor:self.view.safeAreaLayoutGuide.leadingAnchor constant:24.0],
    [self.statusLabel.trailingAnchor constraintLessThanOrEqualToAnchor:self.view.safeAreaLayoutGuide.trailingAnchor constant:-24.0],
  ]];
  [self writeMarkerWithStatus:@"view-loaded" width:0 height:0 red:0 green:0 blue:0];
  dispatch_after(dispatch_time(DISPATCH_TIME_NOW, (int64_t)(2 * NSEC_PER_SEC)), dispatch_get_main_queue(), ^{
    dispatch_async(dispatch_get_global_queue(QOS_CLASS_USER_INITIATED, 0), ^{
      [self startCamera];
    });
  });
}

- (void)viewDidLayoutSubviews {
  [super viewDidLayoutSubviews];
  self.previewLayer.frame = self.view.bounds;
}

- (void)startCamera {
  [self writeMarkerWithStatus:@"camera-enter" width:0 height:0 red:0 green:0 blue:0];
  const char *shmName = getenv("SIMDECK_CAMERA_SHM_NAME");
  [self writeMarkerWithStatus:(shmName && shmName[0] != '\\0') ? @"env-present" : @"env-missing" width:0 height:0 red:0 green:0 blue:0];
  AVCaptureDevice *device = [AVCaptureDevice defaultDeviceWithMediaType:AVMediaTypeVideo];
  if (!device) {
    [self writeMarkerWithStatus:@"no-device" width:0 height:0 red:0 green:0 blue:0];
    dispatch_async(dispatch_get_main_queue(), ^{
      self.statusLabel.text = @"No Camera Device";
    });
    return;
  }
  NSString *deviceStatus = [device.localizedName isEqualToString:@"SimDeck Camera"] ? @"device-simdeck" : @"device-other";
  [self writeMarkerWithStatus:deviceStatus width:0 height:0 red:0 green:0 blue:0];
  NSError *error = nil;
  AVCaptureDeviceInput *input = [AVCaptureDeviceInput deviceInputWithDevice:device error:&error];
  if (!input) {
    [self writeMarkerWithStatus:error.localizedDescription ?: @"input-error" width:0 height:0 red:0 green:0 blue:0];
    dispatch_async(dispatch_get_main_queue(), ^{
      self.statusLabel.text = @"Camera Input Failed";
    });
    return;
  }
  [self writeMarkerWithStatus:@"input" width:0 height:0 red:0 green:0 blue:0];
  AVCaptureVideoDataOutput *output = [[AVCaptureVideoDataOutput alloc] init];
  NSString *outputStatus = [NSString stringWithFormat:@"output-%@", NSStringFromClass(object_getClass(output))];
  [self writeMarkerWithStatus:outputStatus width:0 height:0 red:0 green:0 blue:0];
  dispatch_queue_t sampleQueue = dispatch_queue_create("dev.nativescript.simdeck.camera.fixture", DISPATCH_QUEUE_SERIAL);
  [self writeMarkerWithStatus:@"queue" width:0 height:0 red:0 green:0 blue:0];
  [output setSampleBufferDelegate:self queue:sampleQueue];
  [self writeMarkerWithStatus:@"output" width:0 height:0 red:0 green:0 blue:0];
  self.session = [[AVCaptureSession alloc] init];
  self.session.sessionPreset = AVCaptureSessionPreset1280x720;
  [self writeMarkerWithStatus:@"session" width:0 height:0 red:0 green:0 blue:0];
  if (![self.session canAddInput:input] || ![self.session canAddOutput:output]) {
    [self writeMarkerWithStatus:@"cannot-add-io" width:0 height:0 red:0 green:0 blue:0];
    dispatch_async(dispatch_get_main_queue(), ^{
      self.statusLabel.text = @"Camera Session Failed";
    });
    return;
  }
  [self.session addInput:input];
  [self.session addOutput:output];
  dispatch_sync(dispatch_get_main_queue(), ^{
    self.previewLayer = [AVCaptureVideoPreviewLayer layerWithSession:self.session];
    self.previewLayer.frame = self.view.bounds;
    self.previewLayer.videoGravity = AVLayerVideoGravityResizeAspect;
    [self.view.layer insertSublayer:self.previewLayer atIndex:0];
    self.statusLabel.hidden = YES;
  });
  [self writeMarkerWithStatus:@"starting" width:0 height:0 red:0 green:0 blue:0];
  [self.session startRunning];
  [self writeMarkerWithStatus:@"started" width:0 height:0 red:0 green:0 blue:0];
}

- (void)captureOutput:(AVCaptureOutput *)output
 didOutputSampleBuffer:(CMSampleBufferRef)sampleBuffer
       fromConnection:(AVCaptureConnection *)connection {
  (void)output;
  (void)connection;
  CVPixelBufferRef pixelBuffer = CMSampleBufferGetImageBuffer(sampleBuffer);
  if (!pixelBuffer) return;
  CVPixelBufferLockBaseAddress(pixelBuffer, kCVPixelBufferLock_ReadOnly);
  uint8_t *base = (uint8_t *)CVPixelBufferGetBaseAddress(pixelBuffer);
  size_t width = CVPixelBufferGetWidth(pixelBuffer);
  size_t height = CVPixelBufferGetHeight(pixelBuffer);
  size_t bytesPerRow = CVPixelBufferGetBytesPerRow(pixelBuffer);
  double red = 0;
  double green = 0;
  double blue = 0;
  NSInteger samples = 0;
  size_t yStep = MAX((size_t)1, height / 24);
  size_t xStep = MAX((size_t)1, width / 24);
  for (size_t y = 0; y < height; y += yStep) {
    uint8_t *row = base + y * bytesPerRow;
    for (size_t x = 0; x < width; x += xStep) {
      uint8_t *pixel = row + x * 4;
      blue += pixel[0];
      green += pixel[1];
      red += pixel[2];
      samples += 1;
    }
  }
  CVPixelBufferUnlockBaseAddress(pixelBuffer, kCVPixelBufferLock_ReadOnly);
  if (samples > 0) {
    red /= samples;
    green /= samples;
    blue /= samples;
  }
  self.frames += 1;
  [self writeMarkerWithStatus:@"frame" width:width height:height red:red green:green blue:blue];
  if (self.frames % 10 == 0) {
    dispatch_async(dispatch_get_main_queue(), ^{
      self.statusLabel.text = [NSString stringWithFormat:@"Camera Frame %ld", (long)self.frames];
    });
  }
}

- (void)writeMarkerWithStatus:(NSString *)status
                        width:(size_t)width
                       height:(size_t)height
                          red:(double)red
                        green:(double)green
                         blue:(double)blue {
  NSString *directory = NSSearchPathForDirectoriesInDomains(NSDocumentDirectory, NSUserDomainMask, YES).firstObject;
  [[NSFileManager defaultManager] createDirectoryAtPath:directory withIntermediateDirectories:YES attributes:nil error:nil];
  NSString *path = [directory stringByAppendingPathComponent:@"camera-frame.json"];
  NSString *payload = [NSString stringWithFormat:
    @"{\\"status\\":\\"%@\\",\\"frames\\":%ld,\\"width\\":%zu,\\"height\\":%zu,\\"avgRed\\":%.3f,\\"avgGreen\\":%.3f,\\"avgBlue\\":%.3f}",
    status ?: @"unknown",
    (long)self.frames,
    width,
    height,
    red,
    green,
    blue];
  [payload writeToFile:path atomically:YES encoding:NSUTF8StringEncoding error:nil];
}

@end

@interface SceneDelegate : UIResponder <UIWindowSceneDelegate>
@property (nonatomic, strong) UIWindow *window;
@end

@implementation SceneDelegate

- (void)startCamera {
  CameraViewController *controller = (CameraViewController *)self.window.rootViewController;
  dispatch_async(dispatch_get_global_queue(QOS_CLASS_USER_INITIATED, 0), ^{
    [controller startCamera];
  });
}

- (void)scene:(UIScene *)scene
willConnectToSession:(UISceneSession *)session
      options:(UISceneConnectionOptions *)connectionOptions {
  (void)session;
  (void)connectionOptions;
  UIWindowScene *windowScene = (UIWindowScene *)scene;
  self.window = [[UIWindow alloc] initWithWindowScene:windowScene];
  self.window.rootViewController = [[CameraViewController alloc] init];
  [self.window makeKeyAndVisible];
  if (connectionOptions.URLContexts.count > 0) {
    [self startCamera];
  }
}

- (void)scene:(UIScene *)scene openURLContexts:(NSSet<UIOpenURLContext *> *)URLContexts {
  (void)scene;
  if (URLContexts.count > 0) {
    [self startCamera];
  }
}

@end

@interface AppDelegate : UIResponder <UIApplicationDelegate>
@end

@implementation AppDelegate

- (BOOL)application:(UIApplication *)application didFinishLaunchingWithOptions:(NSDictionary *)launchOptions {
  (void)application;
  (void)launchOptions;
  return YES;
}

@end

int main(int argc, char *argv[]) {
  @autoreleasepool {
    return UIApplicationMain(argc, argv, nil, NSStringFromClass(AppDelegate.class));
  }
}
`;
}

async function waitForMarker(label, predicate) {
  const deadline = Date.now() + 45_000;
  let lastMarker = null;
  while (Date.now() < deadline) {
    const marker = readMarker();
    if (marker) {
      lastMarker = marker;
      if (predicate(marker)) {
        return marker;
      }
    }
    await sleep(500);
  }
  throw new Error(
    `Timed out waiting for ${label}. Last marker: ${JSON.stringify(lastMarker)}`,
  );
}

function readMarker() {
  let container = "";
  try {
    container = runText(
      "xcrun",
      ["simctl", "get_app_container", simulatorUDID, bundleId, "data"],
      { timeoutMs: 30_000 },
    ).trim();
  } catch {
    return null;
  }
  const markerPath = path.join(container, "Documents", "camera-frame.json");
  if (!fs.existsSync(markerPath)) {
    return null;
  }
  try {
    return JSON.parse(fs.readFileSync(markerPath, "utf8"));
  } catch {
    return null;
  }
}

function assertCameraStatus(status, source) {
  if (status.ok !== true || status.alive !== true || status.source !== source) {
    throw new Error(
      `unexpected camera status for ${source}: ${JSON.stringify(status)}`,
    );
  }
  if (status.width !== 1280 || status.height !== 720) {
    throw new Error(`unexpected camera dimensions: ${JSON.stringify(status)}`);
  }
}

function writeSolidBmp(filePath, width, height, color) {
  const rowStride = Math.ceil((width * 3) / 4) * 4;
  const pixelOffset = 54;
  const fileSize = pixelOffset + rowStride * height;
  const buffer = Buffer.alloc(fileSize);
  buffer.write("BM", 0, "ascii");
  buffer.writeUInt32LE(fileSize, 2);
  buffer.writeUInt32LE(pixelOffset, 10);
  buffer.writeUInt32LE(40, 14);
  buffer.writeInt32LE(width, 18);
  buffer.writeInt32LE(height, 22);
  buffer.writeUInt16LE(1, 26);
  buffer.writeUInt16LE(24, 28);
  buffer.writeUInt32LE(rowStride * height, 34);
  for (let y = 0; y < height; y += 1) {
    const row = pixelOffset + y * rowStride;
    for (let x = 0; x < width; x += 1) {
      const offset = row + x * 3;
      buffer[offset] = color.b;
      buffer[offset + 1] = color.g;
      buffer[offset + 2] = color.r;
    }
  }
  fs.writeFileSync(filePath, buffer);
}

function simdeckJson(args, options = {}) {
  return JSON.parse(
    runText(simdeck, args, { timeoutMs: commandTimeoutMs, ...options }),
  );
}

function runJson(command, args, options = {}) {
  return JSON.parse(runText(command, args, options));
}

function runText(command, args, options = {}) {
  const result = spawnSync(command, args, {
    cwd: root,
    encoding: "utf8",
    timeout: options.timeoutMs ?? commandTimeoutMs,
    maxBuffer: 1024 * 1024 * 8,
  });
  if (result.error) {
    throw result.error;
  }
  if (result.status !== 0 && !options.allowFailure) {
    throw new Error(
      `${command} ${args.join(" ")} failed with status ${result.status}: ${[
        result.stderr,
        result.stdout,
      ]
        .filter(Boolean)
        .join("\n")}`,
    );
  }
  const output = `${result.stdout ?? ""}${result.stderr ?? ""}`;
  if (verbose && output.trim()) {
    process.stderr.write(output);
  }
  return result.stdout ?? "";
}

async function retryRunText(command, args, options) {
  let lastError;
  for (let attempt = 1; attempt <= options.attempts; attempt += 1) {
    try {
      return runText(command, args, options);
    } catch (error) {
      lastError = error;
      if (attempt < options.attempts) {
        await sleep(options.delayMs);
      }
    }
  }
  throw lastError;
}

function cleanup() {
  if (chromeProcess) {
    try {
      chromeProcess.kill("SIGTERM");
    } catch {}
    chromeProcess = null;
  }
  if (simulatorUDID) {
    try {
      simdeckJson(["camera", "stop", simulatorUDID], { timeoutMs: 30_000 });
    } catch {}
    if (!keepSimulator) {
      try {
        runText("xcrun", ["simctl", "shutdown", simulatorUDID], {
          allowFailure: true,
          timeoutMs: 120_000,
        });
      } catch {}
      try {
        runText("xcrun", ["simctl", "delete", simulatorUDID], {
          allowFailure: true,
          timeoutMs: 120_000,
        });
      } catch {}
    }
  }
  if (!keepSimulator) {
    try {
      fs.rmSync(tempRoot, { recursive: true, force: true });
    } catch {}
  }
}

function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

function step(label) {
  console.log(`[camera-it] ${label}`);
}
