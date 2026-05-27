# OpenCLI Violentmonkey Bridge Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a local `opencli vm ...` integration that uses a Violentmonkey bridge userscript to inject or evaluate JavaScript in a named OpenCLI browser session.

**Architecture:** Implement `vm` as an OpenCLI external CLI passthrough instead of a normal adapter, because normal adapters do not accept `--session` while `opencli browser <session> ...` does. The external binary shells out to `opencli browser <session> eval <dispatcher-js>`, dispatches a request event to the Violentmonkey bridge in the active tab, waits for the response event, and prints JSON.

**Tech Stack:** OpenCLI external CLI registration, Node.js ESM CLI, Node built-in test runner, Violentmonkey userscript APIs, Chrome page `CustomEvent` bridge.

---

## File Structure

- Create `/Users/lishuo121/.opencli/bin/opencli-vm`: executable Node CLI registered as `opencli vm`.
- Create `/Users/lishuo121/.opencli/lib/opencli-vm/metadata.mjs`: parse userscript metadata and match URLs.
- Create `/Users/lishuo121/.opencli/lib/opencli-vm/envelope.mjs`: create request envelopes and normalize bridge responses.
- Create `/Users/lishuo121/.opencli/lib/opencli-vm/bridge-source.mjs`: generate the installable `OpenCLI VM Bridge` userscript.
- Create `/Users/lishuo121/.opencli/lib/opencli-vm/dispatch.mjs`: call `opencli browser <session> eval` and wait for bridge responses.
- Create `/Users/lishuo121/.opencli/lib/opencli-vm/metadata.test.mjs`: parser and URL matching tests.
- Create `/Users/lishuo121/.opencli/lib/opencli-vm/envelope.test.mjs`: request and response tests.

Writing these files requires sandbox escalation because they live under `/Users/lishuo121/.opencli`, outside the repository writable root.

## Task 1: Metadata Parser

**Files:**
- Create: `/Users/lishuo121/.opencli/lib/opencli-vm/metadata.mjs`
- Test: `/Users/lishuo121/.opencli/lib/opencli-vm/metadata.test.mjs`

- [ ] **Step 1: Write the failing test**

```js
import assert from 'node:assert/strict';
import test from 'node:test';

import { parseUserscriptMetadata, matchesUserscriptUrl } from './metadata.mjs';

const SAMPLE = `// ==UserScript==
// @name         Demo Script
// @match        https://example.com/*
// @include      https://*.example.org/path/*
// @exclude      https://example.com/private/*
// @run-at       document-idle
// @grant        GM_getValue
// @grant        GM_setValue
// ==/UserScript==
return document.title;
`;

test('parses userscript metadata block', () => {
  const parsed = parseUserscriptMetadata(SAMPLE);
  assert.equal(parsed.name, 'Demo Script');
  assert.deepEqual(parsed.match, ['https://example.com/*']);
  assert.deepEqual(parsed.include, ['https://*.example.org/path/*']);
  assert.deepEqual(parsed.exclude, ['https://example.com/private/*']);
  assert.equal(parsed.runAt, 'document-idle');
  assert.deepEqual(parsed.grant, ['GM_getValue', 'GM_setValue']);
  assert.equal(parsed.body.trim(), 'return document.title;');
});

test('plain JavaScript defaults to anonymous script and matches any URL', () => {
  const parsed = parseUserscriptMetadata('return location.href;');
  assert.equal(parsed.name, 'anonymous-opencli-script');
  assert.deepEqual(parsed.match, []);
  assert.equal(matchesUserscriptUrl(parsed, 'https://any.test/'), true);
});

test('matches positive patterns and rejects excludes', () => {
  const parsed = parseUserscriptMetadata(SAMPLE);
  assert.equal(matchesUserscriptUrl(parsed, 'https://example.com/news'), true);
  assert.equal(matchesUserscriptUrl(parsed, 'https://example.com/private/1'), false);
  assert.equal(matchesUserscriptUrl(parsed, 'https://docs.example.org/path/a'), true);
  assert.equal(matchesUserscriptUrl(parsed, 'https://other.test/'), false);
});
```

- [ ] **Step 2: Run test to verify it fails**

```bash
node --test /Users/lishuo121/.opencli/lib/opencli-vm/metadata.test.mjs
```

Expected: fails because `metadata.mjs` does not exist.

- [ ] **Step 3: Implement parser**

```js
const DEFAULT_METADATA = Object.freeze({
  name: 'anonymous-opencli-script',
  match: [],
  include: [],
  exclude: [],
  grant: [],
  runAt: 'document-idle',
});

export function parseUserscriptMetadata(source) {
  const text = String(source ?? '');
  const start = text.indexOf('// ==UserScript==');
  const end = text.indexOf('// ==/UserScript==');
  if (start === -1 || end === -1 || end < start) return { ...DEFAULT_METADATA, body: text };

  const meta = { ...DEFAULT_METADATA, match: [], include: [], exclude: [], grant: [] };
  const block = text.slice(start, end).split(/\r?\n/);
  const body = text.slice(end + '// ==/UserScript=='.length).replace(/^\s*\r?\n/, '');

  for (const line of block) {
    const item = line.match(/^\s*\/\/\s*@(\S+)\s+(.+?)\s*$/);
    if (!item) continue;
    const key = item[1];
    const value = item[2].trim();
    if (key === 'name') meta.name = value;
    if (key === 'match') meta.match.push(value);
    if (key === 'include') meta.include.push(value);
    if (key === 'exclude') meta.exclude.push(value);
    if (key === 'grant') meta.grant.push(value);
    if (key === 'run-at') meta.runAt = value;
  }

  return { ...meta, body };
}

export function matchesUserscriptUrl(metadata, url) {
  const target = String(url ?? '');
  if (!target) return false;
  if ((metadata.exclude ?? []).some((pattern) => wildcardUrlMatches(pattern, target))) return false;
  const positives = [...(metadata.match ?? []), ...(metadata.include ?? [])];
  if (positives.length === 0) return true;
  return positives.some((pattern) => wildcardUrlMatches(pattern, target));
}

function wildcardUrlMatches(pattern, url) {
  const escaped = String(pattern).replace(/[.+?^${}()|[\]\\]/g, '\\$&').replace(/\*/g, '.*');
  return new RegExp(`^${escaped}$`).test(url);
}
```

- [ ] **Step 4: Run test to verify it passes**

```bash
node --test /Users/lishuo121/.opencli/lib/opencli-vm/metadata.test.mjs
```

Expected: all metadata tests pass.

## Task 2: Envelope Helpers

**Files:**
- Create: `/Users/lishuo121/.opencli/lib/opencli-vm/envelope.mjs`
- Test: `/Users/lishuo121/.opencli/lib/opencli-vm/envelope.test.mjs`

- [ ] **Step 1: Write the failing test**

```js
import assert from 'node:assert/strict';
import test from 'node:test';

import { createRequest, normalizeBridgeResponse } from './envelope.mjs';

test('creates request envelopes', () => {
  const request = createRequest('eval', { source: 'return 1;' }, 2000);
  assert.match(request.requestId, /^opencli-vm-/);
  assert.equal(request.type, 'eval');
  assert.deepEqual(request.payload, { source: 'return 1;' });
  assert.equal(request.timeoutMs, 2000);
});

test('normalizes successful response', () => {
  const normalized = normalizeBridgeResponse({ requestId: 'r1', ok: true, result: 42, logs: ['a'] }, 'r1');
  assert.deepEqual(normalized, { ok: true, result: 42, logs: ['a'] });
});

test('rejects request id mismatch and bridge errors', () => {
  assert.throws(() => normalizeBridgeResponse({ requestId: 'wrong', ok: true }, 'r1'), /requestId mismatch/);
  assert.throws(() => normalizeBridgeResponse({ requestId: 'r1', ok: false, error: { message: 'boom' } }, 'r1'), /boom/);
});
```

- [ ] **Step 2: Run test to verify it fails**

```bash
node --test /Users/lishuo121/.opencli/lib/opencli-vm/envelope.test.mjs
```

Expected: fails because `envelope.mjs` does not exist.

- [ ] **Step 3: Implement envelopes**

```js
import { randomUUID } from 'node:crypto';

export function createRequest(type, payload = {}, timeoutMs = 5000) {
  return {
    requestId: `opencli-vm-${randomUUID()}`,
    type,
    payload,
    timeoutMs: Number(timeoutMs || 5000),
  };
}

export function normalizeBridgeResponse(response, expectedRequestId) {
  if (!response || typeof response !== 'object') throw new Error('empty bridge response');
  if (response.requestId !== expectedRequestId) {
    throw new Error(`response requestId mismatch: expected ${expectedRequestId}, got ${response.requestId || 'missing'}`);
  }
  if (!response.ok) {
    throw new Error(String(response.error?.message || response.error || 'bridge command failed'));
  }
  return {
    ok: true,
    result: response.result ?? null,
    logs: Array.isArray(response.logs) ? response.logs : [],
  };
}
```

- [ ] **Step 4: Run test to verify it passes**

```bash
node --test /Users/lishuo121/.opencli/lib/opencli-vm/envelope.test.mjs
```

Expected: all envelope tests pass.

## Task 3: Bridge Source Generator

**Files:**
- Create: `/Users/lishuo121/.opencli/lib/opencli-vm/bridge-source.mjs`

- [ ] **Step 1: Implement bridge source**

```js
export function buildBridgeUserscript() {
  return `// ==UserScript==
// @name         OpenCLI VM Bridge
// @namespace    opencli.vm
// @version      0.1.0
// @description  Bridge that lets explicit local OpenCLI commands request page script execution.
// @match        *://*/*
// @run-at       document-start
// @grant        GM_getValue
// @grant        GM_setValue
// @grant        GM_deleteValue
// @grant        GM_listValues
// @grant        GM_xmlhttpRequest
// ==/UserScript==
(function () {
  'use strict';

  const REQUEST_EVENT = 'opencli-vm-request';
  const RESPONSE_EVENT = 'opencli-vm-response';

  function serializeError(error) {
    return {
      message: String(error && error.message ? error.message : error),
      name: String(error && error.name ? error.name : 'Error'),
      stack: String(error && error.stack ? error.stack : ''),
    };
  }

  function createApi(scriptName) {
    const prefix = 'opencli-vm:' + scriptName + ':';
    return {
      GM_getValue: (key, fallback = undefined) => GM_getValue(prefix + key, fallback),
      GM_setValue: (key, value) => GM_setValue(prefix + key, value),
      GM_deleteValue: (key) => GM_deleteValue(prefix + key),
      GM_listValues: () => GM_listValues().filter((key) => key.startsWith(prefix)).map((key) => key.slice(prefix.length)),
      GM_xmlhttpRequest: (details) => new Promise((resolve, reject) => {
        GM_xmlhttpRequest({
          ...details,
          onload: (response) => resolve({
            status: response.status,
            statusText: response.statusText,
            responseText: response.responseText,
            responseHeaders: response.responseHeaders,
            finalUrl: response.finalUrl,
          }),
          onerror: (error) => reject(error),
          ontimeout: () => reject(new Error('GM_xmlhttpRequest timed out')),
        });
      }),
    };
  }

  async function runSource(source, args, scriptName) {
    const api = createApi(scriptName || 'anonymous-opencli-script');
    const fn = new Function(
      'args',
      'GM_getValue',
      'GM_setValue',
      'GM_deleteValue',
      'GM_listValues',
      'GM_xmlhttpRequest',
      '"use strict"; return (async () => { ' + source + '\\n })();'
    );
    return await fn(args, api.GM_getValue, api.GM_setValue, api.GM_deleteValue, api.GM_listValues, api.GM_xmlhttpRequest);
  }

  async function handle(request) {
    if (!request || typeof request !== 'object') throw new Error('invalid request');
    if (!request.requestId) throw new Error('missing requestId');
    const payload = request.payload || {};
    if (request.type === 'ping') return { bridge: 'opencli-vm', version: '0.1.0', url: location.href };
    if (request.type === 'eval') return await runSource(String(payload.source || ''), payload.args || null, 'eval');
    if (request.type === 'inject') return await runSource(String(payload.source || ''), payload.args || null, payload.name || 'anonymous-opencli-script');
    if (request.type === 'call') {
      const fn = window[payload.functionName];
      if (typeof fn !== 'function') throw new Error('function not found: ' + payload.functionName);
      return await fn(payload.args);
    }
    throw new Error('unsupported request type: ' + request.type);
  }

  window.addEventListener(REQUEST_EVENT, async (event) => {
    const request = event.detail;
    try {
      const result = await Promise.race([
        handle(request),
        new Promise((_, reject) => setTimeout(() => reject(new Error('bridge command timed out')), Number(request && request.timeoutMs || 5000))),
      ]);
      window.dispatchEvent(new CustomEvent(RESPONSE_EVENT, { detail: { requestId: request.requestId, ok: true, result, logs: [] } }));
    } catch (error) {
      window.dispatchEvent(new CustomEvent(RESPONSE_EVENT, { detail: { requestId: request && request.requestId, ok: false, error: serializeError(error), logs: [] } }));
    }
  });

  window.__OPENCLI_VM_BRIDGE__ = { version: '0.1.0' };
})();`;
}
```

- [ ] **Step 2: Smoke-test the generator**

```bash
node --input-type=module -e "import { buildBridgeUserscript } from '/Users/lishuo121/.opencli/lib/opencli-vm/bridge-source.mjs'; const s = buildBridgeUserscript(); if (!s.includes('OpenCLI VM Bridge')) throw new Error('missing bridge name'); console.log(s.length > 1000)"
```

Expected: prints `true`.

## Task 4: Browser Session Dispatcher

**Files:**
- Create: `/Users/lishuo121/.opencli/lib/opencli-vm/dispatch.mjs`

- [ ] **Step 1: Implement dispatch through `opencli browser <session> eval`**

```js
import { execFile } from 'node:child_process';
import { promisify } from 'node:util';

import { createRequest, normalizeBridgeResponse } from './envelope.mjs';

const execFileAsync = promisify(execFile);

export async function dispatchToBridge({ session, type, payload, timeoutMs }) {
  if (!session) throw new Error('--session is required');
  const request = createRequest(type, payload, timeoutMs);
  const dispatcher = `(async () => {
    const request = ${JSON.stringify(request)};
    return await new Promise((resolve) => {
      const timeout = setTimeout(() => {
        window.removeEventListener('opencli-vm-response', onResponse);
        resolve({ requestId: request.requestId, ok: false, error: { message: 'OpenCLI VM Bridge is not installed or did not respond.' } });
      }, request.timeoutMs + 250);
      function onResponse(event) {
        const detail = event.detail || {};
        if (detail.requestId !== request.requestId) return;
        clearTimeout(timeout);
        window.removeEventListener('opencli-vm-response', onResponse);
        resolve(detail);
      }
      window.addEventListener('opencli-vm-response', onResponse);
      window.dispatchEvent(new CustomEvent('opencli-vm-request', { detail: request }));
    });
  })()`;
  const { stdout } = await execFileAsync('opencli', ['browser', session, 'eval', dispatcher], {
    encoding: 'utf8',
    maxBuffer: 1024 * 1024 * 10,
  });
  const parsed = parseOpencliEvalOutput(stdout);
  return normalizeBridgeResponse(parsed, request.requestId);
}

function parseOpencliEvalOutput(stdout) {
  const text = String(stdout || '').trim();
  const firstJson = text.indexOf('{');
  const lastJson = text.lastIndexOf('}');
  if (firstJson === -1 || lastJson === -1 || lastJson < firstJson) {
    throw new Error(`Could not parse opencli browser eval output: ${text}`);
  }
  return JSON.parse(text.slice(firstJson, lastJson + 1));
}
```

- [ ] **Step 2: Check syntax**

```bash
node --check /Users/lishuo121/.opencli/lib/opencli-vm/dispatch.mjs
```

Expected: no output and exit code 0.

## Task 5: External CLI Binary

**Files:**
- Create: `/Users/lishuo121/.opencli/bin/opencli-vm`

- [ ] **Step 1: Implement the executable CLI**

```js
#!/usr/bin/env node
import { mkdir, readFile, writeFile } from 'node:fs/promises';
import { dirname } from 'node:path';

import { buildBridgeUserscript } from '../lib/opencli-vm/bridge-source.mjs';
import { dispatchToBridge } from '../lib/opencli-vm/dispatch.mjs';
import { parseUserscriptMetadata, matchesUserscriptUrl } from '../lib/opencli-vm/metadata.mjs';

const BRIDGE_PATH = '/Users/lishuo121/.opencli/sites/vm/opencli-vm-bridge.user.js';
const SUPPORTED_GRANTS = new Set(['GM_getValue', 'GM_setValue', 'GM_deleteValue', 'GM_listValues', 'GM_xmlhttpRequest', 'none']);

main().catch((error) => {
  console.error(JSON.stringify({ ok: false, error: { message: error.message } }, null, 2));
  process.exitCode = 1;
});

async function main() {
  const argv = process.argv.slice(2);
  const command = argv.shift();
  if (!command || command === '--help' || command === '-h') return printHelp();
  if (command === 'bridge-url') return bridgeUrl();
  if (command === 'check') return check(parseArgs(argv));
  if (command === 'eval') return evalJs(parseArgs(argv, { positionalName: 'javascript' }));
  if (command === 'inject') return inject(parseArgs(argv));
  if (command === 'call') return call(parseArgs(argv));
  throw new Error(`Unknown command: ${command}`);
}

function printHelp() {
  console.log(`Usage:
  opencli vm bridge-url
  opencli vm check --session <name>
  opencli vm eval --session <name> <javascript>
  opencli vm inject --session <name> --file <path>
  opencli vm call --session <name> --function <name> --json <payload>`);
}

function parseArgs(argv, options = {}) {
  const args = { _: [] };
  for (let index = 0; index < argv.length; index += 1) {
    const token = argv[index];
    if (token.startsWith('--')) {
      const key = token.slice(2);
      const next = argv[index + 1];
      if (next && !next.startsWith('--')) {
        args[key] = next;
        index += 1;
      } else {
        args[key] = true;
      }
    } else {
      args._.push(token);
    }
  }
  if (options.positionalName && args._.length) args[options.positionalName] = args._.join(' ');
  return args;
}

async function bridgeUrl() {
  const source = buildBridgeUserscript();
  await mkdir(dirname(BRIDGE_PATH), { recursive: true });
  await writeFile(BRIDGE_PATH, source, 'utf8');
  printJson({ ok: true, path: BRIDGE_PATH, installUrl: `file://${BRIDGE_PATH}` });
}

async function check(args) {
  const response = await dispatchToBridge({ session: args.session, type: 'ping', payload: {}, timeoutMs: Number(args['timeout-ms'] || 2000) });
  printJson({ ok: true, ...response.result });
}

async function evalJs(args) {
  if (!args.javascript) throw new Error('javascript is required');
  const response = await dispatchToBridge({
    session: args.session,
    type: 'eval',
    payload: { source: args.javascript },
    timeoutMs: Number(args['timeout-ms'] || 5000),
  });
  printJson({ ok: true, result: response.result, logs: response.logs });
}

async function inject(args) {
  if (!args.file) throw new Error('--file is required');
  const source = await readFile(args.file, 'utf8');
  const metadata = parseUserscriptMetadata(source);
  const unsupported = metadata.grant.filter((grant) => !SUPPORTED_GRANTS.has(grant));
  if (unsupported.length) throw new Error(`Unsupported @grant: ${unsupported.join(', ')}`);
  const current = await dispatchToBridge({ session: args.session, type: 'eval', payload: { source: 'return location.href;' }, timeoutMs: 2000 });
  if (!matchesUserscriptUrl(metadata, String(current.result))) {
    throw new Error(`Current page does not match script metadata: ${current.result}`);
  }
  const response = await dispatchToBridge({
    session: args.session,
    type: 'inject',
    payload: { name: metadata.name, source: metadata.body, grants: metadata.grant },
    timeoutMs: Number(args['timeout-ms'] || 5000),
  });
  printJson({ ok: true, name: metadata.name, matched: true, result: response.result, logs: response.logs });
}

async function call(args) {
  if (!args.function) throw new Error('--function is required');
  let payload = null;
  try {
    payload = JSON.parse(args.json || 'null');
  } catch {
    throw new Error('--json must be valid JSON');
  }
  const response = await dispatchToBridge({
    session: args.session,
    type: 'call',
    payload: { functionName: args.function, args: payload },
    timeoutMs: Number(args['timeout-ms'] || 5000),
  });
  printJson({ ok: true, function: args.function, result: response.result, logs: response.logs });
}

function printJson(value) {
  console.log(JSON.stringify(value, null, 2));
}
```

- [ ] **Step 2: Make the binary executable**

```bash
chmod +x /Users/lishuo121/.opencli/bin/opencli-vm
```

Expected: `ls -l /Users/lishuo121/.opencli/bin/opencli-vm` shows executable bits.

- [ ] **Step 3: Check syntax**

```bash
node --check /Users/lishuo121/.opencli/bin/opencli-vm
```

Expected: no output and exit code 0.

## Task 6: Register `opencli vm`

**Files:**
- Modify: `/Users/lishuo121/.opencli/external-clis.yaml`

- [ ] **Step 1: Register the external CLI**

```bash
opencli external register vm --binary /Users/lishuo121/.opencli/bin/opencli-vm --desc "OpenCLI Violentmonkey bridge for named browser sessions"
```

Expected: command succeeds.

- [ ] **Step 2: Confirm registration**

```bash
opencli external list | rg -n "name: vm|opencli-vm|Violentmonkey"
```

Expected: output includes `name: vm`, the binary path, and the description.

- [ ] **Step 3: Confirm command help**

```bash
opencli vm --help
```

Expected: help text includes `opencli vm check --session <name>` and `opencli vm inject --session <name> --file <path>`.

## Task 7: Local Verification

**Files:**
- Use existing files from Tasks 1-6.

- [ ] **Step 1: Run unit tests**

```bash
node --test /Users/lishuo121/.opencli/lib/opencli-vm/metadata.test.mjs /Users/lishuo121/.opencli/lib/opencli-vm/envelope.test.mjs
```

Expected: all tests pass.

- [ ] **Step 2: Generate bridge userscript**

```bash
opencli vm bridge-url
```

Expected: JSON output includes:

```json
{
  "ok": true,
  "path": "/Users/lishuo121/.opencli/sites/vm/opencli-vm-bridge.user.js",
  "installUrl": "file:///Users/lishuo121/.opencli/sites/vm/opencli-vm-bridge.user.js"
}
```

- [ ] **Step 3: Verify the generated bridge file exists**

```bash
test -s /Users/lishuo121/.opencli/sites/vm/opencli-vm-bridge.user.js
```

Expected: exit code 0.

## Task 8: Browser Smoke Verification

**Files:**
- Create temporary fixture: `/private/tmp/opencli-vm-smoke.user.js`

- [ ] **Step 1: Ensure OpenCLI browser bridge is healthy**

```bash
opencli doctor -v
```

Expected: daemon, extension, and connectivity are healthy. If it still reports the known extension mismatch `v1.0.10 -> v1.0.15`, update the OpenCLI Chrome extension first.

- [ ] **Step 2: Install the Violentmonkey bridge**

```bash
opencli vm bridge-url
```

Open the returned `installUrl` in Chrome and install the script in Violentmonkey.

- [ ] **Step 3: Open a named browser session**

```bash
opencli browser vmtest open https://example.com --window foreground
```

Expected: Chrome opens `https://example.com` for the `vmtest` session.

- [ ] **Step 4: Check the bridge**

```bash
opencli vm check --session vmtest
```

Expected: JSON output has `"ok": true`, `"bridge": "opencli-vm"`, and `"url": "https://example.com/"`.

- [ ] **Step 5: Evaluate a snippet**

```bash
opencli vm eval --session vmtest "return document.title;"
```

Expected: JSON output has `"result": "Example Domain"`.

- [ ] **Step 6: Inject a userscript**

Create `/private/tmp/opencli-vm-smoke.user.js`:

```js
// ==UserScript==
// @name         OpenCLI VM Smoke
// @match        https://example.com/*
// @grant        GM_setValue
// @grant        GM_getValue
// ==/UserScript==
GM_setValue('lastTitle', document.title);
return { title: document.title, stored: GM_getValue('lastTitle') };
```

Run:

```bash
opencli vm inject --session vmtest --file /private/tmp/opencli-vm-smoke.user.js
```

Expected: JSON output contains `"title": "Example Domain"` and `"stored": "Example Domain"`.

## Task 9: Repository Documentation Commit

**Files:**
- Modify: `/Users/lishuo121/Gitnova/docs/superpowers/plans/2026-05-27-opencli-violentmonkey-bridge-implementation.md`
- Modify: `/Users/lishuo121/Gitnova/docs/superpowers/specs/2026-05-27-opencli-violentmonkey-bridge-design.md` only if the implementation changes the agreed design.

- [ ] **Step 1: Commit the plan file**

```bash
git -C /Users/lishuo121/Gitnova add docs/superpowers/plans/2026-05-27-opencli-violentmonkey-bridge-implementation.md
git -C /Users/lishuo121/Gitnova commit -m "Plan OpenCLI Violentmonkey bridge implementation"
```

Expected: repository commit contains only the plan unless the design spec was intentionally updated.

- [ ] **Step 2: Report local-only files**

Final response should include:

```text
Local OpenCLI files created under /Users/lishuo121/.opencli/bin and /Users/lishuo121/.opencli/lib/opencli-vm.
Bridge userscript generated under /Users/lishuo121/.opencli/sites/vm.
Browser smoke verification: <passed or blocked with exact reason>.
```

## Self-Review

- Spec coverage: The plan covers bridge generation, `check`, `eval`, `inject`, `call`, userscript metadata matching, unsupported grant failure, unit tests, OpenCLI command exposure, and real browser smoke verification.
- Red-flag scan: The plan does not leave undefined implementation steps or unfinished sections.
- Type consistency: Commands use `opencli vm bridge-url`, `opencli vm check`, `opencli vm eval`, `opencli vm inject`, and `opencli vm call`; shared module names match all imports.
