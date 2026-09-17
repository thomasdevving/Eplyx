import * as THREE from 'three';
import { SVGLoader } from '/public/vendor/SVGLoader.js';
import { RoomEnvironment } from '/public/vendor/RoomEnvironment.js';

// Extrude the supplied mark itself. No substitute geometry or sphere is used.
export function mountSculpture(host) {
  if (!host) return () => {};
  const controller = new AbortController();
  const motion = matchMedia('(prefers-reduced-motion: reduce)');
  let disposed = false;
  let renderer, environment, texture, resizeObserver, visibilityObserver;
  let frame = 0;
  let visible = true;
  let draw;
  const geometries = [];
  const materials = [];
  const onMotion = () => {
    cancelAnimationFrame(frame);
    frame = 0;
    draw?.(performance.now());
  };
  const onVisibility = () => {
    if (document.hidden) { cancelAnimationFrame(frame); frame = 0; }
    else if (!frame && visible) draw?.(performance.now());
  };
  const dispose = () => {
    if (disposed) return;
    disposed = true;
    controller.abort();
    cancelAnimationFrame(frame);
    resizeObserver?.disconnect();
    visibilityObserver?.disconnect();
    motion.removeEventListener('change', onMotion);
    document.removeEventListener('visibilitychange', onVisibility);
    geometries.forEach(geometry => geometry.dispose());
    materials.forEach(material => material.dispose());
    texture?.dispose();
    environment?.dispose();
    renderer?.dispose();
    renderer?.domElement.remove();
    host.closest('.logo-core')?.classList.remove('logo-core--rendered');
  };

  (async () => {
    const response = await fetch('/public/logo.svg', { signal: controller.signal });
    if (!response.ok) throw new Error('Logo asset unavailable');
    const source = new DOMParser().parseFromString(await response.text(), 'image/svg+xml');
    // SVGLoader needs flat fills; all original path coordinates stay intact.
    source.querySelector('defs')?.remove();
    source.querySelectorAll('path').forEach(path => path.setAttribute('fill', '#ffffff'));
    const data = new SVGLoader().parse(new XMLSerializer().serializeToString(source));
    if (disposed || !host.isConnected) return;

    renderer = new THREE.WebGLRenderer({ alpha: true, antialias: true, powerPreference: 'low-power' });
    renderer.setPixelRatio(Math.min(devicePixelRatio, 1.75));
    renderer.setClearColor(0x000000, 0);
    renderer.outputColorSpace = THREE.SRGBColorSpace;
    renderer.toneMapping = THREE.ACESFilmicToneMapping;
    renderer.toneMappingExposure = .9;
    renderer.domElement.setAttribute('aria-hidden', 'true');
    renderer.domElement.addEventListener('webglcontextlost', event => {
      event.preventDefault();
      cancelAnimationFrame(frame);
      frame = 0;
      host.closest('.logo-core')?.classList.remove('logo-core--rendered');
    });
    renderer.domElement.addEventListener('webglcontextrestored', () => {
      if (!disposed) draw?.(performance.now());
    });

    const scene = new THREE.Scene();
    const camera = new THREE.OrthographicCamera(-255, 255, 255, -255, 1, 2000);
    camera.position.set(0, 0, 900);
    const studio = new RoomEnvironment();
    studio.background = new THREE.Color('#c6a9ef');
    studio.traverse(object => {
      if (object.material?.isMeshStandardMaterial) object.material.color.set('#d4b9f8');
    });
    const pmrem = new THREE.PMREMGenerator(renderer);
    environment = pmrem.fromScene(studio, .06);
    scene.environment = environment.texture;
    pmrem.dispose();
    studio.dispose();

    const key = new THREE.DirectionalLight('#fffaff', 1.65);
    key.position.set(-180, 350, 450);
    const reflection = new THREE.DirectionalLight('#b58aff', 1.35);
    reflection.position.set(350, -150, 200);
    const rim = new THREE.DirectionalLight('#ffffff', 2.8);
    rim.position.set(80, 390, -80);
    scene.add(key, reflection, rim, new THREE.HemisphereLight('#f6edff', '#743aca', .85));

    const surface = document.createElement('canvas');
    surface.width = surface.height = 128;
    const context = surface.getContext('2d');
    const gradient = context.createLinearGradient(12, 0, 106, 128);
    gradient.addColorStop(0, '#ffffff');
    gradient.addColorStop(.4, '#d8c0f7');
    gradient.addColorStop(.72, '#b68ae7');
    gradient.addColorStop(1, '#8750c9');
    context.fillStyle = gradient;
    context.fillRect(0, 0, 128, 128);
    texture = new THREE.CanvasTexture(surface);
    texture.colorSpace = THREE.SRGBColorSpace;
    const face = new THREE.MeshPhysicalMaterial({ color: '#ffffff', map: texture, metalness: .03, roughness: .23, clearcoat: .85, clearcoatRoughness: .16, envMapIntensity: .7 });
    const side = new THREE.MeshPhysicalMaterial({ color: '#5e26c9', metalness: .18, roughness: .3, clearcoat: .6, envMapIntensity: .65 });
    const bevel = new THREE.MeshPhysicalMaterial({ color: '#f3eaff', metalness: .08, roughness: .2, clearcoat: .8, envMapIntensity: .8 });
    materials.push(face, side, bevel);
    const sculpture = new THREE.Group();
    for (const path of data.paths) {
      for (const shape of path.toShapes()) {
        const geometry = new THREE.ExtrudeGeometry(shape, { depth: 44, steps: 1, bevelEnabled: true, bevelThickness: 5, bevelSize: 4, bevelSegments: 6, curveSegments: 48 });
        const positions = geometry.attributes.position;
        const normals = geometry.attributes.normal;
        const uv = geometry.attributes.uv;
        for (let i = 0; i < uv.count; i++) uv.setXY(i, positions.getX(i) / 440, 1 - positions.getY(i) / 440);
        // Keep a luminous ceramic bevel above the violet side surface.
        geometry.clearGroups();
        let start = 0, previous = -1;
        for (let i = 0; i < positions.count; i += 3) {
          const z = (normals.getZ(i) + normals.getZ(i + 1) + normals.getZ(i + 2)) / 3;
          const material = z > .97 ? 0 : z > .15 ? 2 : 1;
          if (material !== previous) {
            if (previous !== -1) geometry.addGroup(start, i - start, previous);
            start = i;
            previous = material;
          }
        }
        geometry.addGroup(start, positions.count - start, previous);
        geometry.translate(-220, -220, -22);
        geometries.push(geometry);
        const mesh = new THREE.Mesh(geometry, materials);
        // Keep the reflection on the mesh transform so Three.js also flips
        // front-face winding; baking it into vertices would show the back.
        mesh.scale.y = -1;
        sculpture.add(mesh);
      }
    }
    scene.add(sculpture);
    host.append(renderer.domElement);
    const sceneElement = host.closest('.core-scene');
    const started = performance.now();
    draw = now => {
      if (disposed) return;
      frame = 0;
      const time = (now - started) / 1000;
      const px = motion.matches ? 0 : Number(sceneElement.style.getPropertyValue('--px')) || 0;
      const py = motion.matches ? 0 : Number(sceneElement.style.getPropertyValue('--py')) || 0;
      sculpture.rotation.set(.2 + py * .06, -.38 + px * .1, -.035 + (motion.matches ? 0 : Math.sin(time * .65) * .018));
      sculpture.position.y = motion.matches ? 0 : Math.sin(time * .8) * 4;
      renderer.render(scene, camera);
      host.closest('.logo-core').classList.add('logo-core--rendered');
      if (!motion.matches && visible && !document.hidden) frame = requestAnimationFrame(draw);
    };
    resizeObserver = new ResizeObserver(() => {
      if (disposed) return;
      const { width, height } = host.getBoundingClientRect();
      if (!width || !height) return;
      renderer.setSize(width, height, false);
      if (!frame) draw(performance.now());
    });
    resizeObserver.observe(host);
    visibilityObserver = new IntersectionObserver(([entry]) => {
      visible = entry.isIntersecting;
      if (!visible) { cancelAnimationFrame(frame); frame = 0; }
      else if (!frame) draw(performance.now());
    });
    visibilityObserver.observe(host);
    motion.addEventListener('change', onMotion);
    document.addEventListener('visibilitychange', onVisibility);
    const { width, height } = host.getBoundingClientRect();
    renderer.setSize(width || 256, height || 256, false);
    draw(performance.now());
  })().catch(() => dispose());
  return dispose;
}
