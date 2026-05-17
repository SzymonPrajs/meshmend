import * as THREE from "three";

export interface MeshStats {
  fileName: string;
  fileSizeBytes: number;
  triangleCount: number;
  vertexCount: number;
  bounds: THREE.Vector3;
  center: THREE.Vector3;
}

export function getMeshStats(
  file: File,
  geometry: THREE.BufferGeometry,
): MeshStats {
  geometry.computeBoundingBox();

  const position = geometry.getAttribute("position");
  const triangleCount = geometry.index
    ? geometry.index.count / 3
    : position.count / 3;

  const box = geometry.boundingBox ?? new THREE.Box3();
  const bounds = new THREE.Vector3();
  const center = new THREE.Vector3();

  box.getSize(bounds);
  box.getCenter(center);

  return {
    fileName: file.name,
    fileSizeBytes: file.size,
    triangleCount,
    vertexCount: position.count,
    bounds,
    center,
  };
}

export function formatInteger(value: number): string {
  return Math.round(value).toLocaleString();
}

export function formatBytes(bytes: number): string {
  const units = ["B", "KB", "MB", "GB"];
  let value = bytes;
  let unitIndex = 0;

  while (value >= 1024 && unitIndex < units.length - 1) {
    value /= 1024;
    unitIndex += 1;
  }

  return `${value.toFixed(unitIndex === 0 ? 0 : 1)} ${units[unitIndex]}`;
}

export function formatVector(vector: THREE.Vector3): string {
  return `${formatNumber(vector.x)} x ${formatNumber(vector.y)} x ${formatNumber(
    vector.z,
  )}`;
}

export function formatPoint(vector: THREE.Vector3): string {
  return `${formatNumber(vector.x)}, ${formatNumber(vector.y)}, ${formatNumber(
    vector.z,
  )}`;
}

function formatNumber(value: number): string {
  const abs = Math.abs(value);

  if (abs === 0) {
    return "0";
  }

  if (abs >= 1000 || abs < 0.001) {
    return value.toExponential(3);
  }

  return value.toLocaleString(undefined, {
    maximumFractionDigits: 4,
  });
}
