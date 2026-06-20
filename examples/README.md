# Observability examples

Starter, copy-and-adapt artifacts for collecting `game-shell-input`'s metrics.
The daemon only **emits** the signal (see [`../docs/OBSERVABILITY.md`](../docs/OBSERVABILITY.md));
collection is deployment-private — these are non-binding examples.

| File | What it is |
|---|---|
| `grafana-dashboard.json` | Grafana dashboard with panels for the `game_shell_*` counters (rates) and the convenience resource gauges. Import via Grafana → Dashboards → New → Import. Set the `DS_PROMETHEUS` datasource variable on import. |
| `prometheus-scrape.yaml` | A `scrape_configs` snippet for scraping the auth-exempt `GET /metrics` endpoint directly. |

## node_exporter textfile collector (primary path)

The recommended path needs **no Prometheus scrape of the daemon at all** — the
daemon writes a `.prom` file and an existing node_exporter picks it up:

1. Pick a textfile directory node_exporter watches, e.g.
   `/var/lib/node_exporter/textfile/`, and start node_exporter with
   `--collector.textfile.directory=/var/lib/node_exporter/textfile`.
2. Point the daemon at a `.prom` file **inside** that directory:
   `GAME_SHELL_METRICS_TEXTFILE=/var/lib/node_exporter/textfile/game-shell.prom`
   (optionally `GAME_SHELL_METRICS_INTERVAL=15`).
3. The daemon renders every interval and writes atomically (temp file +
   `rename`), so node_exporter never reads a half-written file. The metrics then
   appear on node_exporter's own `/metrics` alongside the `node_*` series, and
   your existing Prometheus job scrapes them for free.

This is preferred when a node_exporter already runs on the host: the daemon needs
no open port, and `game_shell_*` rides the node_exporter scrape you already have.
Use `prometheus-scrape.yaml` instead only when you'd rather scrape the daemon's
own `/metrics` directly (no node_exporter, or you want a separate target).
