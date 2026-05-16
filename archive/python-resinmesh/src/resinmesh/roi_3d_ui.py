"""Interactive Three.js ROI picker for local mesh repair planning."""

from __future__ import annotations

import json
import mimetypes
from pathlib import Path
import subprocess
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import parse_qs, urlparse

import numpy as np
import pymeshlab
import trimesh

from .core import bounds_for, jsonable, topology_for
from .diagnostics import load_mesh_arrays, make_contact_sheet
from .roi import export_mesh_arrays, label_tile, plane_intersection_segments, render_segments


def launch_roi_3d_ui(
    source: Path,
    output_dir: Path,
    *,
    host: str = "127.0.0.1",
    port: int = 8766,
    preview_faces: int = 250_000,
    default_radius_fraction: float = 0.035,
) -> None:
    output_dir.mkdir(parents=True, exist_ok=True)
    preview_path = output_dir / "preview.ply"
    selection_path = output_dir / "selected_points.json"
    analysis_dir = output_dir / "analysis"
    analysis_dir.mkdir(parents=True, exist_ok=True)

    preview_info = prepare_preview_mesh(source, preview_path, preview_faces)
    diagonal = float(np.linalg.norm(np.asarray(preview_info["bounds"]["dims"], dtype=float)))
    default_radius = diagonal * default_radius_fraction

    class Roi3DHandler(BaseHTTPRequestHandler):
        server_version = "ResinMesh3DROI/0.1"

        def do_GET(self) -> None:  # noqa: N802
            parsed = urlparse(self.path)
            if parsed.path == "/":
                self.respond_html(render_3d_html(preview_info, default_radius))
                return
            if parsed.path == "/model.ply":
                self.respond_file(preview_path)
                return
            if parsed.path == "/metadata":
                self.respond_json({"source": str(source), "preview": preview_info, "defaultRadius": default_radius})
                return
            if parsed.path == "/file":
                params = parse_qs(parsed.query)
                requested = Path(params.get("path", [""])[0]).resolve()
                if not is_under(requested, output_dir.resolve()):
                    self.send_error(403, "Path outside ROI output directory")
                    return
                self.respond_file(requested)
                return
            self.send_error(404)

        def do_POST(self) -> None:  # noqa: N802
            parsed = urlparse(self.path)
            if parsed.path not in {"/done", "/analyze"}:
                self.send_error(404)
                return
            length = int(self.headers.get("Content-Length", "0"))
            payload = json.loads(self.rfile.read(length).decode("utf-8"))
            points = normalize_points_payload(payload.get("points", []))
            radius = float(payload.get("radius", default_radius))
            selection = {
                "source": str(source),
                "preview": str(preview_path),
                "radius_source_units": radius,
                "points": points,
                "created_at": time.strftime("%Y-%m-%dT%H:%M:%S%z"),
            }
            selection_path.write_text(json.dumps(jsonable(selection), indent=2) + "\n")
            analysis = analyze_selected_points(source, analysis_dir, points, radius)
            response = {
                "selection": str(selection_path),
                "analysis": analysis,
                "pointCount": len(points),
            }
            self.respond_json(response)

        def log_message(self, format: str, *args: object) -> None:
            print(f"[roi-3d-ui] {self.address_string()} - {format % args}")

        def respond_html(self, html: str) -> None:
            body = html.encode("utf-8")
            self.send_response(200)
            self.send_header("Content-Type", "text/html; charset=utf-8")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)

        def respond_json(self, data: object) -> None:
            body = json.dumps(jsonable(data), indent=2).encode("utf-8")
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

    server = ThreadingHTTPServer((host, port), Roi3DHandler)
    url = f"http://{host}:{server.server_address[1]}"
    print(f"ROI 3D UI: {url}")
    print(f"Output directory: {output_dir}")
    open_browser(url)
    try:
        server.serve_forever()
    finally:
        server.server_close()


def prepare_preview_mesh(source: Path, preview_path: Path, preview_faces: int) -> dict:
    if preview_path.exists():
        ms_existing = pymeshlab.MeshSet()
        ms_existing.load_new_mesh(str(preview_path))
        mesh = ms_existing.current_mesh()
        return {
            "source": str(source),
            "preview": str(preview_path),
            "faces": int(mesh.face_number()),
            "vertices": int(mesh.vertex_number()),
            "bounds": bounds_for(mesh),
            "topology": topology_for(ms_existing),
            "reused": True,
        }

    ms = pymeshlab.MeshSet()
    ms.load_new_mesh(str(source))
    mesh = ms.current_mesh()
    source_faces = int(mesh.face_number())
    if source_faces > preview_faces:
        ms.meshing_decimation_quadric_edge_collapse(
            targetfacenum=preview_faces,
            preservenormal=True,
            preservetopology=False,
            optimalplacement=True,
            autoclean=True,
        )
    ms.meshing_remove_duplicate_vertices()
    ms.meshing_remove_duplicate_faces()
    ms.meshing_remove_unreferenced_vertices()
    ms.save_current_mesh(str(preview_path))
    preview = ms.current_mesh()
    return {
        "source": str(source),
        "preview": str(preview_path),
        "source_faces": source_faces,
        "faces": int(preview.face_number()),
        "vertices": int(preview.vertex_number()),
        "bounds": bounds_for(preview),
        "topology": topology_for(ms),
        "reused": False,
    }


def normalize_points_payload(points: list[dict]) -> list[dict]:
    normalized = []
    for index, point in enumerate(points):
        position = point.get("position") or {}
        normal = point.get("normal") or {}
        normalized.append(
            {
                "index": index,
                "label": point.get("label", f"P{index + 1}"),
                "position": {
                    "x": float(position.get("x", 0.0)),
                    "y": float(position.get("y", 0.0)),
                    "z": float(position.get("z", 0.0)),
                },
                "normal": {
                    "x": float(normal.get("x", 0.0)),
                    "y": float(normal.get("y", 0.0)),
                    "z": float(normal.get("z", 0.0)),
                },
                "face_index": point.get("faceIndex"),
            }
        )
    return normalized


def analyze_selected_points(source: Path, output_dir: Path, points: list[dict], radius: float) -> dict:
    output_dir.mkdir(parents=True, exist_ok=True)
    vertices, faces, bounds, topology = load_mesh_arrays(source)
    rows = []
    for point in points:
        center = np.array(
            [
                point["position"]["x"],
                point["position"]["y"],
                point["position"]["z"],
            ],
            dtype=np.float32,
        )
        normal = np.array(
            [
                point["normal"]["x"],
                point["normal"]["y"],
                point["normal"]["z"],
            ],
            dtype=np.float32,
        )
        if float(np.linalg.norm(normal)) < 1e-9:
            normal = np.array([0.0, 0.0, 1.0], dtype=np.float32)
        normal = normal / np.linalg.norm(normal)
        point_dir = output_dir / f"point_{point['index'] + 1:02d}"
        point_dir.mkdir(parents=True, exist_ok=True)
        local_vertices, local_faces, source_ids = local_sphere_mesh(vertices, faces, center, radius)
        local_path = point_dir / "local_sphere_mesh.ply"
        export_mesh_arrays(local_vertices, local_faces, local_path)
        sections = write_point_sections(point_dir, local_vertices, local_faces, center, normal)
        rows.append(
            {
                "point": point,
                "radius_source_units": radius,
                "local_faces": int(len(local_faces)),
                "source_face_ids": int(len(source_ids)),
                "local_mesh": str(local_path),
                "sections": sections,
            }
        )
    report = {
        "source": str(source),
        "bounds": bounds,
        "topology": topology,
        "radius_source_units": radius,
        "points": rows,
    }
    report_path = output_dir / "analysis.json"
    report_path.write_text(json.dumps(jsonable(report), indent=2) + "\n")
    report["report"] = str(report_path)
    return report


def local_sphere_mesh(
    vertices: np.ndarray,
    faces: np.ndarray,
    center: np.ndarray,
    radius: float,
) -> tuple[np.ndarray, np.ndarray, np.ndarray]:
    centers = vertices[faces].mean(axis=1)
    distance = np.linalg.norm(centers - center[None, :], axis=1)
    keep = distance <= radius
    source_ids = np.flatnonzero(keep)
    if len(source_ids) == 0:
        return np.empty((0, 3), dtype=np.float32), np.empty((0, 3), dtype=np.int64), source_ids
    unique, inverse = np.unique(faces[source_ids].reshape(-1), return_inverse=True)
    return vertices[unique], inverse.reshape((-1, 3)), source_ids


def write_point_sections(
    output_dir: Path,
    vertices: np.ndarray,
    faces: np.ndarray,
    center: np.ndarray,
    normal: np.ndarray,
) -> dict:
    if len(faces) == 0:
        return {}
    basis_a = stable_perpendicular(normal)
    basis_b = np.cross(normal, basis_a)
    basis_b = basis_b / np.linalg.norm(basis_b)
    specs = [
        ("surface_tangent", normal, basis_a, basis_b),
        ("normal_a", basis_a, normal, basis_b),
        ("normal_b", basis_b, basis_a, normal),
    ]
    tiles = []
    paths = {}
    for name, plane_normal, a, b in specs:
        segments = plane_intersection_segments(vertices, faces, center, plane_normal)
        image = render_segments(segments, center, a, b, 768)
        path = output_dir / f"section_{name}.png"
        image.save(path)
        paths[name] = str(path)
        tiles.append(label_tile(image, name))
    sheet = output_dir / "sections.png"
    make_contact_sheet(tiles, sheet, columns=3)
    paths["sheet"] = str(sheet)
    return paths


def stable_perpendicular(vector: np.ndarray) -> np.ndarray:
    vector = vector / np.linalg.norm(vector)
    helper = np.array([0.0, 0.0, 1.0], dtype=np.float32)
    if abs(float(vector @ helper)) > 0.9:
        helper = np.array([0.0, 1.0, 0.0], dtype=np.float32)
    out = np.cross(vector, helper)
    return out / np.linalg.norm(out)


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


def render_3d_html(preview_info: dict, default_radius: float) -> str:
    bounds = preview_info["bounds"]
    center = [
        (bounds["min"][0] + bounds["max"][0]) / 2.0,
        (bounds["min"][1] + bounds["max"][1]) / 2.0,
        (bounds["min"][2] + bounds["max"][2]) / 2.0,
    ]
    diag = float(np.linalg.norm(np.asarray(bounds["dims"], dtype=float)))
    return f"""<!doctype html>
<html>
<head>
  <meta charset="utf-8">
  <title>ResinMesh 3D ROI Picker</title>
  <style>
    html, body {{ margin: 0; height: 100%; overflow: hidden; background: #111318; color: #f2f2f2; font-family: -apple-system, BlinkMacSystemFont, sans-serif; }}
    #app {{ display: grid; grid-template-columns: 1fr 340px; height: 100%; }}
    #viewport {{ position: relative; min-width: 0; }}
    #ui {{ border-left: 1px solid #2c3038; background: #171a21; padding: 14px; overflow: auto; }}
    canvas {{ display: block; }}
    button, input {{ background: #242833; color: #f2f2f2; border: 1px solid #48505f; border-radius: 4px; padding: 7px 9px; }}
    button {{ cursor: pointer; }}
    button.primary {{ background: #275da8; border-color: #3974c3; }}
    button.warn {{ background: #5d3030; border-color: #864444; }}
    .row {{ display: flex; gap: 8px; align-items: center; margin: 10px 0; flex-wrap: wrap; }}
    .hint {{ color: #aab0bc; font-size: 13px; line-height: 1.4; }}
    .point {{ padding: 8px; border: 1px solid #303642; border-radius: 4px; margin: 8px 0; }}
    code {{ color: #9fd1ff; overflow-wrap: anywhere; }}
    #status {{ color: #aab0bc; margin-top: 8px; white-space: pre-wrap; }}
  </style>
</head>
<body>
<div id="app">
  <div id="viewport"></div>
  <aside id="ui">
    <h2>3D ROI Picker</h2>
    <div class="hint">Orbit: left drag. Pan: right drag or shift-drag. Zoom: wheel. Add point: click surface while Add mode is active.</div>
    <div class="row">
      <button id="addMode" class="primary">Add mode: on</button>
      <button id="undo">Undo</button>
      <button id="clear" class="warn">Clear</button>
    </div>
    <div class="row">
      <label>Analysis radius<br><input id="radius" type="number" step="0.001" value="{default_radius:.6f}"></label>
    </div>
    <div class="row">
      <button id="done" class="primary">Done / Analyze</button>
    </div>
    <h3>Selected points</h3>
    <div id="points"></div>
    <h3>Output</h3>
    <div id="status">Loading preview mesh...</div>
  </aside>
</div>
<script type="module">
import * as THREE from 'https://unpkg.com/three@0.165.0/build/three.module.js';
import {{ OrbitControls }} from 'https://unpkg.com/three@0.165.0/examples/jsm/controls/OrbitControls.js';
import {{ PLYLoader }} from 'https://unpkg.com/three@0.165.0/examples/jsm/loaders/PLYLoader.js';

const viewport = document.getElementById('viewport');
const statusEl = document.getElementById('status');
const pointsEl = document.getElementById('points');
const radiusInput = document.getElementById('radius');
const scene = new THREE.Scene();
scene.background = new THREE.Color(0x101216);

const camera = new THREE.PerspectiveCamera(45, 1, 0.0001, 1000);
const renderer = new THREE.WebGLRenderer({{ antialias: true }});
renderer.setPixelRatio(window.devicePixelRatio);
viewport.appendChild(renderer.domElement);

const controls = new OrbitControls(camera, renderer.domElement);
controls.enableDamping = true;
controls.screenSpacePanning = true;

scene.add(new THREE.HemisphereLight(0xffffff, 0x343847, 2.2));
const key = new THREE.DirectionalLight(0xffffff, 2.0);
key.position.set(2, -3, 4);
scene.add(key);

const center = new THREE.Vector3({center[0]}, {center[1]}, {center[2]});
const diagonal = {diag};
camera.position.copy(center).add(new THREE.Vector3(diagonal * 1.2, -diagonal * 1.6, diagonal * 0.9));
camera.near = Math.max(diagonal / 10000, 0.00001);
camera.far = diagonal * 20;
camera.updateProjectionMatrix();
controls.target.copy(center);

const raycaster = new THREE.Raycaster();
const pointer = new THREE.Vector2();
let mesh = null;
let addMode = true;
let selected = [];
let markerGroup = new THREE.Group();
scene.add(markerGroup);

function resize() {{
  const rect = viewport.getBoundingClientRect();
  renderer.setSize(rect.width, rect.height);
  camera.aspect = rect.width / rect.height;
  camera.updateProjectionMatrix();
}}
window.addEventListener('resize', resize);
resize();

new PLYLoader().load('/model.ply', geometry => {{
  geometry.computeVertexNormals();
  const material = new THREE.MeshStandardMaterial({{
    color: 0xb7b7b7,
    roughness: 0.72,
    metalness: 0.02,
    side: THREE.DoubleSide,
    polygonOffset: true,
    polygonOffsetFactor: 1,
    polygonOffsetUnits: 1
  }});
  mesh = new THREE.Mesh(geometry, material);
  scene.add(mesh);
  const wire = new THREE.LineSegments(
    new THREE.WireframeGeometry(geometry),
    new THREE.LineBasicMaterial({{ color: 0x1b1b1b, transparent: true, opacity: 0.18 }})
  );
  mesh.add(wire);
  statusEl.textContent = 'Loaded. Click surface points, then press Done / Analyze.';
}}, undefined, error => {{
  statusEl.textContent = 'Failed to load preview mesh: ' + error;
}});

function addPoint(intersection) {{
  const p = intersection.point.clone();
  const normal = intersection.face ? intersection.face.normal.clone().transformDirection(mesh.matrixWorld) : new THREE.Vector3(0, 0, 1);
  const label = `P${{selected.length + 1}}`;
  selected.push({{
    label,
    position: {{ x: p.x, y: p.y, z: p.z }},
    normal: {{ x: normal.x, y: normal.y, z: normal.z }},
    faceIndex: intersection.faceIndex
  }});
  const sphere = new THREE.Mesh(
    new THREE.SphereGeometry(diagonal * 0.012, 16, 12),
    new THREE.MeshBasicMaterial({{ color: 0xff3030 }})
  );
  sphere.position.copy(p);
  markerGroup.add(sphere);
  renderPointList();
}}

renderer.domElement.addEventListener('click', event => {{
  if (!addMode || !mesh) return;
  if (controls._dragging) return;
  const rect = renderer.domElement.getBoundingClientRect();
  pointer.x = ((event.clientX - rect.left) / rect.width) * 2 - 1;
  pointer.y = -((event.clientY - rect.top) / rect.height) * 2 + 1;
  raycaster.setFromCamera(pointer, camera);
  const hits = raycaster.intersectObject(mesh, false);
  if (hits.length) addPoint(hits[0]);
}});

function renderPointList() {{
  pointsEl.innerHTML = '';
  selected.forEach((point, index) => {{
    const div = document.createElement('div');
    div.className = 'point';
    div.innerHTML = `<strong>${{point.label}}</strong><br>
      <code>${{point.position.x.toFixed(6)}}, ${{point.position.y.toFixed(6)}}, ${{point.position.z.toFixed(6)}}</code>
      <div class="row"><button data-index="${{index}}">Remove</button></div>`;
    div.querySelector('button').addEventListener('click', () => {{
      selected.splice(index, 1);
      rebuildMarkers();
      renderPointList();
    }});
    pointsEl.appendChild(div);
  }});
}}

function rebuildMarkers() {{
  markerGroup.clear();
  selected.forEach(point => {{
    const sphere = new THREE.Mesh(
      new THREE.SphereGeometry(diagonal * 0.012, 16, 12),
      new THREE.MeshBasicMaterial({{ color: 0xff3030 }})
    );
    sphere.position.set(point.position.x, point.position.y, point.position.z);
    markerGroup.add(sphere);
  }});
}}

document.getElementById('addMode').addEventListener('click', event => {{
  addMode = !addMode;
  event.target.textContent = `Add mode: ${{addMode ? 'on' : 'off'}}`;
  event.target.className = addMode ? 'primary' : '';
}});

document.getElementById('undo').addEventListener('click', () => {{
  selected.pop();
  rebuildMarkers();
  renderPointList();
}});

document.getElementById('clear').addEventListener('click', () => {{
  selected = [];
  markerGroup.clear();
  renderPointList();
}});

document.getElementById('done').addEventListener('click', async () => {{
  statusEl.textContent = 'Analyzing selected points...';
  const response = await fetch('/done', {{
    method: 'POST',
    headers: {{ 'Content-Type': 'application/json' }},
    body: JSON.stringify({{ points: selected, radius: Number(radiusInput.value) }})
  }});
  if (!response.ok) {{
    statusEl.textContent = 'Analyze failed: ' + await response.text();
    return;
  }}
  const data = await response.json();
  const lines = [
    `Saved selection: ${{data.selection}}`,
    `Analysis report: ${{data.analysis.report}}`,
    `Points: ${{data.pointCount}}`
  ];
  for (const point of data.analysis.points || []) {{
    lines.push(`${{point.point.label}} local mesh: ${{point.local_mesh}}`);
    if (point.sections?.sheet) lines.push(`${{point.point.label}} sections: ${{point.sections.sheet}}`);
  }}
  statusEl.textContent = lines.join('\\n');
}});

function animate() {{
  controls.update();
  renderer.render(scene, camera);
  requestAnimationFrame(animate);
}}
animate();
</script>
</body>
</html>"""
