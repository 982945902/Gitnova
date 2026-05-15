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
  webgl: null,
  viewport: { x: 0, y: 0, scale: 1 },
  highlightNodeIds: new Set(),
  highlightEdgeKeys: new Set(),
  selectedNodeId: null,
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
  const width = canvas.width / scale;
  const height = canvas.height / scale;
  if (graphState.simulationTicks < 90) {
    tickForceLayout(width, height);
    requestAnimationFrame(drawGraph);
  }
  const renderer = webglRenderer(canvas);
  if (renderer) {
    drawGraphWebgl(renderer, width, height);
  } else {
    drawGraph2d(canvas, width, height);
  }
  updateGraphLabels(width, height);
}

function webglRenderer(canvas) {
  if (graphState.webgl?.canvas === canvas) return graphState.webgl;
  const gl = canvas.getContext("webgl", {
    alpha: false,
    antialias: true,
    preserveDrawingBuffer: true,
  });
  if (!gl) return null;
  const pointProgram = createProgram(
    gl,
    `
      attribute vec2 a_position;
      attribute vec4 a_color;
      attribute float a_size;
      varying vec4 v_color;
      void main() {
        gl_Position = vec4(a_position, 0.0, 1.0);
        gl_PointSize = a_size;
        v_color = a_color;
      }
    `,
    `
      precision mediump float;
      varying vec4 v_color;
      void main() {
        vec2 offset = gl_PointCoord - vec2(0.5);
        if (dot(offset, offset) > 0.25) {
          discard;
        }
        gl_FragColor = v_color;
      }
    `,
  );
  const lineProgram = createProgram(
    gl,
    `
      attribute vec2 a_position;
      attribute vec4 a_color;
      varying vec4 v_color;
      void main() {
        gl_Position = vec4(a_position, 0.0, 1.0);
        v_color = a_color;
      }
    `,
    `
      precision mediump float;
      varying vec4 v_color;
      void main() {
        gl_FragColor = v_color;
      }
    `,
  );
  graphState.webgl = {
    canvas,
    gl,
    pointProgram,
    lineProgram,
    pointBuffer: gl.createBuffer(),
    lineBuffer: gl.createBuffer(),
  };
  return graphState.webgl;
}

function createProgram(gl, vertexSource, fragmentSource) {
  const vertex = compileShader(gl, gl.VERTEX_SHADER, vertexSource);
  const fragment = compileShader(gl, gl.FRAGMENT_SHADER, fragmentSource);
  const program = gl.createProgram();
  gl.attachShader(program, vertex);
  gl.attachShader(program, fragment);
  gl.linkProgram(program);
  if (!gl.getProgramParameter(program, gl.LINK_STATUS)) {
    throw new Error(gl.getProgramInfoLog(program) || "Unable to link WebGL program");
  }
  return program;
}

function compileShader(gl, type, source) {
  const shader = gl.createShader(type);
  gl.shaderSource(shader, source);
  gl.compileShader(shader);
  if (!gl.getShaderParameter(shader, gl.COMPILE_STATUS)) {
    throw new Error(gl.getShaderInfoLog(shader) || "Unable to compile WebGL shader");
  }
  return shader;
}

function drawGraphWebgl(renderer, width, height) {
  const { gl } = renderer;
  const pixelRatio = window.devicePixelRatio || 1;
  gl.viewport(0, 0, Math.floor(width * pixelRatio), Math.floor(height * pixelRatio));
  gl.clearColor(1, 1, 1, 1);
  gl.clear(gl.COLOR_BUFFER_BIT);
  gl.enable(gl.BLEND);
  gl.blendFunc(gl.SRC_ALPHA, gl.ONE_MINUS_SRC_ALPHA);
  drawWebglLines(renderer, width, height);
  drawWebglPoints(renderer, width, height);
}

function drawWebglLines(renderer, width, height) {
  const { gl, lineProgram, lineBuffer } = renderer;
  const data = [];
  for (const edge of graphState.edges) {
    const from = graphState.positions.get(edge.from);
    const to = graphState.positions.get(edge.to);
    if (!from || !to) continue;
    const highlighted = graphState.highlightEdgeKeys.has(edgeKey(edge));
    const color = highlighted ? rgba("#e11d48", 0.95) : edge.kind === "calls" ? rgba("#2563eb", 0.34) : rgba("#667085", 0.22);
    for (const point of [from, to]) {
      const clip = clipPoint(point, width, height);
      data.push(clip.x, clip.y, color.r, color.g, color.b, color.a);
    }
  }
  gl.useProgram(lineProgram);
  gl.bindBuffer(gl.ARRAY_BUFFER, lineBuffer);
  gl.bufferData(gl.ARRAY_BUFFER, new Float32Array(data), gl.DYNAMIC_DRAW);
  const stride = 6 * Float32Array.BYTES_PER_ELEMENT;
  const position = gl.getAttribLocation(lineProgram, "a_position");
  const color = gl.getAttribLocation(lineProgram, "a_color");
  gl.enableVertexAttribArray(position);
  gl.vertexAttribPointer(position, 2, gl.FLOAT, false, stride, 0);
  gl.enableVertexAttribArray(color);
  gl.vertexAttribPointer(color, 4, gl.FLOAT, false, stride, 2 * Float32Array.BYTES_PER_ELEMENT);
  gl.lineWidth(1);
  gl.drawArrays(gl.LINES, 0, data.length / 6);
}

function drawWebglPoints(renderer, width, height) {
  const { gl, pointProgram, pointBuffer } = renderer;
  const data = [];
  const pixelRatio = window.devicePixelRatio || 1;
  for (const node of graphState.nodes) {
    const point = graphState.positions.get(node.id);
    if (!point) continue;
    const highlighted = graphState.highlightNodeIds.has(node.id);
    const selected = graphState.selectedNodeId === node.id;
    const clip = clipPoint(point, width, height);
    const fill = rgba(colorForKind(node.kind), 0.92);
    if (highlighted || selected) {
      const ring = selected ? rgba("#111827", 0.96) : rgba("#e11d48", 0.9);
      data.push(clip.x, clip.y, ring.r, ring.g, ring.b, ring.a, (nodeRadius(node) * 2 + 8) * pixelRatio);
    }
    data.push(clip.x, clip.y, fill.r, fill.g, fill.b, fill.a, nodeRadius(node) * 2 * pixelRatio);
  }
  gl.useProgram(pointProgram);
  gl.bindBuffer(gl.ARRAY_BUFFER, pointBuffer);
  gl.bufferData(gl.ARRAY_BUFFER, new Float32Array(data), gl.DYNAMIC_DRAW);
  const stride = 7 * Float32Array.BYTES_PER_ELEMENT;
  const position = gl.getAttribLocation(pointProgram, "a_position");
  const color = gl.getAttribLocation(pointProgram, "a_color");
  const size = gl.getAttribLocation(pointProgram, "a_size");
  gl.enableVertexAttribArray(position);
  gl.vertexAttribPointer(position, 2, gl.FLOAT, false, stride, 0);
  gl.enableVertexAttribArray(color);
  gl.vertexAttribPointer(color, 4, gl.FLOAT, false, stride, 2 * Float32Array.BYTES_PER_ELEMENT);
  gl.enableVertexAttribArray(size);
  gl.vertexAttribPointer(size, 1, gl.FLOAT, false, stride, 6 * Float32Array.BYTES_PER_ELEMENT);
  gl.drawArrays(gl.POINTS, 0, data.length / 7);
}

function drawGraph2d(canvas, width, height) {
  const ctx = canvas.getContext("2d");
  const scale = window.devicePixelRatio || 1;
  ctx.setTransform(scale, 0, 0, scale, 0, 0);
  ctx.clearRect(0, 0, width, height);
  ctx.save();
  ctx.translate(graphState.viewport.x, graphState.viewport.y);
  ctx.scale(graphState.viewport.scale, graphState.viewport.scale);
  for (const edge of graphState.edges) {
    const from = graphState.positions.get(edge.from);
    const to = graphState.positions.get(edge.to);
    if (!from || !to) continue;
    const highlighted = graphState.highlightEdgeKeys.has(edgeKey(edge));
    ctx.strokeStyle = highlighted ? "#e11d48" : edge.kind === "calls" ? "#2563eb55" : "#66708533";
    ctx.lineWidth = highlighted ? 2.5 : 1;
    ctx.beginPath();
    ctx.moveTo(from.x, from.y);
    ctx.lineTo(to.x, to.y);
    ctx.stroke();
  }

  for (const node of graphState.nodes) {
    const point = graphState.positions.get(node.id);
    const highlighted = graphState.highlightNodeIds.has(node.id);
    const selected = graphState.selectedNodeId === node.id;
    const radius = nodeRadius(node);
    ctx.fillStyle = colorForKind(node.kind);
    ctx.strokeStyle = selected ? "#111827" : highlighted ? "#e11d48" : "#ffffff";
    ctx.lineWidth = selected ? 3 : 2;
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

function nodeRadius(node) {
  const highlighted = graphState.highlightNodeIds.has(node.id);
  return Math.max(5, Math.min(13, 5 + degree(node))) + (highlighted ? 3 : 0);
}

function clipPoint(point, width, height) {
  const screen = screenPoint(point);
  return {
    x: (screen.x / width) * 2 - 1,
    y: 1 - (screen.y / height) * 2,
  };
}

function screenPoint(point) {
  return {
    x: point.x * graphState.viewport.scale + graphState.viewport.x,
    y: point.y * graphState.viewport.scale + graphState.viewport.y,
  };
}

function rgba(hex, alpha) {
  const value = hex.replace("#", "");
  const parsed = Number.parseInt(value, 16);
  return {
    r: ((parsed >> 16) & 255) / 255,
    g: ((parsed >> 8) & 255) / 255,
    b: (parsed & 255) / 255,
    a: alpha,
  };
}

function updateGraphLabels(width, height) {
  const labels = document.querySelector("#graph-labels");
  labels.innerHTML = "";
  const visible = graphState.nodes
    .sort((left, right) => {
      const leftHighlighted = graphState.highlightNodeIds.has(left.id) ? 1 : 0;
      const rightHighlighted = graphState.highlightNodeIds.has(right.id) ? 1 : 0;
      return rightHighlighted - leftHighlighted || degree(right) - degree(left);
    })
    .slice(0, 26);
  for (const node of visible) {
    const point = graphState.positions.get(node.id);
    if (!point) continue;
    const screen = screenPoint(point);
    const x = Math.max(4, Math.min(width - 160, screen.x + nodeRadius(node) + 6));
    const y = Math.max(4, Math.min(height - 24, screen.y - 10));
    const label = document.createElement("span");
    label.className = graphState.highlightNodeIds.has(node.id)
      ? "graph-label is-highlighted"
      : "graph-label";
    label.textContent = node.name;
    label.style.transform = `translate(${Math.round(x)}px, ${Math.round(y)}px)`;
    labels.appendChild(label);
  }
}

function edgeKey(edge) {
  return `${edge.from}->${edge.to}:${edge.kind}`;
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
  const [data, answer] = await Promise.all([
    json(`/api/rank?query=${encodeURIComponent(query)}&limit=10`),
    json(`/api/answer?query=${encodeURIComponent(query)}&depth=1&limit=40`),
  ]);
  const results = document.querySelector("#results");
  results.innerHTML = "";
  renderAnswer(answer);
  for (const result of data.results || []) {
    const item = document.createElement("li");
    item.textContent = `${result.node.qualified_name} ${result.score.toFixed(3)}`;
    item.addEventListener("click", () => {
      focusContext(result.node.id, result.node);
    });
    results.appendChild(item);
  }
  if (answer.context?.target?.id) {
    applyContextHighlight(answer.context, answer.context.target.id, {
      answer: answer.answer,
      target: answer.context.target,
      evidence: answer.evidence || [],
      context_summary: answer.context.summary,
      edges: answer.context.edges || [],
    });
  } else if (data.results?.[0]?.node?.id) {
    await focusContext(data.results[0].node.id, data.results[0].node);
  }
}

function renderAnswer(answer) {
  const answerBox = document.querySelector("#answer");
  const evidenceList = document.querySelector("#evidence");
  answerBox.innerHTML = "";
  evidenceList.innerHTML = "";

  const body = document.createElement("div");
  body.textContent = answer.answer || "No answer available.";
  answerBox.appendChild(body);

  const meta = document.createElement("div");
  meta.className = "answer-meta";
  meta.textContent = answer.llm_used
    ? `LLM: ${answer.model || "configured model"}`
    : `Fallback: ${answer.fallback_reason || "deterministic evidence answer"}`;
  answerBox.appendChild(meta);

  for (const item of answer.evidence || []) {
    const entry = document.createElement("li");
    const span = item.span ? `:${item.span.start_line}` : "";
    entry.textContent = `${item.qualified_name} (${item.path}${span})`;
    entry.addEventListener("click", () => focusContext(item.node_id, item));
    evidenceList.appendChild(entry);
  }
}

async function focusContext(nodeId, fallbackNode) {
  const context = await json(
    `/api/graph-context?node_id=${encodeURIComponent(nodeId)}&depth=1&limit=40`,
  );
  applyContextHighlight(context, nodeId, {
    summary: context.summary,
    target: context.target || fallbackNode,
    incoming: context.incoming || [],
    outgoing: context.outgoing || [],
    edges: context.edges || [],
  });
}

function applyContextHighlight(context, selectedNodeId, detailPayload) {
  graphState.highlightNodeIds = new Set((context.nodes || []).map((node) => node.id));
  graphState.highlightEdgeKeys = new Set((context.edges || []).map(edgeKey));
  graphState.selectedNodeId = selectedNodeId;
  document.querySelector("#detail").textContent = text(detailPayload);
  drawGraph();
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
    focusContext(nearest.id, nearest);
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

async function init() {
  await Promise.all([loadSummary(), loadGraph()]);
  await search(document.querySelector("#query").value);
}

init();
