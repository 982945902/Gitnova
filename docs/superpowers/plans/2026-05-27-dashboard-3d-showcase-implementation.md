# Dashboard 3D Showcase Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add Analyze and Showcase 3D graph modes to the React + Three.js Gitnova dashboard.

**Architecture:** Keep the existing Rust API and React data flow. Refactor the Three.js graph layer so `GraphScene` receives a `mode`, uses deterministic 3D positions, switches camera/material/animation behavior per mode, and preserves node selection/context highlighting.

**Tech Stack:** React, TypeScript, Three.js, Vitest, existing Rust/Axum dashboard server.

---

### Task 1: Mode and 3D Layout Tests

**Files:**
- Create: `crates/gitnova-dashboard/web/src/graph/modes.ts`
- Create: `crates/gitnova-dashboard/web/src/graph/modes.test.ts`
- Modify: `crates/gitnova-dashboard/web/src/graph/layout.test.ts`
- Modify: `crates/gitnova-dashboard/web/src/graph/layout.ts`

- [ ] **Step 1: Write failing tests**

Add `modes.test.ts`:

```ts
import { describe, expect, it } from "vitest";
import { GRAPH_MODES, modeConfig } from "./modes";

describe("graph modes", () => {
  it("defines analyze and showcase modes", () => {
    expect(GRAPH_MODES).toEqual(["analyze", "showcase"]);
    expect(modeConfig.analyze.autoRotate).toBe(false);
    expect(modeConfig.showcase.autoRotate).toBe(true);
  });
});
```

Extend `layout.test.ts` with:

```ts
it("adds visible z-depth for multi-node graphs", () => {
  const positions = layoutGraph([node("a", 1, 1), node("b", 1, 0), node("c", 4, 4)]);
  const depths = [...positions.values()].map((point) => point.z);
  expect(new Set(depths).size).toBeGreaterThan(1);
  expect(Math.max(...depths) - Math.min(...depths)).toBeGreaterThan(20);
});
```

- [ ] **Step 2: Run tests to verify failure**

Run:

```bash
cd crates/gitnova-dashboard/web
npm test
```

Expected: failure because `modes.ts` is missing and layout z-depth is too small.

- [ ] **Step 3: Implement mode constants and deeper layout**

Create `modes.ts` with:

```ts
export const GRAPH_MODES = ["analyze", "showcase"] as const;
export type GraphMode = (typeof GRAPH_MODES)[number];

export const modeConfig = {
  analyze: { autoRotate: false, particles: false, depthScale: 1 },
  showcase: { autoRotate: true, particles: true, depthScale: 1.45 },
} satisfies Record<GraphMode, { autoRotate: boolean; particles: boolean; depthScale: number }>;
```

Update `layoutGraph` so `z` varies by path/kind/index with a range greater than 20 for multi-node graphs.

- [ ] **Step 4: Run tests**

Run:

```bash
cd crates/gitnova-dashboard/web
npm test
```

Expected: pass.

### Task 2: 3D Scene Upgrade

**Files:**
- Modify: `crates/gitnova-dashboard/web/src/graph/GraphScene.tsx`
- Modify: `crates/gitnova-dashboard/web/src/graph/colors.ts`
- Modify: `crates/gitnova-dashboard/web/src/graph/layout.ts`

- [ ] **Step 1: Update GraphScene props**

Add:

```ts
mode: GraphMode;
resetSignal: number;
```

to `GraphSceneProps`.

- [ ] **Step 2: Replace orthographic camera with perspective camera**

Use:

```ts
const camera = new THREE.PerspectiveCamera(48, 1, 0.1, 4000);
camera.position.set(0, 0, 860);
camera.lookAt(0, 0, 0);
```

Resize should update `camera.aspect`.

- [ ] **Step 3: Add group rotation controls**

Drag rotates `meshGroup.rotation.y` and `meshGroup.rotation.x`. Wheel moves camera along z while clamping between 260 and 1600.

- [ ] **Step 4: Add showcase animation**

Use `requestAnimationFrame`. In showcase mode, slowly auto-rotate the graph and pulse nodes/rings. In analyze mode, render on state/interaction changes without constant auto-rotation except for the shared animation frame required by scene updates.

- [ ] **Step 5: Add particles in showcase mode**

Create a `THREE.Points` starfield only when mode is `showcase`.

- [ ] **Step 6: Preserve click selection**

Raycast against node meshes after group rotation. Clicking a node still calls `onSelectNode`.

### Task 3: App Mode Controls

**Files:**
- Modify: `crates/gitnova-dashboard/web/src/App.tsx`
- Modify: `crates/gitnova-dashboard/web/src/styles.css`

- [ ] **Step 1: Add graph mode state**

Add:

```ts
const [graphMode, setGraphMode] = useState<GraphMode>("analyze");
const [graphResetSignal, setGraphResetSignal] = useState(0);
```

- [ ] **Step 2: Add toolbar controls**

Add Analyze/Showcase segmented buttons and a reset camera button in the graph toolbar.

- [ ] **Step 3: Pass mode to GraphScene**

Pass `mode={graphMode}` and `resetSignal={graphResetSignal}`.

- [ ] **Step 4: Add styles**

Style segmented controls, active mode state, and a showcase status treatment.

### Task 4: Verification

**Files:**
- Generated: `crates/gitnova-dashboard/web/dist/*`

- [ ] **Step 1: Run frontend tests**

Run:

```bash
cd crates/gitnova-dashboard/web
npm test
```

Expected: pass.

- [ ] **Step 2: Build frontend**

Run:

```bash
cd crates/gitnova-dashboard/web
npm run build
```

Expected: pass and refresh `dist`.

- [ ] **Step 3: Run focused dashboard tests**

Run:

```bash
cargo test -p gitnova --test full_challenge dashboard_tests::dashboard_serves
```

Expected: 4 dashboard tests pass.

- [ ] **Step 4: Browser verification**

Launch dashboard against a fixture and verify:

- one canvas renders
- Summary shows real node/edge counts
- Analyze mode is active by default
- Showcase mode button changes active state
- search still ranks `formatDate` first for `formatDate utility`
- clicking the graph canvas keeps node detail populated

---

## Self-Review

- Spec coverage: dual modes, perspective 3D, auto-rotation, particles, selection preservation, tests, and browser verification are covered.
- Placeholder scan: no TBD/TODO placeholders.
- Type consistency: `GraphMode`, `modeConfig`, `mode`, and `resetSignal` are consistent across tasks.
