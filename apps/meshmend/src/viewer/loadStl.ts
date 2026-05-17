import * as THREE from "three";
import { STLLoader } from "three/examples/jsm/loaders/STLLoader.js";

const STL_EXTENSION = ".stl";

export async function loadStlGeometry(file: File): Promise<THREE.BufferGeometry> {
  assertStlFile(file);

  const arrayBuffer = await file.arrayBuffer();
  const loader = new STLLoader();
  const geometry = loader.parse(arrayBuffer);

  geometry.computeVertexNormals();
  geometry.computeBoundingBox();
  geometry.computeBoundingSphere();

  return geometry;
}

function assertStlFile(file: File): void {
  if (!file.name.toLowerCase().endsWith(STL_EXTENSION)) {
    throw new Error("Only .stl files are supported.");
  }

  if (file.size === 0) {
    throw new Error("The selected STL file is empty.");
  }
}
