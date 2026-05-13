# Dashboard

Start the local dashboard after indexing a repository:

```bash
cargo run -p gitnova -- index /path/to/repo --force
cargo run -p gitnova -- dashboard --repo /path/to/repo --port 4567
```

Open `http://127.0.0.1:4567`.

Routes:

- `/`
- `/api/summary`
- `/api/nodes`
- `/api/edges`
- `/api/rank?query=...`
- `/api/hubs`
- `/api/churn`

The dashboard has no build step. Assets are plain HTML, CSS, and JavaScript
served by Axum.

