# Dashboard 3D Showcase Graph Design

## Goal

Upgrade the dashboard graph from a mostly flat WebGL node map into a true 3D graph experience with two modes:

- **Analyze mode:** practical 3D graph exploration for understanding code relationships.
- **Showcase mode:** cinematic 3D presentation for demos, screenshots, and recordings.

The feature should build on the existing React + TypeScript + Three.js dashboard and should not require backend API changes.

## Chosen Approach

Add a graph display mode switch in the dashboard toolbar:

- `Analyze`
- `Showcase`

Both modes use the same graph data, selected node state, highlighted context, and `/api/*` workflows. The difference is camera behavior, layout depth, materials, animation, and supporting visual layers.

## Analyze Mode

Analyze mode is the default because the dashboard is still an engineering tool.

Behavior:

- Use a real perspective camera instead of the current orthographic camera.
- Lay nodes out in 3D space with deterministic x/y/z positions.
- Group nearby code areas by path prefix and node kind.
- Keep selected and highlighted nodes readable and stable.
- Support drag rotate, wheel zoom, and click-to-select.
- Keep labels/metadata in side panels rather than covering the 3D viewport.

Visual treatment:

- Dark canvas background.
- Color by node kind.
- Larger nodes for higher degree.
- Brighter edges for highlighted context.
- Muted non-context edges to reduce visual noise.

## Showcase Mode

Showcase mode is optimized for visual impact.

Behavior:

- Add slow auto-rotation when the user is not dragging.
- Add subtle floating/pulse animation to nodes.
- Emphasize highlighted context with glowing rings and brighter edges.
- Move non-highlighted nodes slightly deeper and dimmer so context feels spatial.
- Add a camera reset button that returns to a composed view.

Visual treatment:

- Add a starfield or lightweight particle background using Three.js points.
- Use emissive-looking node materials where practical.
- Use additive blended glow rings for selected/highlighted nodes.
- Use curved or slightly lifted edge lines for depth.

## Component Design

Refactor the graph code into focused files:

```text
crates/gitnova-dashboard/web/src/graph/
  GraphScene.tsx
  camera.ts
  colors.ts
  layout.ts
  materials.ts
  sceneObjects.ts
  modes.ts
```

Responsibilities:

- `GraphScene.tsx`: React boundary, event listeners, renderer lifecycle.
- `modes.ts`: `GraphMode` type and mode-specific constants.
- `camera.ts`: perspective camera creation, resize, zoom, reset.
- `layout.ts`: deterministic 3D layout.
- `materials.ts`: materials for nodes, edges, rings, and particles.
- `sceneObjects.ts`: create and dispose Three.js meshes/lines/particles.

## Interaction Model

Mouse and trackpad:

- Drag: rotate graph around center.
- Wheel: zoom in/out.
- Click node: select node and load graph context.
- Double click or reset button: restore camera and rotation.

Mode switch:

- Analyze mode starts with a stable camera and no auto-rotation.
- Showcase mode starts auto-rotation and particles.
- Switching mode should preserve selected node and highlighted context.

## Data Flow

No backend changes are required.

The existing React app continues to:

1. Load nodes and edges.
2. Filter visible graph nodes.
3. Fetch rank/answer/context.
4. Pass `nodes`, `edges`, highlighted ids, selected id, and `mode` into `GraphScene`.

`GraphScene` derives:

- 3D positions from `layoutGraph`.
- Materials from current mode and highlight state.
- Camera behavior from current mode.

## Performance Constraints

Keep the first 3D showcase version simple and robust:

- Limit visible nodes using the existing visible node limit.
- Rebuild scene objects only when graph data, mode, or highlight state changes.
- Use `requestAnimationFrame` only while the graph is visible.
- Dispose geometries/materials on unmount.
- Avoid postprocessing in the first version unless the base scene is already stable.

## Testing

Add focused tests for deterministic non-flat layout:

- `layoutGraph` returns finite x/y/z positions.
- At least one visible graph with multiple nodes has non-zero z-depth.
- Mode constants exist for `analyze` and `showcase`.

Use browser verification for visual behavior:

- Dashboard renders one canvas.
- Canvas is visually nonblank.
- Mode switch changes toolbar state.
- Search still updates ranked results.
- Clicking a node still updates detail.

## Non-Goals

- Do not change `/api/*` responses.
- Do not add server-side graph layout computation.
- Do not add heavyweight postprocessing or physics engines in the first pass.
- Do not make the graph unreadable for the sake of effects.

## Acceptance Criteria

- Dashboard graph uses a perspective 3D scene.
- UI exposes Analyze and Showcase modes.
- Showcase mode has visible auto-rotation and particle/depth styling.
- Analyze mode remains practical for selecting nodes and inspecting context.
- Existing search, evidence, ranked results, and node detail workflows continue to work.
- Frontend tests and dashboard focused tests pass.
- Browser verification confirms a nonblank 3D canvas and working mode switch.
