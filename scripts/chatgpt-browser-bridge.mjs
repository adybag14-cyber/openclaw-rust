#!/usr/bin/env node

import fs from "node:fs/promises";
import http from "node:http";
import { createRequire } from "node:module";
import path from "node:path";
import process from "node:process";
import { fileURLToPath } from "node:url";

const HOST = process.env.OPENCLAW_CHATGPT_BRIDGE_HOST || "127.0.0.1";
const PORT = Number.parseInt(process.env.OPENCLAW_CHATGPT_BRIDGE_PORT || "43010", 10);
const PROFILE_DIR =
  process.env.OPENCLAW_CHATGPT_PROFILE_DIR ||
  path.join(".openclaw-rs", "chatgpt-browser-profile");
const BASE_ORIGIN = "https://chatgpt.com";
const DEFAULT_MODEL = "gpt-5-2";
const COMPLETION_TIMEOUT_MS = Number.parseInt(
  process.env.OPENCLAW_CHATGPT_COMPLETION_TIMEOUT_MS || "180000",
  10,
);
const SCRIPT_DIR = path.dirname(fileURLToPath(import.meta.url));

let pwState = null;
let ppState = null;
let pwInitError = null;
let ppInitError = null;

function parseJsonSafe(text) {
  try {
    return JSON.parse(text);
  } catch {
    return null;
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
          path.join(nodeModulesPath, "__openclaw_chatgpt_bridge__.cjs"),
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

function trimText(value) {
  if (typeof value !== "string") {
    return "";
  }
  return value.trim();
}

function stripProviderPrefix(model) {
  const raw = trimText(model);
  if (!raw) {
    return "";
  }
  const parts = raw.split("/");
  return parts[parts.length - 1] || raw;
}

function parseBrowserMode(model) {
  const normalized = stripProviderPrefix(model).toLowerCase().replaceAll("_", "-");
  if (!normalized) {
    return null;
  }
  if (normalized.includes("instant")) {
    return "Instant";
  }
  if (normalized.includes("thinking")) {
    return "Thinking";
  }
  if (normalized.endsWith("-pro") || normalized.includes(".pro") || normalized.includes(" pro")) {
    return "Pro";
  }
  if (normalized.includes("auto")) {
    return "Auto";
  }
  return null;
}

function normalizeModelSlug(model) {
  const normalized = stripProviderPrefix(model).toLowerCase().replaceAll("_", "-");
  if (!normalized) {
    return DEFAULT_MODEL;
  }
  if (normalized.includes("gpt-5.2") || normalized.startsWith("gpt-5-2")) {
    return "gpt-5-2";
  }
  if (normalized.includes("gpt-5.1") || normalized.startsWith("gpt-5-1")) {
    return "gpt-5-1";
  }
  if (normalized.includes("gpt-5-mini") || normalized.includes("gpt-5mini")) {
    return "gpt-5-mini";
  }
  if (normalized.startsWith("gpt-5")) {
    return "gpt-5";
  }
  if (normalized.startsWith("gpt-4o")) {
    return "gpt-4o";
  }
  return normalized;
}

function normalizeMessageText(content) {
  if (typeof content === "string") {
    return content.trim();
  }
  if (Array.isArray(content)) {
    return content
      .map((part) => {
        if (typeof part === "string") {
          return part;
        }
        if (part && typeof part === "object" && typeof part.text === "string") {
          return part.text;
        }
        return "";
      })
      .join("\n")
      .trim();
  }
  if (content && typeof content === "object") {
    if (typeof content.text === "string") {
      return content.text.trim();
    }
    if (Array.isArray(content.parts)) {
      return content.parts
        .filter((item) => typeof item === "string")
        .join("\n")
        .trim();
    }
  }
  return "";
}

function extractUserPrompt(messages) {
  if (!Array.isArray(messages)) {
    return "";
  }
  for (let i = messages.length - 1; i >= 0; i -= 1) {
    const item = messages[i];
    if (!item || typeof item !== "object") {
      continue;
    }
    const role = trimText(item.role).toLowerCase();
    if (role !== "user") {
      continue;
    }
    const text = normalizeMessageText(item.content);
    if (text) {
      return text;
    }
  }
  return "";
}

async function readSessionState(page) {
  return page.evaluate(async () => {
    try {
      const response = await fetch("/api/auth/session", {
        method: "GET",
        credentials: "include",
        cache: "no-store",
      });
      const payload = await response.json().catch(() => ({}));
      const accessToken =
        typeof payload?.accessToken === "string" ? payload.accessToken.trim() : "";
      const email = typeof payload?.user?.email === "string" ? payload.user.email : null;
      return {
        ok: response.ok && accessToken.length > 0,
        status: response.status,
        hasAccessToken: accessToken.length > 0,
        email,
      };
    } catch (error) {
      return {
        ok: false,
        status: 0,
        hasAccessToken: false,
        email: null,
        error: error instanceof Error ? error.message : String(error),
      };
    }
  });
}

async function waitForComposer(page, timeoutMs = 60_000) {
  const started = Date.now();
  while (Date.now() - started < timeoutMs) {
    const ready = await page.evaluate(() => {
      return Boolean(
        document.querySelector("#prompt-textarea") ||
          document.querySelector('textarea') ||
          document.querySelector('[contenteditable="true"]'),
      );
    });
    if (ready) {
      return true;
    }
    await sleep(200);
  }
  return false;
}

async function applyThinkingModeIfNeeded(page, mode) {
  if (!mode) {
    return;
  }
  await page.evaluate((targetMode) => {
    function clickElement(element) {
      if (!element) {
        return false;
      }
      element.dispatchEvent(new MouseEvent("mousedown", { bubbles: true }));
      element.dispatchEvent(new MouseEvent("mouseup", { bubbles: true }));
      element.dispatchEvent(new MouseEvent("click", { bubbles: true }));
      if (typeof element.click === "function") {
        element.click();
      }
      return true;
    }

    const toggleCandidates = Array.from(
      document.querySelectorAll("button,[role='button'],div[role='button'],span"),
    ).filter((element) => {
      const text = (element.innerText || "").trim().toLowerCase();
      if (!text || text.length > 80) {
        return false;
      }
      return (
        text.includes("extended pro") ||
        text === "auto" ||
        text === "instant" ||
        text === "thinking" ||
        text === "pro"
      );
    });

    if (toggleCandidates.length > 0) {
      clickElement(toggleCandidates[0]);
    }

    const desired = targetMode.toLowerCase();
    const optionCandidates = Array.from(
      document.querySelectorAll("button,[role='menuitem'],[role='option'],li,div"),
    ).filter((element) => {
      const text = (element.innerText || "").trim().toLowerCase();
      return text === desired;
    });
    if (optionCandidates.length > 0) {
      clickElement(optionCandidates[0]);
    }
  }, mode);
}

async function submitPromptAndWaitForReply(page, prompt, timeoutMs) {
  const baseline = await page.evaluate(() => {
    return {
      userCount: document.querySelectorAll('[data-message-author-role="user"]').length,
      assistantCount: document.querySelectorAll('[data-message-author-role="assistant"]').length,
    };
  });

  const typed = await page.evaluate((text) => {
    const composer =
      document.querySelector("#prompt-textarea") ||
      document.querySelector('[contenteditable="true"]') ||
      document.querySelector("textarea");
    if (!composer) {
      return false;
    }
    composer.focus();
    if (composer.tagName === "TEXTAREA") {
      composer.value = text;
      composer.dispatchEvent(new Event("input", { bubbles: true }));
      return true;
    }
    composer.textContent = text;
    composer.dispatchEvent(new InputEvent("input", { bubbles: true }));
    return true;
  }, prompt);

  if (!typed) {
    throw new Error("composer not available for prompt submit");
  }

  await page.keyboard.press("Enter");

  const started = Date.now();
  let stableText = "";
  let stableTicks = 0;
  while (Date.now() - started < timeoutMs) {
    await sleep(900);
    const state = await page.evaluate((baselineAssistantCount) => {
      const assistants = Array.from(
        document.querySelectorAll('[data-message-author-role="assistant"]'),
      )
        .map((element) => (element.innerText || "").trim())
        .filter((text) => Boolean(text));
      const lastText = assistants.length > 0 ? assistants[assistants.length - 1] : "";
      const stopVisible = Boolean(
        document.querySelector('button[data-testid="stop-button"]') ||
          document.querySelector('button[aria-label*="Stop"]') ||
          document.querySelector('button[aria-label*="stop"]'),
      );
      return {
        assistantCount: assistants.length,
        hasNewAssistant: assistants.length > baselineAssistantCount,
        lastText,
        stopVisible,
      };
    }, baseline.assistantCount);

    if (!state.hasNewAssistant || !state.lastText) {
      continue;
    }
    if (state.lastText === stableText) {
      stableTicks += 1;
    } else {
      stableText = state.lastText;
      stableTicks = 1;
    }
    if (!state.stopVisible && stableTicks >= 2) {
      return state.lastText;
    }
  }
  throw new Error("assistant response did not complete before timeout");
}

async function completeViaPage(page, payload) {
  const prompt = extractUserPrompt(payload.messages);
  if (!prompt) {
    throw new Error("no user prompt provided");
  }
  const requestedModel = trimText(payload.model) || DEFAULT_MODEL;
  const modelSlug = normalizeModelSlug(requestedModel);
  const mode = parseBrowserMode(requestedModel);

  await page.goto(`${BASE_ORIGIN}/?model=${encodeURIComponent(modelSlug)}`, {
    waitUntil: "domcontentloaded",
    timeout: 60_000,
  });

  const composerReady = await waitForComposer(page, 60_000);
  if (!composerReady) {
    throw new Error("chat composer not ready");
  }

  const sessionState = await readSessionState(page);
  if (!sessionState.ok) {
    throw new Error(
      `chatgpt session unavailable (status=${sessionState.status}, hasToken=${sessionState.hasAccessToken})`,
    );
  }

  await applyThinkingModeIfNeeded(page, mode);
  const assistantText = await submitPromptAndWaitForReply(
    page,
    prompt,
    Math.max(20_000, COMPLETION_TIMEOUT_MS),
  );

  return {
    id: `chatcmpl-chatgpt-browser-${Date.now()}`,
    object: "chat.completion",
    created: Math.floor(Date.now() / 1000),
    model: modelSlug,
    choices: [
      {
        index: 0,
        message: {
          role: "assistant",
          content: assistantText,
        },
        finish_reason: "stop",
      },
    ],
  };
}

async function ensurePlaywright() {
  if (pwState) {
    return pwState;
  }
  try {
    const playwright = await importModuleWithFallback("playwright");
    const context = await playwright.chromium.launchPersistentContext(PROFILE_DIR, {
      headless: false,
      viewport: null,
      args: ["--disable-blink-features=AutomationControlled"],
    });
    const page = context.pages()[0] ?? (await context.newPage());
    pwState = { context, page };
    pwInitError = null;
    return pwState;
  } catch (error) {
    pwInitError = error instanceof Error ? error.message : String(error);
    if (pwState?.context) {
      try {
        await pwState.context.close();
      } catch {}
    }
    pwState = null;
    return null;
  }
}

async function ensurePuppeteer() {
  if (ppState) {
    return ppState;
  }
  try {
    const puppeteer = await importModuleWithFallback("puppeteer");
    const browser = await puppeteer.launch({
      headless: false,
      userDataDir: PROFILE_DIR,
      defaultViewport: null,
      args: ["--disable-blink-features=AutomationControlled"],
    });
    const pages = await browser.pages();
    const page = pages[0] ?? (await browser.newPage());
    ppState = { browser, page };
    ppInitError = null;
    return ppState;
  } catch (error) {
    ppInitError = error instanceof Error ? error.message : String(error);
    if (ppState?.browser) {
      try {
        await ppState.browser.close();
      } catch {}
    }
    ppState = null;
    return null;
  }
}

async function completionViaPlaywright(payload) {
  const state = await ensurePlaywright();
  if (!state) {
    return {
      ok: false,
      provider: "playwright",
      error: pwInitError || "playwright unavailable",
    };
  }
  try {
    const body = await completeViaPage(state.page, payload);
    return { ok: true, provider: "playwright", body };
  } catch (error) {
    if (pwState?.context) {
      try {
        await pwState.context.close();
      } catch {}
    }
    pwState = null;
    return {
      ok: false,
      provider: "playwright",
      error: error instanceof Error ? error.message : String(error),
    };
  }
}

async function completionViaPuppeteer(payload) {
  const state = await ensurePuppeteer();
  if (!state) {
    return {
      ok: false,
      provider: "puppeteer",
      error: ppInitError || "puppeteer unavailable",
    };
  }
  try {
    const body = await completeViaPage(state.page, payload);
    return { ok: true, provider: "puppeteer", body };
  } catch (error) {
    return {
      ok: false,
      provider: "puppeteer",
      error: error instanceof Error ? error.message : String(error),
    };
  }
}

function readBody(req) {
  return new Promise((resolve, reject) => {
    let body = "";
    req.on("data", (chunk) => {
      body += chunk.toString("utf8");
      if (body.length > 5_000_000) {
        reject(new Error("request body too large"));
      }
    });
    req.on("error", reject);
    req.on("end", () => resolve(body));
  });
}

function writeJson(res, statusCode, payload) {
  res.writeHead(statusCode, {
    "Content-Type": "application/json",
    "Cache-Control": "no-store",
  });
  res.end(JSON.stringify(payload));
}

async function handleChatCompletion(req, res) {
  const raw = await readBody(req);
  const payload = parseJsonSafe(raw);
  if (!payload || typeof payload !== "object") {
    writeJson(res, 400, { error: "invalid JSON body" });
    return;
  }

  const attempts = [];
  const playwrightResult = await completionViaPlaywright(payload);
  attempts.push(playwrightResult);
  if (playwrightResult.ok) {
    writeJson(res, 200, playwrightResult.body);
    return;
  }

  const puppeteerResult = await completionViaPuppeteer(payload);
  attempts.push(puppeteerResult);
  if (puppeteerResult.ok) {
    writeJson(res, 200, puppeteerResult.body);
    return;
  }

  writeJson(res, 502, {
    error: "all browser providers failed",
    attempts,
  });
}

const server = http.createServer(async (req, res) => {
  try {
    if (!req.url) {
      writeJson(res, 404, { error: "not found" });
      return;
    }
    if (req.method === "GET" && req.url === "/health") {
      writeJson(res, 200, {
        ok: true,
        bridge: "chatgpt-browser-bridge",
        playwrightReady: Boolean(pwState),
        puppeteerReady: Boolean(ppState),
      });
      return;
    }
    if (
      req.method === "POST" &&
      (req.url === "/v1/chat/completions" ||
        req.url === "/api/v1/chat/completions" ||
        req.url === "/api/chat/completions")
    ) {
      await handleChatCompletion(req, res);
      return;
    }
    writeJson(res, 404, { error: "not found" });
  } catch (error) {
    writeJson(res, 500, { error: error instanceof Error ? error.message : String(error) });
  }
});

async function shutdown() {
  server.close();
  if (pwState?.context) {
    try {
      await pwState.context.close();
    } catch {}
  }
  if (ppState?.browser) {
    try {
      await ppState.browser.close();
    } catch {}
  }
  process.exit(0);
}

await ensureDir(PROFILE_DIR);
server.listen(PORT, HOST, () => {
  // eslint-disable-next-line no-console
  console.log(`chatgpt browser bridge listening on http://${HOST}:${PORT}`);
});
server.on("error", (error) => {
  // eslint-disable-next-line no-console
  console.error(
    `chatgpt browser bridge failed to bind ${HOST}:${PORT}: ${
      error instanceof Error ? error.message : String(error)
    }`,
  );
  process.exit(1);
});

process.on("SIGINT", () => {
  void shutdown();
});
process.on("SIGTERM", () => {
  void shutdown();
});
