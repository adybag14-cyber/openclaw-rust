#!/usr/bin/env node

import fs from "node:fs/promises";
import path from "node:path";
import process from "node:process";

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

function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
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
      out.error = error instanceof Error ? error.message : String(error);
      return out;
    }
  });
}

async function runPlaywrightFlow(options) {
  const playwright = await import("playwright");
  const context = await playwright.chromium.launchPersistentContext(options.profileDir, {
    headless: false,
    viewport: null,
    args: ["--disable-blink-features=AutomationControlled"],
  });

  try {
    const page = context.pages()[0] ?? (await context.newPage());
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
          source: "chatgpt-browser-playwright",
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
      source: "chatgpt-browser-playwright",
      message:
        "Login still pending. Complete ChatGPT sign-in in the opened browser and retry /auth wait.",
    };
  } finally {
    await context.close();
  }
}

async function runPuppeteerFlow(options) {
  const puppeteer = await import("puppeteer");
  const browser = await puppeteer.launch({
    headless: false,
    userDataDir: options.profileDir,
    defaultViewport: null,
    args: ["--disable-blink-features=AutomationControlled"],
  });

  try {
    const pages = await browser.pages();
    const page = pages[0] ?? (await browser.newPage());
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
          source: "chatgpt-browser-puppeteer",
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
      source: "chatgpt-browser-puppeteer",
      message:
        "Login still pending. Complete ChatGPT sign-in in the opened browser and retry /auth wait.",
    };
  } finally {
    await browser.close();
  }
}

async function main() {
  const args = parseArgs(process.argv.slice(2));
  const providerId = normalizeText(args["provider"], 128) ?? "openai";
  const accountId = normalizeText(args["account-id"], 128) ?? "default";
  const sessionId = normalizeText(args["session-id"], 128) ?? "session";
  const timeoutMs = normalizeTimeoutMs(args["timeout-ms"]);
  const engine = normalizeText(args["engine"], 64);

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
  };

  const errors = [];
  if (!engine || engine === "playwright") {
    try {
      const result = await runPlaywrightFlow(options);
      process.stdout.write(JSON.stringify(result));
      return;
    } catch (error) {
      errors.push(`playwright: ${error instanceof Error ? error.message : String(error)}`);
      if (engine === "playwright") {
        process.stdout.write(
          JSON.stringify({
            ok: false,
            status: "error",
            providerId,
            accountId,
            sessionId,
            error: errors[errors.length - 1],
          }),
        );
        process.exitCode = 1;
        return;
      }
    }
  }

  if (!engine || engine === "puppeteer") {
    try {
      const result = await runPuppeteerFlow(options);
      process.stdout.write(JSON.stringify(result));
      return;
    } catch (error) {
      errors.push(`puppeteer: ${error instanceof Error ? error.message : String(error)}`);
      if (engine === "puppeteer") {
        process.stdout.write(
          JSON.stringify({
            ok: false,
            status: "error",
            providerId,
            accountId,
            sessionId,
            error: errors[errors.length - 1],
          }),
        );
        process.exitCode = 1;
        return;
      }
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
        "No browser automation engine available. Install playwright or puppeteer.",
    }),
  );
  process.exitCode = 1;
}

main().catch((error) => {
  process.stdout.write(
    JSON.stringify({
      ok: false,
      status: "error",
      error: error instanceof Error ? error.message : String(error),
    }),
  );
  process.exitCode = 1;
});

