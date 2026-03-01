#!/usr/bin/env node

import fs from "node:fs/promises";
import { createRequire } from "node:module";
import path from "node:path";
import process from "node:process";
import { fileURLToPath } from "node:url";

const SCRIPT_DIR = path.dirname(fileURLToPath(import.meta.url));

function parseArgs(argv) {
  const out = {};
  for (let i = 0; i < argv.length; i += 1) {
    const raw = argv[i];
    if (!raw.startsWith("--")) {
      continue;
    }
    const key = raw.slice(2);
    const next = argv[i + 1];
    if (!next || next.startsWith("--")) {
      out[key] = true;
      continue;
    }
    out[key] = next;
    i += 1;
  }
  return out;
}

function normalizeText(value, maxLen = 4096) {
  if (typeof value !== "string") {
    return null;
  }
  const trimmed = value.trim();
  if (!trimmed) {
    return null;
  }
  if (trimmed.length <= maxLen) {
    return trimmed;
  }
  return `${trimmed.slice(0, maxLen)}...`;
}

function normalizeTimeoutMs(raw, fallback = 300_000) {
  if (typeof raw !== "string") {
    return fallback;
  }
  const parsed = Number.parseInt(raw, 10);
  if (!Number.isFinite(parsed) || parsed <= 0) {
    return fallback;
  }
  return Math.min(Math.max(parsed, 5_000), 900_000);
}

function normalizeEndpoint(raw) {
  if (typeof raw !== "string") {
    return null;
  }
  const trimmed = raw.trim();
  if (!trimmed) {
    return null;
  }
  if (
    trimmed.startsWith("ws://") ||
    trimmed.startsWith("wss://") ||
    trimmed.startsWith("http://") ||
    trimmed.startsWith("https://")
  ) {
    return trimmed;
  }
  return `ws://${trimmed}`;
}

function normalizeEngine(raw) {
  const normalized = normalizeText(raw, 64);
  if (!normalized) {
    return null;
  }
  return normalized.toLowerCase().replaceAll("_", "-");
}

function formatError(error) {
  if (error instanceof Error) {
    return error.message;
  }
  if (typeof error === "string") {
    return error;
  }
  if (error && typeof error === "object") {
    const maybeMessage = error.message || error.error || error.reason;
    if (typeof maybeMessage === "string" && maybeMessage.trim()) {
      return maybeMessage;
    }
    if (typeof error.toString === "function") {
      const text = error.toString();
      if (typeof text === "string" && text.trim() && text !== "[object Object]") {
        return text;
      }
    }
  }
  try {
    return JSON.stringify(error);
  } catch {
    return String(error);
  }
}

function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

async function importModuleWithFallback(moduleName) {
  try {
    return await import(moduleName);
  } catch (primaryError) {
    const fallbackNodeModules = [
      path.join(process.cwd(), "node_modules"),
      path.join(SCRIPT_DIR, "..", "node_modules"),
      path.join(SCRIPT_DIR, "..", "tmp_mercury_bridge", "node_modules"),
      path.join(SCRIPT_DIR, "..", "..", "tmp-chatgpt-auth", "node_modules"),
    ];
    for (const nodeModulesPath of fallbackNodeModules) {
      try {
        const requireFromPath = createRequire(
          path.join(nodeModulesPath, "__openclaw_chatgpt_auth__.cjs"),
        );
        return requireFromPath(moduleName);
      } catch {}
    }
    throw primaryError;
  }
}

async function ensureDir(dir) {
  await fs.mkdir(dir, { recursive: true });
}

async function readChatGptSession(page) {
  return page.evaluate(async () => {
    const out = {
      ok: false,
      accessToken: null,
      expiresAtMs: null,
      email: null,
      error: null,
    };
    try {
      const res = await fetch("/api/auth/session", {
        method: "GET",
        credentials: "include",
      });
      if (!res.ok) {
        out.error = `session_status_${res.status}`;
        return out;
      }
      const data = await res.json();
      const token = typeof data?.accessToken === "string" ? data.accessToken.trim() : "";
      if (!token) {
        out.error = "missing_access_token";
        return out;
      }
      out.ok = true;
      out.accessToken = token;
      if (typeof data?.expires === "string") {
        const expires = Date.parse(data.expires);
        if (Number.isFinite(expires)) {
          out.expiresAtMs = Math.floor(expires);
        }
      }
      out.email = typeof data?.user?.email === "string" ? data.user.email : null;
      return out;
    } catch (error) {
      out.error = formatError(error);
      return out;
    }
  });
}

function lightpandaConnectOptions(endpoint) {
  if (endpoint.startsWith("http://") || endpoint.startsWith("https://")) {
    return { browserURL: endpoint };
  }
  return { browserWSEndpoint: endpoint };
}

async function captureSessionFromPage(page, options, source) {
  await page.goto("https://chatgpt.com/", {
    waitUntil: "domcontentloaded",
    timeout: 60_000,
  });

  const deadline = Date.now() + options.timeoutMs;
  while (Date.now() < deadline) {
    const session = await readChatGptSession(page);
    if (session.ok && session.accessToken) {
      return {
        ok: true,
        status: "connected",
        providerId: options.providerId,
        accountId: options.accountId,
        sessionId: options.sessionId,
        accessToken: session.accessToken,
        expiresAtMs: session.expiresAtMs ?? undefined,
        source,
        message: `ChatGPT browser session captured (${session.email ?? "unknown account"}).`,
      };
    }
    await sleep(2_000);
  }

  return {
    ok: true,
    status: "pending",
    providerId: options.providerId,
    accountId: options.accountId,
    sessionId: options.sessionId,
    source,
    message:
      "Login still pending. Complete ChatGPT sign-in in the opened browser and retry /auth wait.",
  };
}

async function runPlaywrightFlow(options) {
  const playwright = await importModuleWithFallback("playwright");
  const context = await playwright.chromium.launchPersistentContext(options.profileDir, {
    headless: false,
    viewport: null,
    args: ["--disable-blink-features=AutomationControlled"],
  });

  try {
    const page = context.pages()[0] ?? (await context.newPage());
    return await captureSessionFromPage(page, options, "chatgpt-browser-playwright");
  } finally {
    await context.close();
  }
}

async function runPuppeteerFlow(options) {
  const puppeteer = await importModuleWithFallback("puppeteer");
  const browser = await puppeteer.launch({
    headless: false,
    userDataDir: options.profileDir,
    defaultViewport: null,
    args: ["--disable-blink-features=AutomationControlled"],
  });

  try {
    const pages = await browser.pages();
    const page = pages[0] ?? (await browser.newPage());
    return await captureSessionFromPage(page, options, "chatgpt-browser-puppeteer");
  } finally {
    await browser.close();
  }
}

async function runLightpandaPlaywrightFlow(options) {
  if (!options.lightpandaEndpoint) {
    throw new Error(
      "lightpanda endpoint is missing; set --lightpanda-endpoint or OPENCLAW_CHATGPT_LIGHTPANDA_WS_ENDPOINT",
    );
  }
  const playwright = await importModuleWithFallback("playwright");
  const browser = await playwright.chromium.connectOverCDP(options.lightpandaEndpoint);
  try {
    const context = browser.contexts()[0] ?? (await browser.newContext({ viewport: null }));
    const page = context.pages()[0] ?? (await context.newPage());
    return await captureSessionFromPage(page, options, "chatgpt-browser-lightpanda-playwright");
  } finally {
    await browser.close();
  }
}

async function runLightpandaPuppeteerFlow(options) {
  if (!options.lightpandaEndpoint) {
    throw new Error(
      "lightpanda endpoint is missing; set --lightpanda-endpoint or OPENCLAW_CHATGPT_LIGHTPANDA_WS_ENDPOINT",
    );
  }
  const puppeteer = await importModuleWithFallback("puppeteer");
  const browser = await puppeteer.connect(lightpandaConnectOptions(options.lightpandaEndpoint));
  try {
    const pages = await browser.pages();
    const page = pages[0] ?? (await browser.newPage());
    return await captureSessionFromPage(page, options, "chatgpt-browser-lightpanda-puppeteer");
  } finally {
    if (typeof browser.disconnect === "function") {
      await browser.disconnect();
    } else {
      await browser.close();
    }
  }
}

async function main() {
  const args = parseArgs(process.argv.slice(2));
  const providerId = normalizeText(args["provider"], 128) ?? "openai";
  const accountId = normalizeText(args["account-id"], 128) ?? "default";
  const sessionId = normalizeText(args["session-id"], 128) ?? "session";
  const timeoutMs = normalizeTimeoutMs(args["timeout-ms"]);
  const engine = normalizeEngine(args["engine"]);
  const lightpandaEndpoint =
    normalizeEndpoint(args["lightpanda-endpoint"]) ??
    normalizeEndpoint(args["lightpanda-ws-endpoint"]) ??
    normalizeEndpoint(process.env.OPENCLAW_CHATGPT_LIGHTPANDA_WS_ENDPOINT) ??
    normalizeEndpoint(process.env.OPENCLAW_LIGHTPANDA_WS_ENDPOINT);

  const profileDir =
    normalizeText(args["profile-dir"], 2048) ??
    path.join(".openclaw-rs", "chatgpt-browser-profile");
  await ensureDir(profileDir);

  const options = {
    providerId,
    accountId,
    sessionId,
    timeoutMs,
    profileDir,
    lightpandaEndpoint,
  };

  const localPlans = [
    { name: "playwright", run: runPlaywrightFlow },
    { name: "puppeteer", run: runPuppeteerFlow },
  ];
  const lightpandaPlans = [
    { name: "lightpanda-playwright", run: runLightpandaPlaywrightFlow },
    { name: "lightpanda-puppeteer", run: runLightpandaPuppeteerFlow },
  ];

  const planByEngine = new Map([
    ["playwright", [localPlans[0]]],
    ["puppeteer", [localPlans[1]]],
    ["lightpanda-playwright", [lightpandaPlans[0]]],
    ["lightpanda-puppeteer", [lightpandaPlans[1]]],
    ["lightpanda", lightpandaPlans],
  ]);

  const planQueue = [];
  if (engine) {
    const explicit = planByEngine.get(engine);
    if (!explicit) {
      process.stdout.write(
        JSON.stringify({
          ok: false,
          status: "error",
          providerId,
          accountId,
          sessionId,
          error: `unsupported engine '${engine}'`,
        }),
      );
      process.exitCode = 1;
      return;
    }
    planQueue.push(...explicit);
  } else {
    if (lightpandaEndpoint) {
      planQueue.push(...lightpandaPlans);
    }
    planQueue.push(...localPlans);
  }

  const errors = [];
  for (const plan of planQueue) {
    try {
      const result = await plan.run(options);
      process.stdout.write(JSON.stringify(result));
      return;
    } catch (error) {
      errors.push(`${plan.name}: ${formatError(error)}`);
    }
  }

  process.stdout.write(
    JSON.stringify({
      ok: false,
      status: "error",
      providerId,
      accountId,
      sessionId,
      error: normalizeText(errors.join(" | "), 8_192) ??
        "No browser automation engine available. Install playwright/puppeteer or configure Lightpanda endpoint.",
    }),
  );
  process.exitCode = 1;
}

main().catch((error) => {
  process.stdout.write(
    JSON.stringify({
      ok: false,
      status: "error",
      error: formatError(error),
    }),
  );
  process.exitCode = 1;
});

