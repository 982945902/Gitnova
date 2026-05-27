import { useEffect, useMemo, useRef } from "react";
import * as THREE from "three";
import type { CodeEdge, CodeNode } from "../types";
import { colorForKind, edgeColor } from "./colors";
import { layoutGraph, nodeDegree } from "./layout";
import type { GraphMode } from "./modes";
import { modeConfig } from "./modes";

interface GraphSceneProps {
  nodes: CodeNode[];
  edges: CodeEdge[];
  highlightedNodeIds: Set<string>;
  highlightedEdgeKeys: Set<string>;
  selectedNodeId: string | null;
  mode: GraphMode;
  resetSignal: number;
  onSelectNode: (node: CodeNode) => void;
}

const edgeKey = (edge: CodeEdge) => `${edge.from}->${edge.to}:${edge.kind}`;

export function GraphScene({
  nodes,
  edges,
  highlightedNodeIds,
  highlightedEdgeKeys,
  selectedNodeId,
  mode,
  resetSignal,
  onSelectNode,
}: GraphSceneProps) {
  const hostRef = useRef<HTMLDivElement | null>(null);
  const positions = useMemo(() => layoutGraph(nodes), [nodes]);
  const config = modeConfig[mode];

  useEffect(() => {
    const host = hostRef.current;
    if (!host) return;

    const scene = new THREE.Scene();
    scene.background = new THREE.Color(mode === "showcase" ? 0x060914 : 0x101827);

    const camera = new THREE.PerspectiveCamera(48, 1, 0.1, 4000);
    camera.position.set(0, 0, mode === "showcase" ? 980 : 860);
    camera.lookAt(0, 0, 0);

    const renderer = new THREE.WebGLRenderer({ antialias: true, alpha: false });
    renderer.setPixelRatio(Math.min(window.devicePixelRatio || 1, 2));
    renderer.domElement.className = "graph-canvas";
    host.appendChild(renderer.domElement);

    const nodeByMesh = new Map<THREE.Object3D, CodeNode>();
    const meshGroup = new THREE.Group();
    scene.add(meshGroup);
    scene.add(new THREE.AmbientLight(0xffffff, mode === "showcase" ? 0.72 : 0.58));
    const keyLight = new THREE.PointLight(0x9edcff, mode === "showcase" ? 1.2 : 0.7, 2200);
    keyLight.position.set(260, 320, 680);
    scene.add(keyLight);
    if (config.particles) {
      scene.add(createStarfield(nodes.length));
    }

    const edgeMaterialCache = new Map<string, THREE.LineBasicMaterial>();
    const materialForEdge = (edge: CodeEdge) => {
      const highlighted = highlightedEdgeKeys.has(edgeKey(edge));
      const key = `${edge.kind}:${highlighted}`;
      const cached = edgeMaterialCache.get(key);
      if (cached) return cached;
      const material = new THREE.LineBasicMaterial({
        color: edgeColor(edge.kind, highlighted),
        transparent: true,
        opacity: highlighted ? 0.92 : edge.kind === "calls" ? 0.42 : 0.24,
      });
      edgeMaterialCache.set(key, material);
      return material;
    };

    for (const edge of edges) {
      const from = positions.get(edge.from);
      const to = positions.get(edge.to);
      if (!from || !to) continue;
      const geometry = new THREE.BufferGeometry().setFromPoints([
        new THREE.Vector3(from.x, from.y, from.z * config.depthScale - 4),
        new THREE.Vector3(to.x, to.y, to.z * config.depthScale - 4),
      ]);
      meshGroup.add(new THREE.Line(geometry, materialForEdge(edge)));
    }

    for (const node of nodes) {
      const point = positions.get(node.id);
      if (!point) continue;
      const highlighted = highlightedNodeIds.has(node.id);
      const selected = selectedNodeId === node.id;
      const radius = Math.max(7, Math.min(20, 7 + nodeDegree(node))) + (highlighted ? 4 : 0);
      const geometry = new THREE.SphereGeometry(radius, 24, 16);
      const material = new THREE.MeshStandardMaterial({
        color: selected ? 0xffffff : colorForKind(node.kind),
        emissive: new THREE.Color(highlighted || selected || mode === "showcase" ? colorForKind(node.kind) : 0x000000),
        emissiveIntensity: selected ? 0.55 : highlighted ? 0.36 : mode === "showcase" ? 0.16 : 0.02,
        roughness: 0.38,
        metalness: mode === "showcase" ? 0.18 : 0.04,
        transparent: true,
        opacity: mode === "showcase" && !highlighted && !selected ? 0.82 : 1,
      });
      const mesh = new THREE.Mesh(geometry, material);
      mesh.position.set(point.x, point.y, point.z * config.depthScale);
      mesh.userData.nodeId = node.id;
      mesh.userData.baseScale = 1;
      nodeByMesh.set(mesh, node);
      meshGroup.add(mesh);

      if (selected || highlighted) {
        const ring = new THREE.Mesh(
          new THREE.TorusGeometry(radius + 7, mode === "showcase" ? 1.9 : 1.2, 8, 48),
          new THREE.MeshBasicMaterial({
            color: selected ? 0xffffff : 0xff5d7a,
            transparent: true,
            opacity: mode === "showcase" ? 0.92 : 0.72,
            blending: mode === "showcase" ? THREE.AdditiveBlending : THREE.NormalBlending,
          }),
        );
        ring.position.copy(mesh.position);
        ring.userData.baseScale = 1;
        nodeByMesh.set(ring, node);
        meshGroup.add(ring);
      }
    }

    const resize = () => {
      const rect = host.getBoundingClientRect();
      const width = Math.max(360, Math.floor(rect.width));
      const height = Math.max(320, Math.floor(rect.height));
      renderer.setSize(width, height, false);
      camera.aspect = width / height;
      camera.updateProjectionMatrix();
      renderer.render(scene, camera);
    };

    const raycaster = new THREE.Raycaster();
    const pointer = new THREE.Vector2();
    let dragging = false;
    let dragMoved = false;
    let lastPointer: { x: number; y: number } | null = null;

    const setPointer = (event: PointerEvent) => {
      const rect = renderer.domElement.getBoundingClientRect();
      pointer.x = ((event.clientX - rect.left) / rect.width) * 2 - 1;
      pointer.y = -((event.clientY - rect.top) / rect.height) * 2 + 1;
    };

    const onPointerDown = (event: PointerEvent) => {
      dragging = true;
      dragMoved = false;
      lastPointer = { x: event.clientX, y: event.clientY };
      renderer.domElement.setPointerCapture(event.pointerId);
    };

    const onPointerMove = (event: PointerEvent) => {
      if (!dragging || !lastPointer) return;
      const dx = event.clientX - lastPointer.x;
      const dy = event.clientY - lastPointer.y;
      if (Math.abs(dx) + Math.abs(dy) > 3) dragMoved = true;
      meshGroup.rotation.y += dx * 0.006;
      meshGroup.rotation.x += dy * 0.004;
      meshGroup.rotation.x = Math.max(-0.95, Math.min(0.95, meshGroup.rotation.x));
      lastPointer = { x: event.clientX, y: event.clientY };
      renderer.render(scene, camera);
    };

    const onPointerUp = (event: PointerEvent) => {
      if (!dragMoved) {
        setPointer(event);
        raycaster.setFromCamera(pointer, camera);
        const hit = raycaster.intersectObjects(meshGroup.children, false)[0];
        if (hit) {
          const node = nodeByMesh.get(hit.object);
          if (node) onSelectNode(node);
        }
      }
      dragging = false;
      lastPointer = null;
      renderer.domElement.releasePointerCapture(event.pointerId);
    };

    const onWheel = (event: WheelEvent) => {
      event.preventDefault();
      camera.position.z = Math.max(
        260,
        Math.min(1600, camera.position.z + (event.deltaY > 0 ? 72 : -72)),
      );
      renderer.render(scene, camera);
    };

    const onDoubleClick = () => {
      resetCamera(camera, meshGroup, mode);
      renderer.render(scene, camera);
    };

    let animationFrame = 0;
    const startedAt = performance.now();
    const animate = (now: number) => {
      const elapsed = (now - startedAt) / 1000;
      if (config.autoRotate && !dragging) {
        meshGroup.rotation.y += 0.0028;
        meshGroup.rotation.x = Math.sin(elapsed * 0.35) * 0.12;
      }
      if (mode === "showcase") {
        for (const child of meshGroup.children) {
          const base = child.userData.baseScale;
          if (typeof base === "number") {
            const pulse = 1 + Math.sin(elapsed * 1.8 + child.position.x * 0.01) * 0.045;
            child.scale.setScalar(base * pulse);
          }
        }
      }
      renderer.render(scene, camera);
      animationFrame = requestAnimationFrame(animate);
    };

    resize();
    window.addEventListener("resize", resize);
    renderer.domElement.addEventListener("pointerdown", onPointerDown);
    renderer.domElement.addEventListener("pointermove", onPointerMove);
    renderer.domElement.addEventListener("pointerup", onPointerUp);
    renderer.domElement.addEventListener("wheel", onWheel, { passive: false });
    renderer.domElement.addEventListener("dblclick", onDoubleClick);
    resetCamera(camera, meshGroup, mode);
    renderer.render(scene, camera);
    animationFrame = requestAnimationFrame(animate);

    return () => {
      cancelAnimationFrame(animationFrame);
      window.removeEventListener("resize", resize);
      renderer.domElement.removeEventListener("pointerdown", onPointerDown);
      renderer.domElement.removeEventListener("pointermove", onPointerMove);
      renderer.domElement.removeEventListener("pointerup", onPointerUp);
      renderer.domElement.removeEventListener("wheel", onWheel);
      renderer.domElement.removeEventListener("dblclick", onDoubleClick);
      host.removeChild(renderer.domElement);
      scene.traverse((object) => {
        const mesh = object as THREE.Mesh;
        mesh.geometry?.dispose();
        const material = mesh.material as THREE.Material | THREE.Material[] | undefined;
        if (Array.isArray(material)) {
          material.forEach((item) => item.dispose());
        } else {
          material?.dispose();
        }
      });
      edgeMaterialCache.forEach((material) => material.dispose());
      renderer.dispose();
    };
  }, [config, edges, highlightedEdgeKeys, highlightedNodeIds, mode, nodes, onSelectNode, positions, resetSignal, selectedNodeId]);

  return (
    <div className="graph-scene" ref={hostRef}>
      {nodes.length === 0 ? <div className="graph-empty">No graph nodes match the filters.</div> : null}
    </div>
  );
}

function resetCamera(
  camera: THREE.PerspectiveCamera,
  meshGroup: THREE.Group,
  mode: GraphMode,
) {
  camera.position.set(0, 0, mode === "showcase" ? 980 : 860);
  camera.lookAt(0, 0, 0);
  meshGroup.rotation.set(mode === "showcase" ? -0.12 : -0.08, mode === "showcase" ? 0.36 : 0.18, 0);
}

function createStarfield(nodeCount: number): THREE.Points {
  const count = Math.max(120, Math.min(460, nodeCount * 18));
  const data: number[] = [];
  for (let index = 0; index < count; index += 1) {
    const angle = index * 2.399963229728653;
    const radius = 420 + (index % 31) * 18;
    data.push(
      Math.cos(angle) * radius,
      Math.sin(angle) * radius * 0.74,
      -620 + (index % 53) * 24,
    );
  }
  const geometry = new THREE.BufferGeometry();
  geometry.setAttribute("position", new THREE.Float32BufferAttribute(data, 3));
  const material = new THREE.PointsMaterial({
    color: 0x8fb8ff,
    transparent: true,
    opacity: 0.38,
    size: 2.2,
    sizeAttenuation: true,
    blending: THREE.AdditiveBlending,
  });
  return new THREE.Points(geometry, material);
}
