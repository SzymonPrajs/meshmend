import * as THREE from "three";
import { OrbitControls } from "three/examples/jsm/controls/OrbitControls.js";

const FIT_PADDING = 1.35;

export function fitCameraToObject(
  camera: THREE.PerspectiveCamera,
  controls: OrbitControls,
  object: THREE.Object3D,
): void {
  const box = new THREE.Box3().setFromObject(object);
  const size = new THREE.Vector3();
  const center = new THREE.Vector3();

  box.getSize(size);
  box.getCenter(center);

  const maxSize = Math.max(size.x, size.y, size.z);
  const verticalFov = THREE.MathUtils.degToRad(camera.fov);
  const horizontalFov = 2 * Math.atan(Math.tan(verticalFov / 2) * camera.aspect);
  const fitHeightDistance = maxSize / (2 * Math.tan(verticalFov / 2));
  const fitWidthDistance = maxSize / (2 * Math.tan(horizontalFov / 2));
  const distance = FIT_PADDING * Math.max(fitHeightDistance, fitWidthDistance);

  const direction = new THREE.Vector3(0.7, 0.45, 0.9).normalize();

  camera.near = Math.max(distance / 1000, 0.0001);
  camera.far = Math.max(distance * 1000, maxSize * 100);
  camera.position.copy(center).addScaledVector(direction, distance);
  camera.lookAt(center);
  camera.updateProjectionMatrix();

  controls.target.copy(center);
  controls.minDistance = Math.max(distance / 100, 0.0001);
  controls.maxDistance = distance * 100;
  controls.update();
}
