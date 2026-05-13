async function json(path) {
  const response = await fetch(path);
  return response.json();
}

function text(value) {
  return JSON.stringify(value, null, 2);
}

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

loadSummary();
search(document.querySelector("#query").value);

