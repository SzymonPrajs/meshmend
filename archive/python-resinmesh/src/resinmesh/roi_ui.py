"""Small local browser UI for ROI probing."""

from __future__ import annotations

from dataclasses import asdict
import json
import mimetypes
from pathlib import Path
import subprocess
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import parse_qs, urlparse

from .roi import RoiProbeConfig, probe_roi, render_roi_views


def launch_roi_ui(
    source: Path,
    output_dir: Path,
    *,
    host: str = "127.0.0.1",
    port: int = 8765,
    view_count: int = 24,
    image_size: int = 768,
) -> None:
    """Start a local UI for marking a circular ROI and running probes."""

    output_dir.mkdir(parents=True, exist_ok=True)
    views_dir = output_dir / "views"
    probes_dir = output_dir / "probes"
    views_dir.mkdir(parents=True, exist_ok=True)
    probes_dir.mkdir(parents=True, exist_ok=True)
    view_report = render_roi_views(source, views_dir, view_count, image_size, padding=0.08)

    class RoiHandler(BaseHTTPRequestHandler):
        server_version = "ResinMeshROI/0.1"

        def do_GET(self) -> None:  # noqa: N802
            parsed = urlparse(self.path)
            if parsed.path == "/":
                self.respond_html(render_index_html(view_report, image_size))
                return
            if parsed.path == "/view":
                params = parse_qs(parsed.query)
                index = int(params.get("index", ["0"])[0])
                path = views_dir / f"view_{index:03d}_normal.png"
                self.respond_file(path)
                return
            if parsed.path == "/file":
                params = parse_qs(parsed.query)
                requested = Path(params.get("path", [""])[0]).resolve()
                if not is_under(requested, output_dir.resolve()):
                    self.send_error(403, "Path outside ROI output directory")
                    return
                self.respond_file(requested)
                return
            if parsed.path == "/report":
                self.respond_json(view_report)
                return
            self.send_error(404)

        def do_POST(self) -> None:  # noqa: N802
            parsed = urlparse(self.path)
            if parsed.path != "/probe":
                self.send_error(404)
                return
            length = int(self.headers.get("Content-Length", "0"))
            payload = json.loads(self.rfile.read(length).decode("utf-8"))
            view_index = int(payload["viewIndex"])
            circle_x = float(payload["x"])
            circle_y = float(payload["y"])
            radius = float(payload["radius"])
            probe_dir = probes_dir / f"view_{view_index:03d}_x{int(circle_x)}_y{int(circle_y)}_r{int(radius)}"
            config = RoiProbeConfig(
                view_count=view_count,
                view_index=view_index,
                image_size=image_size,
                circle_x=circle_x,
                circle_y=circle_y,
                circle_radius=radius,
                max_hits=12,
                section_size=768,
                local_expand=1.0,
            )
            report = probe_roi(source, probe_dir, config)
            self.respond_json(
                {
                    "report": report["report"],
                    "markdown": report["markdown"],
                    "overlay": report["images"]["overlay_sheet"],
                    "sections": report["images"]["cross_sections"],
                    "localMesh": report["roi"]["local_volume_mesh"],
                    "hitMesh": report["roi"]["hit_faces_mesh"],
                    "summary": report["roi"],
                }
            )

        def log_message(self, format: str, *args: object) -> None:
            print(f"[roi-ui] {self.address_string()} - {format % args}")

        def respond_html(self, html: str) -> None:
            body = html.encode("utf-8")
            self.send_response(200)
            self.send_header("Content-Type", "text/html; charset=utf-8")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)

        def respond_json(self, data: object) -> None:
            body = json.dumps(data, indent=2).encode("utf-8")
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)

        def respond_file(self, path: Path) -> None:
            if not path.exists() or not path.is_file():
                self.send_error(404)
                return
            body = path.read_bytes()
            content_type = mimetypes.guess_type(path.name)[0] or "application/octet-stream"
            self.send_response(200)
            self.send_header("Content-Type", content_type)
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)

    server = ThreadingHTTPServer((host, port), RoiHandler)
    url = f"http://{host}:{server.server_address[1]}"
    print(f"ROI UI: {url}")
    print(f"Output directory: {output_dir}")
    open_browser(url)
    try:
        server.serve_forever()
    finally:
        server.server_close()


def is_under(path: Path, root: Path) -> bool:
    try:
        path.relative_to(root)
        return True
    except ValueError:
        return False


def open_browser(url: str) -> None:
    try:
        subprocess.run(["open", url], check=False)
    except Exception:
        pass


def render_index_html(view_report: dict, image_size: int) -> str:
    views = view_report["views"]
    options = "\n".join(f'<option value="{view["index"]}">view {view["index"]:03d}</option>' for view in views)
    return f"""<!doctype html>
<html>
<head>
  <meta charset="utf-8">
  <title>ResinMesh ROI Probe</title>
  <style>
    body {{ margin: 0; font-family: -apple-system, BlinkMacSystemFont, sans-serif; background: #101114; color: #f1f1f1; }}
    header {{ padding: 12px 16px; border-bottom: 1px solid #2d2f36; display: flex; gap: 12px; align-items: center; }}
    main {{ display: grid; grid-template-columns: minmax(520px, {image_size}px) 1fr; gap: 16px; padding: 16px; }}
    canvas {{ background: #000; width: 100%; height: auto; border: 1px solid #343640; cursor: crosshair; }}
    button, select, input {{ background: #20222a; color: #f1f1f1; border: 1px solid #444854; border-radius: 4px; padding: 6px 8px; }}
    button {{ cursor: pointer; }}
    .panel {{ background: #17191f; border: 1px solid #2d2f36; border-radius: 6px; padding: 12px; }}
    .row {{ display: flex; gap: 8px; align-items: center; margin: 8px 0; flex-wrap: wrap; }}
    img {{ max-width: 100%; border: 1px solid #343640; background: #000; }}
    code {{ color: #9ddcff; overflow-wrap: anywhere; }}
    .muted {{ color: #a6a8b0; }}
  </style>
</head>
<body>
  <header>
    <strong>ResinMesh ROI Probe</strong>
    <label>View <select id="viewSelect">{options}</select></label>
    <label>Radius <input id="radius" type="number" value="70" min="4" step="1"></label>
    <button id="run">Run Probe</button>
    <span id="status" class="muted"></span>
  </header>
  <main>
    <section class="panel">
      <canvas id="canvas" width="{image_size}" height="{image_size}"></canvas>
      <div class="row muted">Click to set the centre. Drag to adjust the radius. Then run probe.</div>
    </section>
    <section class="panel">
      <h3>Output</h3>
      <div id="summary" class="muted">No probe run yet.</div>
      <h4>Overlay</h4>
      <img id="overlay">
      <h4>Cross-sections</h4>
      <img id="sections">
      <h4>Files</h4>
      <div id="files"></div>
    </section>
  </main>
  <script>
    const canvas = document.getElementById('canvas');
    const ctx = canvas.getContext('2d');
    const viewSelect = document.getElementById('viewSelect');
    const radiusInput = document.getElementById('radius');
    const statusEl = document.getElementById('status');
    let image = new Image();
    let circle = {{ x: {image_size / 2}, y: {image_size / 2}, r: 70 }};
    let dragging = false;

    function loadView() {{
      image = new Image();
      image.onload = draw;
      image.src = `/view?index=${{viewSelect.value}}&t=${{Date.now()}}`;
    }}

    function draw() {{
      ctx.clearRect(0, 0, canvas.width, canvas.height);
      ctx.drawImage(image, 0, 0, canvas.width, canvas.height);
      ctx.strokeStyle = '#ff2727';
      ctx.lineWidth = 4;
      ctx.beginPath();
      ctx.arc(circle.x, circle.y, circle.r, 0, Math.PI * 2);
      ctx.stroke();
      ctx.beginPath();
      ctx.moveTo(circle.x - 12, circle.y);
      ctx.lineTo(circle.x + 12, circle.y);
      ctx.moveTo(circle.x, circle.y - 12);
      ctx.lineTo(circle.x, circle.y + 12);
      ctx.stroke();
    }}

    function pointerPos(event) {{
      const rect = canvas.getBoundingClientRect();
      return {{
        x: (event.clientX - rect.left) * canvas.width / rect.width,
        y: (event.clientY - rect.top) * canvas.height / rect.height
      }};
    }}

    canvas.addEventListener('pointerdown', event => {{
      const p = pointerPos(event);
      circle.x = p.x;
      circle.y = p.y;
      circle.r = Number(radiusInput.value);
      dragging = true;
      draw();
    }});
    canvas.addEventListener('pointermove', event => {{
      if (!dragging) return;
      const p = pointerPos(event);
      circle.r = Math.max(4, Math.hypot(p.x - circle.x, p.y - circle.y));
      radiusInput.value = Math.round(circle.r);
      draw();
    }});
    window.addEventListener('pointerup', () => dragging = false);
    radiusInput.addEventListener('input', () => {{
      circle.r = Number(radiusInput.value);
      draw();
    }});
    viewSelect.addEventListener('change', loadView);

    document.getElementById('run').addEventListener('click', async () => {{
      statusEl.textContent = 'Running probe...';
      const response = await fetch('/probe', {{
        method: 'POST',
        headers: {{ 'Content-Type': 'application/json' }},
        body: JSON.stringify({{
          viewIndex: Number(viewSelect.value),
          x: circle.x,
          y: circle.y,
          radius: circle.r
        }})
      }});
      const data = await response.json();
      statusEl.textContent = 'Done';
      document.getElementById('summary').textContent =
        `${{data.summary.unique_hit_faces}} hit faces, ${{data.summary.local_volume_faces}} local faces`;
      document.getElementById('overlay').src = `/file?path=${{encodeURIComponent(data.overlay)}}&t=${{Date.now()}}`;
      document.getElementById('sections').src = `/file?path=${{encodeURIComponent(data.sections)}}&t=${{Date.now()}}`;
      document.getElementById('files').innerHTML = `
        <p>Report: <code>${{data.report}}</code></p>
        <p>Local mesh: <code>${{data.localMesh}}</code></p>
        <p>Hit mesh: <code>${{data.hitMesh}}</code></p>`;
    }});

    loadView();
  </script>
</body>
</html>"""
