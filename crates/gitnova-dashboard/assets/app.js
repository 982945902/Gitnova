async function json(path) {
  const response = await fetch(path);
  return response.json();
}

function text(value) {
  return JSON.stringify(value, null, 2);
}

const graphState = {
  allNodes: [],
  allEdges: [],
  nodes: [],
  edges: [],
  positions: new Map(),
  velocities: new Map(),
  viewport: { x: 0, y: 0, scale: 1 },
  dragging: false,
  lastPointer: null,
  simulationTicks: 0,
};

async function loadSummary() {
  const summary = await json("/api/summary");
  document.querySelector("#summary").textContent = text(summary);
  const hubs = document.querySelector("#hubs");
  hubs.innerHTML = "";
  for (const hub of summary.hubs || []) {
    const item = document.createElement("li");
    item.textContent = `${hub.qualified_name} (${hub.in_degree + hub.out_degree})`;
    hubs.appendChild(item);
  }
}

async function loadGraph() {
  const [nodes, edges] = await Promise.all([json("/api/nodes"), json("/api/edges")]);
  graphState.allNodes = Array.isArray(nodes) ? nodes : [];
  graphState.allEdges = Array.isArray(edges) ? edges : [];
  applyGraphFilters();
}

function applyGraphFilters() {
  graphState.nodes = visibleNodes(graphState.allNodes);
  const visibleIds = new Set(graphState.nodes.map((node) => node.id));
  graphState.edges = graphState.allEdges.filter(
    (edge) => visibleIds.has(edge.from) && visibleIds.has(edge.to),
  );
  document.querySelector("#graph-count").textContent =
    `${graphState.nodes.length} nodes / ${graphState.edges.length} edges`;
  seedPositions();
  runForceLayout();
  drawGraph();
}

function visibleNodes(nodes) {
  const filter = document.querySelector("#graph-filter")?.value.trim().toLowerCase() || "";
  const kind = document.querySelector("#graph-kind")?.value || "";
  return nodes
    .filter((node) => !["repository", "import"].includes(node.kind))
    .filter((node) => !kind || node.kind === kind)
    .filter((node) => {
      if (!filter) return true;
      return `${node.name} ${node.qualified_name} ${node.path}`.toLowerCase().includes(filter);
    })
    .sort((left, right) => degree(right) - degree(left))
    .slice(0, 48);
}

function degree(node) {
  return (node.metrics?.in_degree || 0) + (node.metrics?.out_degree || 0);
}

function drawGraph() {
  const canvas = document.querySelector("#graph-canvas");
  const rect = canvas.getBoundingClientRect();
  const scale = window.devicePixelRatio || 1;
  canvas.width = Math.max(640, Math.floor(rect.width * scale));
  canvas.height = Math.max(320, Math.floor(rect.height * scale));
  const ctx = canvas.getContext("2d");
  ctx.setTransform(scale, 0, 0, scale, 0, 0);
  const width = canvas.width / scale;
  const height = canvas.height / scale;
  ctx.clearRect(0, 0, width, height);
  if (graphState.simulationTicks < 90) {
    tickForceLayout(width, height);
    requestAnimationFrame(drawGraph);
  }
  ctx.save();
  ctx.translate(graphState.viewport.x, graphState.viewport.y);
  ctx.scale(graphState.viewport.scale, graphState.viewport.scale);

  ctx.lineWidth = 1;
  for (const edge of graphState.edges) {
    const from = graphState.positions.get(edge.from);
    const to = graphState.positions.get(edge.to);
    if (!from || !to) continue;
    ctx.strokeStyle = edge.kind === "calls" ? "#2563eb55" : "#66708533";
    ctx.beginPath();
    ctx.moveTo(from.x, from.y);
    ctx.lineTo(to.x, to.y);
    ctx.stroke();
  }

  for (const node of graphState.nodes) {
    const point = graphState.positions.get(node.id);
    const radius = Math.max(5, Math.min(13, 5 + degree(node)));
    ctx.fillStyle = colorForKind(node.kind);
    ctx.strokeStyle = "#ffffff";
    ctx.lineWidth = 2;
    ctx.beginPath();
    ctx.arc(point.x, point.y, radius, 0, Math.PI * 2);
    ctx.fill();
    ctx.stroke();
    ctx.fillStyle = "#20242c";
    ctx.font = "12px Inter, system-ui, sans-serif";
    ctx.textBaseline = "middle";
    ctx.fillText(node.name, point.x + radius + 5, point.y);
  }
  ctx.restore();
}

function seedPositions() {
  graphState.positions.clear();
  graphState.velocities.clear();
  graphState.simulationTicks = 0;
  const canvas = document.querySelector("#graph-canvas");
  const width = Math.max(640, canvas.getBoundingClientRect().width);
  const height = Math.max(320, canvas.getBoundingClientRect().height);
  const centerX = width / 2;
  const centerY = height / 2;
  const radiusX = Math.max(160, width * 0.38);
  const radiusY = Math.max(105, height * 0.33);
  graphState.nodes.forEach((node, index) => {
    const angle = (Math.PI * 2 * index) / Math.max(1, graphState.nodes.length);
    const hubPull = Math.min(0.45, degree(node) / 28);
    graphState.positions.set(node.id, {
      x: centerX + Math.cos(angle) * radiusX * (1 - hubPull),
      y: centerY + Math.sin(angle) * radiusY * (1 - hubPull),
    });
    graphState.velocities.set(node.id, { x: 0, y: 0 });
  });
}

function runForceLayout() {
  graphState.simulationTicks = 0;
}

function tickForceLayout(width, height) {
  const center = { x: width / 2, y: height / 2 };
  const nodes = graphState.nodes;
  const edgePairs = graphState.edges
    .map((edge) => [graphState.positions.get(edge.from), graphState.positions.get(edge.to)])
    .filter(([from, to]) => from && to);
  for (let i = 0; i < nodes.length; i += 1) {
    const left = graphState.positions.get(nodes[i].id);
    const leftVelocity = graphState.velocities.get(nodes[i].id);
    if (!left || !leftVelocity) continue;
    leftVelocity.x += (center.x - left.x) * 0.0008;
    leftVelocity.y += (center.y - left.y) * 0.0008;
    for (let j = i + 1; j < nodes.length; j += 1) {
      const right = graphState.positions.get(nodes[j].id);
      const rightVelocity = graphState.velocities.get(nodes[j].id);
      if (!right || !rightVelocity) continue;
      const dx = left.x - right.x;
      const dy = left.y - right.y;
      const distanceSquared = Math.max(60, dx * dx + dy * dy);
      const force = 620 / distanceSquared;
      leftVelocity.x += dx * force;
      leftVelocity.y += dy * force;
      rightVelocity.x -= dx * force;
      rightVelocity.y -= dy * force;
    }
  }
  for (const [from, to] of edgePairs) {
    const dx = to.x - from.x;
    const dy = to.y - from.y;
    const distance = Math.max(1, Math.hypot(dx, dy));
    const force = (distance - 145) * 0.002;
    const fx = (dx / distance) * force;
    const fy = (dy / distance) * force;
    for (const [point, direction] of [[from, 1], [to, -1]]) {
      const node = nearestNode(point);
      if (!node) continue;
      const velocity = graphState.velocities.get(node.id);
      velocity.x += fx * direction;
      velocity.y += fy * direction;
    }
  }
  for (const node of nodes) {
    const point = graphState.positions.get(node.id);
    const velocity = graphState.velocities.get(node.id);
    if (!point || !velocity) continue;
    point.x += velocity.x;
    point.y += velocity.y;
    velocity.x *= 0.82;
    velocity.y *= 0.82;
  }
  graphState.simulationTicks += 1;
}

function nearestNode(point) {
  for (const node of graphState.nodes) {
    if (graphState.positions.get(node.id) === point) return node;
  }
  return null;
}

function colorForKind(kind) {
  switch (kind) {
    case "file":
      return "#12a594";
    case "module":
      return "#8b5cf6";
    case "class":
    case "struct":
    case "interface":
    case "trait":
      return "#d97706";
    case "method":
      return "#db2777";
    default:
      return "#2563eb";
  }
}

async function search(query) {
  const data = await json(`/api/rank?query=${encodeURIComponent(query)}&limit=10`);
  const results = document.querySelector("#results");
  results.innerHTML = "";
  for (const result of data.results || []) {
    const item = document.createElement("li");
    item.textContent = `${result.node.qualified_name} ${result.score.toFixed(3)}`;
    item.addEventListener("click", () => {
      document.querySelector("#detail").textContent = text(result.node);
    });
    results.appendChild(item);
  }
}

document.querySelector("#search-form").addEventListener("submit", (event) => {
  event.preventDefault();
  search(document.querySelector("#query").value);
});

document.querySelector("#graph-canvas").addEventListener("click", (event) => {
  const canvas = event.currentTarget;
  const rect = canvas.getBoundingClientRect();
  const x = (event.clientX - rect.left - graphState.viewport.x) / graphState.viewport.scale;
  const y = (event.clientY - rect.top - graphState.viewport.y) / graphState.viewport.scale;
  let nearest = null;
  let distance = Infinity;
  for (const node of graphState.nodes) {
    const point = graphState.positions.get(node.id);
    if (!point) continue;
    const candidate = Math.hypot(point.x - x, point.y - y);
    if (candidate < distance) {
      nearest = node;
      distance = candidate;
    }
  }
  if (nearest && distance < 28) {
    document.querySelector("#detail").textContent = text(nearest);
  }
});

document.querySelector("#graph-canvas").addEventListener("pointerdown", (event) => {
  graphState.dragging = true;
  graphState.lastPointer = { x: event.clientX, y: event.clientY };
  event.currentTarget.setPointerCapture(event.pointerId);
});

document.querySelector("#graph-canvas").addEventListener("pointermove", (event) => {
  if (!graphState.dragging || !graphState.lastPointer) return;
  graphState.viewport.x += event.clientX - graphState.lastPointer.x;
  graphState.viewport.y += event.clientY - graphState.lastPointer.y;
  graphState.lastPointer = { x: event.clientX, y: event.clientY };
  drawGraph();
});

document.querySelector("#graph-canvas").addEventListener("pointerup", (event) => {
  graphState.dragging = false;
  graphState.lastPointer = null;
  event.currentTarget.releasePointerCapture(event.pointerId);
});

document.querySelector("#graph-canvas").addEventListener("wheel", (event) => {
  event.preventDefault();
  zoomGraph(event.deltaY < 0 ? 1.12 : 0.88, event.offsetX, event.offsetY);
});

document.querySelector("#graph-filter").addEventListener("input", applyGraphFilters);
document.querySelector("#graph-kind").addEventListener("change", applyGraphFilters);
document.querySelector("#graph-zoom-in").addEventListener("click", () => zoomGraph(1.18));
document.querySelector("#graph-zoom-out").addEventListener("click", () => zoomGraph(0.84));
document.querySelector("#graph-reset").addEventListener("click", () => {
  graphState.viewport = { x: 0, y: 0, scale: 1 };
  seedPositions();
  runForceLayout();
  drawGraph();
});

function zoomGraph(factor, originX = 450, originY = 210) {
  const before = {
    x: (originX - graphState.viewport.x) / graphState.viewport.scale,
    y: (originY - graphState.viewport.y) / graphState.viewport.scale,
  };
  graphState.viewport.scale = Math.max(0.35, Math.min(3.5, graphState.viewport.scale * factor));
  graphState.viewport.x = originX - before.x * graphState.viewport.scale;
  graphState.viewport.y = originY - before.y * graphState.viewport.scale;
  drawGraph();
}

window.addEventListener("resize", drawGraph);
loadSummary();
loadGraph();
search(document.querySelector("#query").value);
