import { useEffect, useRef, useState } from "react";
import { invoke, isTauri } from "@tauri-apps/api/core";
import * as THREE from "three";
import { OrbitControls } from "three/examples/jsm/controls/OrbitControls.js";
import type { Wind } from "../../../api/client";
import { PARTS, type PartId, alignment, bearingOf, directionName, nearestPrincipal } from "./climate";

interface Props {
  winds: Wind[];
  selected: PartId | null;
  onSelect: (part: PartId | null) => void;
}

/** What the controls change. The render loop reads it every frame. */
interface Sim {
  wind: Wind;
  inlet: number;
  exploded: boolean;
  roofHidden: boolean;
  selected: PartId | null;
}

/** One closed solid from the geometry engine: triangles, edge lines and where it goes when taken apart. */
interface RoomPart {
  id: PartId;
  mesh: { positions: number[]; normals: number[]; edges: number[] };
  apart: [number, number, number];
  volume: number;
}

const COLOURS: Partial<Record<PartId, string>> = { floor: "#d8d3c6", roof: "#b5674d", vent: "#7d4532" };
const HOME = new THREE.Vector3(11, 9, 14);
const TARGET = new THREE.Vector3(0, 1.4, 0);

/** A horizontal unit vector pointing toward a compass bearing. North is -z and east is +x. */
function toward(bearing: number) {
  const angle = THREE.MathUtils.degToRad(bearing);
  return new THREE.Vector3(Math.sin(angle), 0, -Math.cos(angle));
}

function textSprite(text: string, color: string, scale: number) {
  const canvas = document.createElement("canvas");
  canvas.width = 256;
  canvas.height = 128;
  const context = canvas.getContext("2d");
  if (context) {
    context.font = "600 64px 'Segoe UI', sans-serif";
    context.fillStyle = color;
    context.textAlign = "center";
    context.textBaseline = "middle";
    context.fillText(text, 128, 64);
  }
  const sprite = new THREE.Sprite(new THREE.SpriteMaterial({ map: new THREE.CanvasTexture(canvas), depthTest: false }));
  sprite.scale.set(scale * 2, scale, 1);
  return sprite;
}

/** Builds the scene and returns how to refresh it after a control changes, and how to dispose of it. */
function createScene(host: HTMLElement, parts: RoomPart[], sim: Sim, onPick: (part: PartId | null) => void) {
  const renderer = new THREE.WebGLRenderer({ antialias: true });
  renderer.setPixelRatio(Math.min(window.devicePixelRatio, 2));
  host.append(renderer.domElement);
  renderer.domElement.style.display = "block";
  const scene = new THREE.Scene();
  scene.background = new THREE.Color("#f7f6f3");
  const camera = new THREE.PerspectiveCamera(38, 1, 0.1, 200);
  camera.position.copy(HOME);
  const controls = new OrbitControls(camera, renderer.domElement);
  controls.target.copy(TARGET);
  controls.maxPolarAngle = Math.PI / 2.05;
  controls.minDistance = 6;
  controls.maxDistance = 40;
  scene.add(new THREE.HemisphereLight("#ffffff", "#b9b4a8", 1.6));
  const sun = new THREE.DirectionalLight("#ffffff", 1.4);
  sun.position.set(6, 12, 4);
  scene.add(sun);

  const ground = new THREE.Mesh(new THREE.CircleGeometry(11, 64), new THREE.MeshLambertMaterial({ color: "#ecebe5" }));
  ground.rotation.x = -Math.PI / 2;
  scene.add(ground);
  for (const [name, bearing] of [["N", 0], ["E", 90], ["S", 180], ["W", 270]] as const) {
    const sprite = textSprite(name, "#6b6860", 1.5);
    sprite.position.copy(toward(bearing).multiplyScalar(10)).setY(0.6);
    scene.add(sprite);
  }

  // The building: solids built by the Rust geometry engine. Its inlet wall faces local +z, and the
  // openings are real holes through the walls, so nothing here models geometry.
  const building = new THREE.Group();
  scene.add(building);
  const meshes: THREE.Mesh<THREE.BufferGeometry, THREE.MeshLambertMaterial>[] = [];
  const edgeMaterial = new THREE.LineBasicMaterial({ color: "#3a3832" });
  for (const part of parts) {
    const geometry = new THREE.BufferGeometry();
    geometry.setAttribute("position", new THREE.Float32BufferAttribute(part.mesh.positions, 3));
    geometry.setAttribute("normal", new THREE.Float32BufferAttribute(part.mesh.normals, 3));
    const mesh = new THREE.Mesh(geometry, new THREE.MeshLambertMaterial({ color: COLOURS[part.id] ?? "#f3efe6" }));
    // Every edge drawn, as in an architectural model.
    const outline = new THREE.BufferGeometry();
    outline.setAttribute("position", new THREE.Float32BufferAttribute(part.mesh.edges, 3));
    mesh.add(new THREE.LineSegments(outline, edgeMaterial));
    mesh.userData = { part: part.id, home: new THREE.Vector3(), apart: new THREE.Vector3(...part.apart) };
    building.add(mesh);
    meshes.push(mesh);
  }
  const roof = meshes.filter((mesh) => mesh.userData.part === "roof" || mesh.userData.part === "vent");

  // The air path, drawn in the building's own frame. It illustrates the principle; nothing is simulated.
  const path = new THREE.CatmullRomCurve3([
    new THREE.Vector3(0, 1.6, 4.4), new THREE.Vector3(0, 1.6, 2), new THREE.Vector3(0, 1.5, 0),
    new THREE.Vector3(0, 2.2, -2), new THREE.Vector3(0, 2.3, -4.2),
  ]);
  const airMaterial = new THREE.MeshBasicMaterial({ color: "#2f5d50", transparent: true });
  const tube = new THREE.Mesh(new THREE.TubeGeometry(path, 40, 0.07, 8), airMaterial);
  building.add(tube);
  const puffs = Array.from({ length: 6 }, () => {
    const puff = new THREE.Mesh(new THREE.SphereGeometry(0.16, 12, 12), airMaterial);
    building.add(puff);
    return puff;
  });

  // Arrows across the site, longer for a more frequent wind.
  const arrows = new THREE.Group();
  scene.add(arrows);
  let drawnWind: Wind | null = null;
  function drawWind() {
    arrows.clear();
    const from = toward(bearingOf(sim.wind.direction));
    const direction = from.clone().negate();
    const side = new THREE.Vector3(-direction.z, 0, direction.x);
    for (const offset of [-5, -2.5, 0, 2.5, 5]) {
      const start = from.clone().multiplyScalar(9.5).addScaledVector(side, offset).setY(1.6);
      arrows.add(new THREE.ArrowHelper(direction, start, 1.5 + sim.wind.share / 12, "#3d6fb0", 0.7, 0.4));
    }
    const tag = textSprite(`${sim.wind.direction} ${sim.wind.share}%`, "#3d6fb0", 1.5);
    tag.position.copy(from.clone().multiplyScalar(8.5)).setY(3.4);
    arrows.add(tag);
    drawnWind = sim.wind;
  }

  function refresh() {
    if (drawnWind !== sim.wind) drawWind();
    building.rotation.y = THREE.MathUtils.degToRad(180 - sim.inlet);
    const facing = alignment(sim.inlet, bearingOf(sim.wind.direction));
    airMaterial.opacity = Math.max(facing, 0) ** 1.5;
    tube.visible = facing > 0.15;
    puffs.forEach((puff) => (puff.visible = tube.visible));
    roof.forEach((mesh) => (mesh.visible = !sim.roofHidden));
    meshes.forEach((mesh) => mesh.material.emissive.set(mesh.userData.part === sim.selected ? "#335d50" : "#000000"));
  }

  // A click, as opposed to a drag that turns the view, picks a part.
  const ray = new THREE.Raycaster();
  let pressed: [number, number] | null = null;
  const onDown = (event: PointerEvent) => (pressed = [event.clientX, event.clientY]);
  const onUp = (event: PointerEvent) => {
    if (!pressed || Math.hypot(event.clientX - pressed[0], event.clientY - pressed[1]) > 5) return;
    const box = renderer.domElement.getBoundingClientRect();
    const point = new THREE.Vector2(((event.clientX - box.left) / box.width) * 2 - 1, -((event.clientY - box.top) / box.height) * 2 + 1);
    ray.setFromCamera(point, camera);
    const hit = ray.intersectObjects(meshes.filter((mesh) => mesh.visible))[0];
    onPick(hit ? (hit.object.userData.part as PartId) : null);
  };
  renderer.domElement.addEventListener("pointerdown", onDown);
  renderer.domElement.addEventListener("pointerup", onUp);

  const resize = () => {
    const width = host.clientWidth, height = host.clientHeight;
    if (!width || !height) return;
    renderer.setSize(width, height, false);
    camera.aspect = width / height;
    camera.updateProjectionMatrix();
  };
  const observer = new ResizeObserver(resize);
  observer.observe(host);
  resize();
  refresh();

  const clock = new THREE.Clock();
  const goal = new THREE.Vector3();
  renderer.setAnimationLoop(() => {
    const elapsed = clock.getElapsedTime();
    puffs.forEach((puff, index) => puff.position.copy(path.getPointAt((elapsed * 0.18 + index / puffs.length) % 1)));
    for (const mesh of meshes) {
      mesh.position.lerp(goal.copy(mesh.userData.home).addScaledVector(mesh.userData.apart, sim.exploded ? 1 : 0), 0.12);
    }
    controls.update();
    renderer.render(scene, camera);
  });

  return {
    refresh,
    resetView() {
      camera.position.copy(HOME);
      controls.target.copy(TARGET);
    },
    dispose() {
      renderer.setAnimationLoop(null);
      observer.disconnect();
      controls.dispose();
      scene.traverse((object) => {
        if (object instanceof THREE.Mesh || object instanceof THREE.Sprite || object instanceof THREE.LineSegments) {
          object.geometry?.dispose();
          for (const material of [object.material].flat()) {
            (material as THREE.SpriteMaterial).map?.dispose();
            material.dispose();
          }
        }
      });
      renderer.dispose();
      renderer.domElement.remove();
    },
  };
}

export default function Model3D({ winds, selected, onSelect }: Props) {
  const prevailing = winds.reduce((best, wind) => (wind.share > best.share ? wind : best), winds[0]);
  const [wind, setWind] = useState(prevailing);
  const [inlet, setInlet] = useState(180);
  const [exploded, setExploded] = useState(false);
  const [roofHidden, setRoofHidden] = useState(false);
  const [failed, setFailed] = useState<string | null>(null);
  const [parts, setParts] = useState<RoomPart[] | null>(null);
  const host = useRef<HTMLDivElement>(null);
  const sim = useRef<Sim>({ wind, inlet, exploded, roofHidden, selected });
  const view = useRef<ReturnType<typeof createScene> | null>(null);
  const pick = useRef(onSelect);
  pick.current = onSelect;

  useEffect(() => {
    // The solids come from the geometry engine inside the desktop app.
    if (!isTauri()) {
      setFailed("The 3D model is built by the desktop app's geometry engine, so it is not available in a browser preview.");
      return;
    }
    invoke<RoomPart[]>("room_model").then(setParts, (reason) => setFailed(`The 3D model could not be built. ${String(reason)}`));
  }, []);

  useEffect(() => {
    if (!host.current || !parts) return;
    try {
      view.current = createScene(host.current, parts, sim.current, (part) => pick.current(part));
    } catch {
      setFailed("The 3D model needs graphics acceleration, which is not available here. The other figures show the same data.");
      return;
    }
    return () => {
      view.current?.dispose();
      view.current = null;
    };
  }, [parts]);

  useEffect(() => {
    Object.assign(sim.current, { wind, inlet, exploded, roofHidden, selected });
    view.current?.refresh();
  }, [wind, inlet, exploded, roofHidden, selected]);

  const facing = alignment(inlet, bearingOf(wind.direction));
  const status =
    facing > 0.92
      ? `The inlet faces the ${directionName(wind.direction)} wind, which blows ${wind.share}% of the time. The breeze can cross the room.`
      : facing > 0.15
        ? `The ${directionName(wind.direction)} wind meets the inlet at an angle, so less air enters.`
        : `The inlet is on the sheltered side for the ${directionName(wind.direction)} wind. Little air crosses the room.`;
  const toggle = "rounded-md border px-2.5 py-1 text-xs";
  const on = "border-accent bg-accent-soft text-accent";
  const off = "border-rule bg-surface hover:border-accent";

  if (failed) {
    return <p role="alert" className="text-sm text-danger">{failed}</p>;
  }

  return (
    <div>
      <div ref={host} data-testid="model-3d" aria-label="3D model of a room and the wind. Drag to rotate, scroll to zoom, click a part." role="img" className="h-[26rem] w-full overflow-hidden rounded-md border border-rule" />
      <p className="mt-1 text-xs text-muted">Drag to rotate · scroll to zoom · click a part of the building</p>

      <div className="mt-3 grid gap-x-8 gap-y-3 text-sm md:grid-cols-2">
        <div>
          <p className="mb-1 text-xs uppercase tracking-wide text-muted">Wind from the data</p>
          <div className="flex flex-wrap gap-1.5">
            {[...winds].sort((a, b) => b.share - a.share).map((item) => (
              <button key={item.direction} type="button" aria-pressed={item === wind} onClick={() => setWind(item)} className={`${toggle} ${item === wind ? on : off}`}>
                {item.direction} · {item.share}%
              </button>
            ))}
          </div>
        </div>
        <div>
          <p className="mb-1 text-xs uppercase tracking-wide text-muted">Take it apart</p>
          <div className="flex flex-wrap gap-1.5">
            <button type="button" aria-pressed={roofHidden} onClick={() => setRoofHidden(!roofHidden)} className={`${toggle} ${roofHidden ? on : off}`}>
              Hide roof
            </button>
            <button type="button" aria-pressed={exploded} onClick={() => setExploded(!exploded)} className={`${toggle} ${exploded ? on : off}`}>
              Explode
            </button>
            <button type="button" onClick={() => view.current?.resetView()} className={`${toggle} ${off}`}>
              Reset view
            </button>
          </div>
        </div>
        <div className="md:col-span-2">
          <label className="block text-xs uppercase tracking-wide text-muted" htmlFor="inlet-bearing">
            Turn the building
          </label>
          <input id="inlet-bearing" type="range" min={0} max={359} value={inlet} onChange={(event) => setInlet(Number(event.target.value))} className="w-full" />
          <p className="text-xs text-muted">
            Inlet faces {nearestPrincipal(inlet)} ({inlet}°)
          </p>
          <p role="status" className={`mt-2 rounded-md px-3 py-2 text-sm ${facing > 0.92 ? "bg-accent-soft text-accent" : "bg-danger/10 text-danger"}`}>
            {status}
          </p>
        </div>
      </div>
      <p className="sr-only">Parts: {Object.values(PARTS).map((part) => part.name).join(", ")}.</p>
    </div>
  );
}
