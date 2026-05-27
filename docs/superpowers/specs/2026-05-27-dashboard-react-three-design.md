# Dashboard React + Three.js Design

## Goal

Replace the current hand-written dashboard frontend with a TypeScript React app and a Three.js WebGL graph viewport while preserving the existing Rust dashboard API surface.

The dashboard should still be launched from the Rust CLI with:

```bash
cargo run -p gitnova -- dashboard --repo <path> --port 4567
```

Development should use a normal frontend workflow, but the packaged dashboard should remain served by `gitnova-dashboard`.

## Chosen Approach

Use Vite + React + TypeScript + Three.js inside `crates/gitnova-dashboard/web`.

Rust remains responsible for:

- Serving the dashboard shell and static build assets.
- Serving the existing `/api/*` routes.
- Loading graph data from Gitnova storage.

React is responsible for:

- Query input and filters.
- Summary, ranked results, answer, evidence, and node detail UI.
- Data fetching from `/api/*`.
- Client-side graph selection/highlighting state.

Three.js is responsible for:

- Rendering nodes and edges in a WebGL scene.
- Camera pan/zoom controls.
- Raycast-based node selection.
- Highlighting selected nodes, context nodes, and context edges.

## Frontend Structure

Create this source layout:

```text
crates/gitnova-dashboard/web/
  package.json
  tsconfig.json
  vite.config.ts
  index.html
  src/
    main.tsx
    App.tsx
    api.ts
    types.ts
    graph/
      GraphScene.tsx
      layout.ts
      colors.ts
    components/
      SearchPanel.tsx
      SummaryPanel.tsx
      ResultsPanel.tsx
      DetailPanel.tsx
    styles.css
```

The implementation can keep the first pass dependency-light: direct `three` and React hooks are enough. React Three Fiber is not required for this migration because the graph renderer benefits from explicit control over scene objects, buffers, and raycasting.

## UI Layout

The first screen uses a dense operational layout:

- Left rail: query input, node filter, kind filter, summary, hubs.
- Center: full-height Three.js graph viewport.
- Right rail: answer, evidence, ranked results, selected node detail.

This keeps the graph as the primary object and avoids a marketing-style landing page. The graph viewport should be visually dominant and usable immediately after page load.

## Data Flow

Initial load:

1. Fetch `/api/summary`.
2. Fetch `/api/nodes`.
3. Fetch `/api/edges`.
4. Build a filtered graph model in React state.
5. Pass graph nodes, edges, selection, and highlight state into `GraphScene`.

Search flow:

1. User submits query.
2. Fetch `/api/rank?query=...&limit=10`.
3. Fetch `/api/answer?query=...&depth=1&limit=40`.
4. Update answer/evidence/results.
5. If the answer contains a context target, highlight that context in the graph.

Selection flow:

1. User clicks a rendered node in Three.js.
2. `GraphScene` emits the node id.
3. React fetches `/api/graph-context?node_id=...&depth=1&limit=40`.
4. Right rail updates detail and the graph highlights context nodes/edges.

## Rust Serving Strategy

During development, Vite can run separately and proxy API requests to the Rust dashboard port.

For the Rust-served dashboard, use built assets from `crates/gitnova-dashboard/web/dist`. The Rust crate should serve:

- `/` -> `dist/index.html` if present.
- `/assets/*` -> files from `dist/assets`.
- Existing `/api/*` routes unchanged.

Keep the current embedded plain HTML/CSS/JS assets as a fallback only if needed during migration. The final steady state should prefer the Vite build output.

## Error Handling

The React app should show compact inline error states for:

- Graph load failure.
- Search/rank failure.
- Answer/context failure.
- Empty graph or empty search results.

The graph viewport should still render a non-crashing empty state if nodes or edges are missing.

## Testing

Add focused tests at three levels:

- Rust dashboard tests should keep verifying `/api/*` responses and add a smoke check that `/` serves an HTML shell.
- Frontend unit tests should cover API URL construction and graph filtering/layout helpers.
- Browser verification should confirm the app renders, the Three.js canvas is nonblank, search updates results, and clicking a node updates detail.

Because the current dashboard tests can fail under sandboxed local port permissions, browser verification may require an approved local server run.

## Non-Goals

- Do not redesign the backend graph API in this pass.
- Do not introduce authentication, multi-repo sessions, or remote deployment.
- Do not build a full 3D physics engine. A deterministic client layout is enough for the first React migration.
- Do not migrate the rest of the Rust workspace to a JS build system.

## Acceptance Criteria

- `cargo run -p gitnova -- dashboard --repo <fixture-or-repo> --port <port>` serves the React dashboard.
- The dashboard is authored in TypeScript React.
- The central graph is rendered with Three.js/WebGL.
- Existing summary, rank, answer, graph context, and node detail workflows remain available.
- The UI handles loading and error states without crashing.
- Verification includes a Rust test/build command and a browser visual check of the rendered graph.
