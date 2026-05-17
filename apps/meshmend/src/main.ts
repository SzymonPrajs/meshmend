import * as THREE from "three";
import "./styles.css";
import { createScene } from "./viewer/createScene";
import { fitCameraToObject } from "./viewer/fitCamera";
import { loadStlGeometry } from "./viewer/loadStl";
import {
  formatBytes,
  formatInteger,
  formatPoint,
  formatVector,
  getMeshStats,
  type MeshStats,
} from "./viewer/stats";

const viewerEl = getElement<HTMLElement>("viewer");
const fileInput = getElement<HTMLInputElement>("stl-file");
const dropZone = getElement<HTMLElement>("drop-zone");
const statusEl = getElement<HTMLElement>("status");
const fitButton = getElement<HTMLButtonElement>("fit-view");
const resetButton = getElement<HTMLButtonElement>("reset-view");
const wireframeToggle = getElement<HTMLInputElement>("wireframe-toggle");
const backfaceToggle = getElement<HTMLInputElement>("backface-toggle");
const opacitySlider = getElement<HTMLInputElement>("opacity-slider");

const statFile = getElement<HTMLElement>("stat-file");
const statTriangles = getElement<HTMLElement>("stat-triangles");
const statVertices = getElement<HTMLElement>("stat-vertices");
const statBounds = getElement<HTMLElement>("stat-bounds");
const statCenter = getElement<HTMLElement>("stat-center");
const statSize = getElement<HTMLElement>("stat-size");

const viewer = createScene(viewerEl);
const material = new THREE.MeshStandardMaterial({
  color: 0xb6bcc4,
  metalness: 0.05,
  roughness: 0.58,
  side: THREE.FrontSide,
});

let currentMesh: THREE.Mesh<THREE.BufferGeometry, THREE.MeshStandardMaterial> | null =
  null;

window.addEventListener("resize", viewer.resize);

fileInput.addEventListener("change", () => {
  const file = fileInput.files?.[0];

  if (file) {
    void openStlFile(file);
  }
});

fitButton.addEventListener("click", () => fitCurrentMesh());
resetButton.addEventListener("click", () => fitCurrentMesh());

wireframeToggle.addEventListener("change", () => {
  material.wireframe = wireframeToggle.checked;
  viewer.render();
});

backfaceToggle.addEventListener("change", () => {
  material.side = backfaceToggle.checked ? THREE.DoubleSide : THREE.FrontSide;
  material.needsUpdate = true;
  viewer.render();
});

opacitySlider.addEventListener("input", () => {
  const opacity = Number(opacitySlider.value);
  material.opacity = opacity;
  material.transparent = opacity < 1;
  material.depthWrite = opacity >= 1;
  material.needsUpdate = true;
  viewer.render();
});

viewerEl.addEventListener("dragenter", handleDragEnter);
viewerEl.addEventListener("dragover", handleDragOver);
viewerEl.addEventListener("dragleave", handleDragLeave);
viewerEl.addEventListener("drop", handleDrop);

viewer.resize();

async function openStlFile(file: File): Promise<void> {
  try {
    showStatus(`Loading ${file.name} (${formatBytes(file.size)})...`);
    await nextFrame();

    const geometry = await loadStlGeometry(file);
    const mesh = new THREE.Mesh(geometry, material);
    const stats = getMeshStats(file, geometry);

    replaceMesh(mesh);
    updateStats(stats);
    setViewerReady(true);
    fitCurrentMesh();
    showStatus(`Loaded ${file.name}`);
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    showStatus(message, true);
  } finally {
    fileInput.value = "";
  }
}

function replaceMesh(
  mesh: THREE.Mesh<THREE.BufferGeometry, THREE.MeshStandardMaterial>,
): void {
  if (currentMesh) {
    viewer.modelGroup.remove(currentMesh);
    currentMesh.geometry.dispose();
  }

  currentMesh = mesh;
  viewer.modelGroup.add(mesh);
  dropZone.classList.add("is-hidden");
}

function fitCurrentMesh(): void {
  if (!currentMesh) {
    return;
  }

  fitCameraToObject(viewer.camera, viewer.controls, currentMesh);
  viewer.render();
}

function updateStats(stats: MeshStats): void {
  statFile.textContent = stats.fileName;
  statTriangles.textContent = formatInteger(stats.triangleCount);
  statVertices.textContent = formatInteger(stats.vertexCount);
  statBounds.textContent = formatVector(stats.bounds);
  statCenter.textContent = formatPoint(stats.center);
  statSize.textContent = formatBytes(stats.fileSizeBytes);
}

function setViewerReady(isReady: boolean): void {
  fitButton.disabled = !isReady;
  resetButton.disabled = !isReady;
}

function showStatus(message: string, isError = false): void {
  statusEl.textContent = message;
  statusEl.hidden = false;
  statusEl.classList.toggle("is-error", isError);

  if (!isError) {
    window.setTimeout(() => {
      if (statusEl.textContent === message) {
        statusEl.hidden = true;
      }
    }, 3200);
  }
}

function handleDragEnter(event: DragEvent): void {
  event.preventDefault();
  dropZone.classList.add("is-dragging");
}

function handleDragOver(event: DragEvent): void {
  event.preventDefault();
}

function handleDragLeave(event: DragEvent): void {
  if (event.target === viewerEl) {
    dropZone.classList.remove("is-dragging");
  }
}

function handleDrop(event: DragEvent): void {
  event.preventDefault();
  dropZone.classList.remove("is-dragging");

  const file = event.dataTransfer?.files[0];

  if (file) {
    void openStlFile(file);
  }
}

function getElement<T extends HTMLElement>(id: string): T {
  const element = document.getElementById(id);

  if (!element) {
    throw new Error(`Missing element #${id}`);
  }

  return element as T;
}

function nextFrame(): Promise<void> {
  return new Promise((resolve) => {
    requestAnimationFrame(() => resolve());
  });
}
