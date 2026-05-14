async function json(path) {
  const response = await fetch(path);
  return response.json();
}

function text(value) {
  return JSON.stringify(value, null, 2);
}

const graphState = {
  nodes: [],
  edges: [],
  positions: new Map(),
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
  graphState.nodes = visibleNodes(Array.isArray(nodes) ? nodes : []);
  const visibleIds = new Set(graphState.nodes.map((node) => node.id));
  graphState.edges = (Array.isArray(edges) ? edges : []).filter(
    (edge) => visibleIds.has(edge.from) && visibleIds.has(edge.to),
  );
  document.querySelector("#graph-count").textContent =
    `${graphState.nodes.length} nodes / ${graphState.edges.length} edges`;
  drawGraph();
}

function visibleNodes(nodes) {
  return nodes
    .filter((node) => !["repository", "import"].includes(node.kind))
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
  layoutGraph(width, height);

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
}

function layoutGraph(width, height) {
  graphState.positions.clear();
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
  });
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
  const x = event.clientX - rect.left;
  const y = event.clientY - rect.top;
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

window.addEventListener("resize", drawGraph);
loadSummary();
loadGraph();
search(document.querySelector("#query").value);
