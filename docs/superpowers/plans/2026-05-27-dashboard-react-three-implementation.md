# Dashboard React Three Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Migrate Gitnova Dashboard to a TypeScript React frontend with a Three.js WebGL graph viewport while preserving the Rust dashboard API.

**Architecture:** Add a Vite app under `crates/gitnova-dashboard/web`; React owns UI and state, Three.js owns graph rendering, and Axum serves `/api/*` plus built dashboard assets. Keep legacy embedded assets as fallback so `cargo run -p gitnova -- dashboard` still works before frontend assets are built.

**Tech Stack:** Rust/Axum, Vite, React, TypeScript, Three.js, Vitest.

---

### Task 1: Asset Serving Contract

**Files:**
- Modify: `tests/integration/dashboard_tests.rs`
- Modify: `crates/gitnova-dashboard/src/server.rs`

- [ ] **Step 1: Write the failing test**

Update `dashboard_serves_graph_visualization_shell` so it expects a React/Vite shell and bundled asset reference instead of the legacy handwritten DOM ids:

```rust
let html = html.expect("dashboard index should respond");
+assert!(html.contains("Gitnova Dashboard"));
+assert!(html.contains("type=\"module\""));
+assert!(html.contains("/assets/"));
+assert!(html.contains("id=\"root\""));
+
+let app_js = app_js.expect("dashboard app js should respond");
+assert!(app_js.contains("React") || app_js.contains("three") || app_js.contains("Gitnova"));
-assert!(html.contains("graph-canvas"));
-assert!(html.contains("graph-labels"));
-assert!(html.contains("Graph"));
-assert!(html.contains("graph-filter"));
-assert!(html.contains("graph-kind"));
-assert!(html.contains("graph-zoom-in"));
-
-let app_js = app_js.expect("dashboard app js should respond");
-assert!(app_js.contains("webgl"));
```

- [ ] **Step 2: Run the test to verify it fails**

Run:

```bash
cargo test -p gitnova --test full_challenge dashboard_tests::dashboard_serves_graph_visualization_shell
```

Expected: failure because the current HTML is the legacy non-React shell.

- [ ] **Step 3: Implement minimal asset-serving support**

In `crates/gitnova-dashboard/src/server.rs`, add helpers that serve built Vite assets from `web/dist` when present, and fallback to legacy `include_str!` assets when absent:

```rust
use axum::extract::Path;
use axum::http::{header, HeaderValue, StatusCode};
use axum::response::{Html, IntoResponse, Response};
use std::fs;

const DIST_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/web/dist");

async fn index() -> impl IntoResponse {
    if let Some(html) = read_dist_file("index.html") {
        return Html(html);
    }
    Html(include_str!("../assets/index.html").to_string())
}

async fn asset(Path(path): Path<String>) -> Response {
    if let Some(bytes) = read_dist_bytes(&format!("assets/{path}")) {
        return with_content_type(bytes, content_type_for(&path));
    }
    match path.as_str() {
        "app.js" => (
            [(header::CONTENT_TYPE, "application/javascript; charset=utf-8")],
            include_str!("../assets/app.js").as_bytes().to_vec(),
        )
            .into_response(),
        "styles.css" => (
            [(header::CONTENT_TYPE, "text/css; charset=utf-8")],
            include_str!("../assets/styles.css").as_bytes().to_vec(),
        )
            .into_response(),
        _ => StatusCode::NOT_FOUND.into_response(),
    }
}

fn read_dist_file(path: &str) -> Option<String> {
    fs::read_to_string(format!("{DIST_DIR}/{path}")).ok()
}

fn read_dist_bytes(path: &str) -> Option<Vec<u8>> {
    fs::read(format!("{DIST_DIR}/{path}")).ok()
}

fn with_content_type(bytes: Vec<u8>, content_type: &'static str) -> Response {
    let mut response = bytes.into_response();
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static(content_type),
    );
    response
}

fn content_type_for(path: &str) -> &'static str {
    if path.ends_with(".js") {
        "application/javascript; charset=utf-8"
    } else if path.ends_with(".css") {
        "text/css; charset=utf-8"
    } else if path.ends_with(".svg") {
        "image/svg+xml"
    } else {
        "application/octet-stream"
    }
}
```

Change router asset routes to:

```rust
.route("/", get(index))
.route("/assets/{*path}", get(asset))
```

- [ ] **Step 4: Run the focused test**

Run:

```bash
cargo test -p gitnova --test full_challenge dashboard_tests::dashboard_serves_graph_visualization_shell
```

Expected: pass when local port binding is permitted.

### Task 2: React Frontend Scaffold

**Files:**
- Create: `crates/gitnova-dashboard/web/package.json`
- Create: `crates/gitnova-dashboard/web/tsconfig.json`
- Create: `crates/gitnova-dashboard/web/tsconfig.node.json`
- Create: `crates/gitnova-dashboard/web/vite.config.ts`
- Create: `crates/gitnova-dashboard/web/index.html`
- Create: `crates/gitnova-dashboard/web/src/main.tsx`
- Create: `crates/gitnova-dashboard/web/src/App.tsx`
- Create: `crates/gitnova-dashboard/web/src/types.ts`
- Create: `crates/gitnova-dashboard/web/src/api.ts`
- Create: `crates/gitnova-dashboard/web/src/styles.css`

- [ ] **Step 1: Write package and config files**

Create `package.json` with scripts:

```json
{
  "scripts": {
    "dev": "vite",
    "build": "tsc -b && vite build",
    "test": "vitest run"
  },
  "dependencies": {
    "@vitejs/plugin-react": "^5.0.0",
    "vite": "^7.0.0",
    "typescript": "^5.6.0",
    "react": "^19.0.0",
    "react-dom": "^19.0.0",
    "three": "^0.180.0",
    "lucide-react": "^0.468.0"
  },
  "devDependencies": {
    "@types/react": "^19.0.0",
    "@types/react-dom": "^19.0.0",
    "@types/three": "^0.180.0",
    "vitest": "^3.0.0",
    "jsdom": "^26.0.0"
  }
}
```

Configure Vite to output to `dist` and proxy `/api` to `http://127.0.0.1:4567` during dev.

- [ ] **Step 2: Install dependencies**

Run:

```bash
npm install
```

inside `crates/gitnova-dashboard/web`.

Expected: `package-lock.json` is created.

- [ ] **Step 3: Add minimal React shell**

Implement `index.html`, `main.tsx`, `App.tsx`, and `styles.css` so the app renders:

- left rail query controls
- center graph viewport placeholder
- right rail answer/detail panels

- [ ] **Step 4: Run frontend build**

Run:

```bash
npm run build
```

Expected: TypeScript and Vite build succeed and `dist/index.html` plus `dist/assets/*` exist.

### Task 3: Typed API and Frontend Tests

**Files:**
- Modify: `crates/gitnova-dashboard/web/src/types.ts`
- Modify: `crates/gitnova-dashboard/web/src/api.ts`
- Create: `crates/gitnova-dashboard/web/src/api.test.ts`

- [ ] **Step 1: Write failing API tests**

Create tests covering API URL construction:

```ts
import { describe, expect, it } from "vitest";
import { paths } from "./api";

describe("dashboard api paths", () => {
  it("encodes rank query", () => {
    expect(paths.rank("Where is auth session validated?", 10)).toBe(
      "/api/rank?query=Where%20is%20auth%20session%20validated%3F&limit=10",
    );
  });

  it("encodes graph context node id", () => {
    expect(paths.graphContext("gn_a/b:c", 1, 40)).toBe(
      "/api/graph-context?node_id=gn_a%2Fb%3Ac&depth=1&limit=40",
    );
  });
});
```

- [ ] **Step 2: Run tests to verify failure**

Run:

```bash
npm test
```

Expected: failure because `paths` is not implemented.

- [ ] **Step 3: Implement API helpers**

Add typed fetch helpers and exported `paths` object to `api.ts`.

- [ ] **Step 4: Run tests**

Run:

```bash
npm test
```

Expected: pass.

### Task 4: Three.js Graph Scene

**Files:**
- Create: `crates/gitnova-dashboard/web/src/graph/colors.ts`
- Create: `crates/gitnova-dashboard/web/src/graph/layout.ts`
- Create: `crates/gitnova-dashboard/web/src/graph/layout.test.ts`
- Create: `crates/gitnova-dashboard/web/src/graph/GraphScene.tsx`
- Modify: `crates/gitnova-dashboard/web/src/App.tsx`

- [ ] **Step 1: Write failing layout tests**

Create tests that prove layout generation is deterministic and finite:

```ts
import { describe, expect, it } from "vitest";
import { layoutGraph } from "./layout";
import type { CodeNode } from "../types";

const nodes: CodeNode[] = [
  { id: "a", kind: "function", name: "a", qualified_name: "a", path: "a.ts", span: null, language: "typescript", text: "", tags: [], metrics: { in_degree: 1, out_degree: 1, churn_90d: 0, last_changed_unix: null, is_test: false } },
  { id: "b", kind: "method", name: "b", qualified_name: "b", path: "b.ts", span: null, language: "typescript", text: "", tags: [], metrics: { in_degree: 1, out_degree: 0, churn_90d: 0, last_changed_unix: null, is_test: false } },
];

describe("layoutGraph", () => {
  it("returns deterministic finite positions", () => {
    const first = layoutGraph(nodes);
    const second = layoutGraph(nodes);
    expect(first).toEqual(second);
    expect(Number.isFinite(first.get("a")?.x)).toBe(true);
    expect(Number.isFinite(first.get("b")?.y)).toBe(true);
  });
});
```

- [ ] **Step 2: Run tests to verify failure**

Run:

```bash
npm test
```

Expected: failure because `layoutGraph` is not implemented.

- [ ] **Step 3: Implement deterministic layout and Three.js scene**

Implement `layoutGraph` as a deterministic radial layout with hub nodes closer to center. Implement `GraphScene` with:

- `Scene`, `PerspectiveCamera`, `WebGLRenderer`
- line geometry for edges
- sphere meshes for nodes
- raycaster click selection
- wheel zoom and pointer pan
- cleanup on unmount

- [ ] **Step 4: Wire scene into `App.tsx`**

Pass filtered nodes, edges, highlighted ids, and selected id into `GraphScene`.

- [ ] **Step 5: Run tests and build**

Run:

```bash
npm test
npm run build
```

Expected: pass.

### Task 5: Rust + Frontend Integration Verification

**Files:**
- Modify: `crates/gitnova-dashboard/src/server.rs`
- Modify: `tests/integration/dashboard_tests.rs`

- [ ] **Step 1: Re-run Rust focused dashboard test**

Run:

```bash
cargo test -p gitnova --test full_challenge dashboard_tests::dashboard_serves_graph_visualization_shell
```

Expected: pass when local port binding is permitted.

- [ ] **Step 2: Run broader compile check**

Run:

```bash
cargo test -p gitnova-dashboard
```

Expected: pass.

- [ ] **Step 3: Launch dashboard for browser verification**

Index a fixture and serve dashboard:

```bash
cargo run -p gitnova -- index tests/fixtures/ts_sample --force
cargo run -p gitnova -- dashboard --repo tests/fixtures/ts_sample --port 4567
```

Expected: server runs at `http://127.0.0.1:4567`.

- [ ] **Step 4: Verify in browser**

Open `http://127.0.0.1:4567` and confirm:

- React app shell renders.
- Three.js canvas is nonblank.
- Search returns ranked results.
- Clicking a graph node updates the detail panel.

---

## Self-Review

- Spec coverage: frontend scaffold, React/TS, Three.js graph, Rust serving, error states, tests, and browser verification are covered.
- Placeholder scan: no TBD/TODO placeholders remain.
- Type consistency: API path names, graph type names, and file paths are consistent across tasks.
