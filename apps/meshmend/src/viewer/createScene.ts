import * as THREE from "three";
import { OrbitControls } from "three/examples/jsm/controls/OrbitControls.js";

export interface ViewerScene {
  scene: THREE.Scene;
  camera: THREE.PerspectiveCamera;
  renderer: THREE.WebGLRenderer;
  controls: OrbitControls;
  modelGroup: THREE.Group;
  render: () => void;
  resize: () => void;
  dispose: () => void;
}

export function createScene(container: HTMLElement): ViewerScene {
  const scene = new THREE.Scene();
  scene.background = new THREE.Color(0x101215);

  const camera = new THREE.PerspectiveCamera(45, 1, 0.01, 100000);
  camera.position.set(2.5, 1.8, 3.2);

  const renderer = new THREE.WebGLRenderer({
    antialias: true,
    powerPreference: "high-performance",
  });
  renderer.setPixelRatio(Math.min(window.devicePixelRatio, 2));
  renderer.outputColorSpace = THREE.SRGBColorSpace;
  container.append(renderer.domElement);

  const controls = new OrbitControls(camera, renderer.domElement);
  controls.enableDamping = true;
  controls.dampingFactor = 0.08;
  controls.screenSpacePanning = true;
  controls.addEventListener("change", render);

  const modelGroup = new THREE.Group();
  scene.add(modelGroup);

  const grid = new THREE.GridHelper(4, 20, 0x3b4250, 0x242a32);
  grid.name = "Grid";
  scene.add(grid);

  const axes = new THREE.AxesHelper(1);
  axes.name = "Axes";
  scene.add(axes);

  scene.add(new THREE.HemisphereLight(0xffffff, 0x1d232c, 2.2));

  const keyLight = new THREE.DirectionalLight(0xffffff, 3.2);
  keyLight.position.set(3, 4, 5);
  scene.add(keyLight);

  const fillLight = new THREE.DirectionalLight(0xaebcff, 1.1);
  fillLight.position.set(-4, 2, -3);
  scene.add(fillLight);

  function animate(): void {
    controls.update();
    render();
  }

  renderer.setAnimationLoop(animate);

  function render(): void {
    renderer.render(scene, camera);
  }

  function resize(): void {
    const { clientWidth, clientHeight } = container;

    if (clientWidth === 0 || clientHeight === 0) {
      return;
    }

    camera.aspect = clientWidth / clientHeight;
    camera.updateProjectionMatrix();
    renderer.setSize(clientWidth, clientHeight, false);
    render();
  }

  function dispose(): void {
    renderer.setAnimationLoop(null);
    controls.dispose();
    renderer.dispose();
    renderer.domElement.remove();
  }

  resize();

  return {
    scene,
    camera,
    renderer,
    controls,
    modelGroup,
    render,
    resize,
    dispose,
  };
}
