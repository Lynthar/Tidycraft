import { useRef, useEffect, useState } from "react";
import * as THREE from "three";
import { OrbitControls } from "three/addons/controls/OrbitControls.js";
import { GLTFLoader } from "three/addons/loaders/GLTFLoader.js";
import { OBJLoader } from "three/addons/loaders/OBJLoader.js";
import { FBXLoader } from "three/addons/loaders/FBXLoader.js";
import { MTLLoader } from "three/addons/loaders/MTLLoader.js";
import { convertFileSrc } from "@tauri-apps/api/core";
import { RotateCcw, Box, Maximize2 } from "lucide-react";
import { useTranslation } from "react-i18next";
import { buildTextureUrlResolver } from "../lib/modelUrlResolver";
import { dirname } from "../lib/pathUtils";

// Supported 3D model formats. `.blend` is in the list so AssetPreview
// routes the file into this component (rather than the box-icon
// fallback) — we then short-circuit to an actionable "export to GLB"
// message inside the dispatch below. Real loading is impossible: .blend
// is Blender's private binary format with no web loader.
const SUPPORTED_FORMATS = ["gltf", "glb", "fbx", "obj", "dae", "3ds", "blend", "vox"];

interface ModelViewer3DProps {
  filePath: string;
  extension: string;
  /// Backend's canonical unique-vertex count (from Rust scan metadata).
  /// When present it's shown in the footer instead of three.js's own
  /// count, which inflates for non-indexed OBJ/FBX (the loader expands
  /// vertices per-face) and so wouldn't match the preview card / analyzer.
  /// Undefined for formats the backend doesn't parse (dae/3ds/vox) — the
  /// footer then falls back to the three.js count.
  vertexCount?: number;
  onFullscreen?: () => void;
}

interface LoadingStats {
  format: string;
  vertexCount: number;
  meshCount: number;
}

// Error stored as an i18n key (+ optional fallback) rather than a
// pre-translated string, so it re-translates on a language switch
// without re-running the WebGL setup effect — which would otherwise
// tear down and rebuild the whole scene just to relabel one message.
// Rendered via t(error.key, error.fallback) in the JSX below.
interface ModelError {
  key: string;
  fallback?: string;
}

export function ModelViewer3D({ filePath, extension, vertexCount, onFullscreen }: ModelViewer3DProps) {
  const { t } = useTranslation();
  const containerRef = useRef<HTMLDivElement>(null);
  const rendererRef = useRef<THREE.WebGLRenderer | null>(null);
  const sceneRef = useRef<THREE.Scene | null>(null);
  const cameraRef = useRef<THREE.PerspectiveCamera | null>(null);
  const controlsRef = useRef<OrbitControls | null>(null);
  const animationIdRef = useRef<number>(0);
  // Monotonic token identifying the current setup-effect run. Loader
  // callbacks capture their run's value and re-check it before touching
  // any state — a shared boolean can't do this, because the next run
  // resets it to "alive" and a still-in-flight onLoad/onError from the
  // previous model then passes the guard (hijacking mixerRef, adding the
  // stale mesh to an orphaned scene, or painting "Failed to load" over a
  // successfully rendered model). cleanup() bumps it so unmount/close
  // invalidates in-flight callbacks too.
  const runIdRef = useRef(0);
  const mixerRef = useRef<THREE.AnimationMixer | null>(null);
  const clockRef = useRef<THREE.Clock>(new THREE.Clock());

  const [isLoading, setIsLoading] = useState(true);
  const [error, setError] = useState<ModelError | null>(null);
  const [stats, setStats] = useState<LoadingStats | null>(null);

  // Clean up Three.js resources
  const cleanup = () => {
    runIdRef.current++; // invalidate any in-flight loader callbacks
    if (animationIdRef.current) {
      cancelAnimationFrame(animationIdRef.current);
      animationIdRef.current = 0;
    }
    if (mixerRef.current) {
      mixerRef.current.stopAllAction();
      mixerRef.current = null;
    }
    if (controlsRef.current) {
      controlsRef.current.dispose();
      controlsRef.current = null;
    }
    if (rendererRef.current) {
      rendererRef.current.dispose();
      // dispose() frees GPU buffers but does NOT release the WebGL context;
      // browsers cap active contexts (~16/page) so without this, swapping
      // between many model previews exhausts them and the oldest get
      // force-lost by the browser ("Too many active WebGL contexts").
      rendererRef.current.forceContextLoss();
      const domElement = rendererRef.current.domElement;
      if (domElement && domElement.parentNode) {
        domElement.parentNode.removeChild(domElement);
      }
      rendererRef.current = null;
    }
    if (sceneRef.current) {
      sceneRef.current.traverse((object) => {
        if (object instanceof THREE.Mesh) {
          object.geometry?.dispose();
          if (Array.isArray(object.material)) {
            object.material.forEach((m) => m.dispose());
          } else if (object.material) {
            object.material.dispose();
          }
        }
      });
      sceneRef.current.clear();
      sceneRef.current = null;
    }
    cameraRef.current = null;
  };

  // Fix materials for models that lack proper materials
  const fixMaterials = (object: THREE.Object3D): { meshCount: number; vertexCount: number } => {
    let meshCount = 0;
    let vertexCount = 0;

    object.traverse((child) => {
      if (child instanceof THREE.Mesh) {
        meshCount++;

        // Count vertices
        if (child.geometry) {
          const posAttr = child.geometry.getAttribute("position");
          if (posAttr) {
            vertexCount += posAttr.count;
          }
        }

        // Ensure mesh has valid material
        const ensureMaterial = (mat: THREE.Material | null): THREE.Material => {
          if (!mat) {
            return new THREE.MeshStandardMaterial({
              color: 0x888888,
              metalness: 0.3,
              roughness: 0.7,
              side: THREE.DoubleSide,
            });
          }

          // Fix invisible materials - MeshBasicMaterial without texture.
          // Preserve `vertexColors` so OBJ files with inline vertex
          // colors (and any unlit GLTF using KHR_materials_unlit) keep
          // their colors after the conversion.
          if (mat instanceof THREE.MeshBasicMaterial && !mat.map) {
            return new THREE.MeshStandardMaterial({
              color: mat.color || 0x888888,
              metalness: 0.3,
              roughness: 0.7,
              side: THREE.DoubleSide,
              vertexColors: mat.vertexColors,
            });
          }

          // Convert MeshPhongMaterial (common in FBX, and what OBJLoader
          // creates for OBJs with no `mtllib`) to MeshStandardMaterial.
          // `vertexColors` is preserved because OBJLoader sets it true
          // when the OBJ has 6-value `v x y z r g b` lines — without
          // this, voxel-style OBJs render flat gray.
          if (mat instanceof THREE.MeshPhongMaterial) {
            const stdMat = new THREE.MeshStandardMaterial({
              color: mat.color || 0x888888,
              map: mat.map,
              normalMap: mat.normalMap,
              metalness: 0.3,
              roughness: 0.7,
              side: THREE.DoubleSide,
              vertexColors: mat.vertexColors,
            });
            return stdMat;
          }

          // Convert MeshLambertMaterial to MeshStandardMaterial
          if (mat instanceof THREE.MeshLambertMaterial) {
            return new THREE.MeshStandardMaterial({
              color: mat.color || 0x888888,
              map: mat.map,
              metalness: 0.1,
              roughness: 0.9,
              side: THREE.DoubleSide,
              vertexColors: mat.vertexColors,
            });
          }

          // Fix transparent materials with zero opacity
          if (mat.transparent && mat.opacity === 0) {
            mat.opacity = 1;
            mat.transparent = false;
          }

          // Show both sides
          mat.side = THREE.DoubleSide;
          mat.needsUpdate = true;

          return mat;
        };

        if (Array.isArray(child.material)) {
          child.material = child.material.map(ensureMaterial);
        } else {
          child.material = ensureMaterial(child.material);
        }

        child.castShadow = true;
        child.receiveShadow = true;
      }
    });

    return { meshCount, vertexCount };
  };

  // Setup animations if available
  const setupAnimations = (object: THREE.Object3D): THREE.AnimationMixer | null => {
    const animations = (object as THREE.Object3D & { animations?: THREE.AnimationClip[] }).animations;
    if (!animations || animations.length === 0) {
      return null;
    }

    const mixer = new THREE.AnimationMixer(object);
    const clip = animations[0];
    if (clip) {
      const action = mixer.clipAction(clip);
      action.play();
    }

    return mixer;
  };

  useEffect(() => {
    if (!containerRef.current) return;

    // Cleanup previous instance (bumps runIdRef, cutting off any loader
    // callback still in flight from the previous model).
    cleanup();
    const runId = runIdRef.current;

    setIsLoading(true);
    setError(null);

    const container = containerRef.current;
    const width = container.clientWidth || 250;
    const height = container.clientHeight || 250;

    // Create scene
    const scene = new THREE.Scene();
    scene.background = new THREE.Color(0x1a1a1a);
    sceneRef.current = scene;

    // Create camera
    const camera = new THREE.PerspectiveCamera(50, width / height, 0.1, 1000);
    camera.position.set(2, 2, 2);
    cameraRef.current = camera;

    // Create renderer with error handling
    let renderer: THREE.WebGLRenderer;
    try {
      renderer = new THREE.WebGLRenderer({ antialias: true, alpha: true });
      renderer.setSize(width, height);
      renderer.setPixelRatio(Math.min(window.devicePixelRatio, 2));
      container.appendChild(renderer.domElement);
      rendererRef.current = renderer;
    } catch (err) {
      console.error("Failed to create WebGL renderer:", err);
      setError({ key: "modelViewer.webglError", fallback: "WebGL not supported" });
      setIsLoading(false);
      return;
    }

    // Create controls
    const controls = new OrbitControls(camera, renderer.domElement);
    controls.enableDamping = true;
    controls.dampingFactor = 0.05;
    controls.enableZoom = true;
    controls.enablePan = true;
    controlsRef.current = controls;

    // Add lights
    const ambientLight = new THREE.AmbientLight(0xffffff, 0.8);
    scene.add(ambientLight);

    const directionalLight = new THREE.DirectionalLight(0xffffff, 1.0);
    directionalLight.position.set(5, 10, 7.5);
    scene.add(directionalLight);

    const directionalLight2 = new THREE.DirectionalLight(0xffffff, 0.5);
    directionalLight2.position.set(-5, -5, -5);
    scene.add(directionalLight2);

    // Add grid helper
    const gridHelper = new THREE.GridHelper(10, 10, 0x444444, 0x333333);
    scene.add(gridHelper);

    // Load model based on extension
    const modelUrl = convertFileSrc(filePath);
    const ext = extension.toLowerCase();

    // Calculate resource path for textures (model's directory converted to asset:// URL)
    const dir = dirname(filePath);
    const modelDir = dir ? `${dir}/` : "";
    const resourcePath = convertFileSrc(modelDir);

    const onLoad = (object: THREE.Object3D) => {
      if (runIdRef.current !== runId) return;

      // Fix materials and get stats
      const modelStats = fixMaterials(object);

      // Center + fit. Order matters: scale BEFORE the position offset,
      // because the resulting world transform is T·S, so a mesh ends up at
      // `position + scale * localCenter`. Translating first only happens
      // to look right when localCenter is already near the hierarchy root
      // (typical of GLTF/GLB/OBJ); FBX and DAE often place the mesh node
      // far from its root, and the previous order then drifted the model
      // off the grid by `(scale - 1) * center`.
      const box = new THREE.Box3().setFromObject(object);
      const center = box.getCenter(new THREE.Vector3());
      const size = box.getSize(new THREE.Vector3());
      const maxDim = Math.max(size.x, size.y, size.z);
      const scale = maxDim > 0 ? 2 / maxDim : 1;

      object.scale.multiplyScalar(scale);
      object.position.sub(center.multiplyScalar(scale));

      scene.add(object);

      // Setup animations if available
      const mixer = setupAnimations(object);
      if (mixer) {
        mixerRef.current = mixer;
      }

      // Set stats
      setStats({
        format: ext.toUpperCase(),
        vertexCount: modelStats.vertexCount,
        meshCount: modelStats.meshCount,
      });

      setIsLoading(false);
    };

    const onError = (err: unknown) => {
      if (runIdRef.current !== runId) return;
      console.error(`[ModelViewer3D] Failed to load ${ext.toUpperCase()} model:`, {
        filePath,
        modelUrl,
        error: err,
      });
      const message = err instanceof Error ? err.message : String(err);

      // Provide more helpful error messages
      if (message.includes("404") || message.includes("not found")) {
        setError({ key: "modelViewer.fileNotFound", fallback: "File not found" });
      } else if (
        // three.js's FBXLoader is a reverse-engineered parser that
        // doesn't cover every UV/MappingInformationType combination
        // Autodesk DCC tools emit. The failure mode is a cryptic
        // `Cannot read properties of undefined (reading 'a')` from
        // GeometryParser.parseUVs. We can't fix the parser, but we can
        // tell the user a path forward (re-export as GLB).
        ext === "fbx" &&
        (message.includes("Cannot read properties of undefined") ||
          message.includes("parseUVs"))
      ) {
        setError({ key: "modelViewer.fbxIncompatible" });
      } else if (message.includes("parse") || message.includes("invalid")) {
        setError({ key: "modelViewer.parseError", fallback: "Failed to parse model file" });
      } else {
        setError({ key: "modelViewer.loadError", fallback: "Failed to load model" });
      }
      setIsLoading(false);
    };

    if (!SUPPORTED_FORMATS.includes(ext)) {
      setError({
        key: "modelViewer.unsupportedFormat",
        fallback: `Format .${ext} not supported. Use GLTF, GLB, FBX, or OBJ.`,
      });
      setIsLoading(false);
    } else {
      // Kick off loading in an async IIFE so we can await the sibling-texture
      // scan before wiring the URL modifier. The scan is a single filesystem
      // walk of the model's directory, typically <10ms.
      (async () => {
        const urlModifier = await buildTextureUrlResolver(filePath);
        if (runIdRef.current !== runId) return;

        const loadingManager = new THREE.LoadingManager();
        loadingManager.setURLModifier(urlModifier);
        // Some Three.js loaders use resolveURL instead of the URL modifier; set both.
        loadingManager.resolveURL = urlModifier;

        try {
          if (ext === "gltf" || ext === "glb") {
            const loader = new GLTFLoader(loadingManager);
            loader.setResourcePath(resourcePath);
            loader.load(
              modelUrl,
              (gltf) => onLoad(gltf.scene),
              undefined,
              onError
            );
          } else if (ext === "obj") {
            // Pre-fetch the OBJ text so we can (a) honor the actual
            // `mtllib` filename instead of guessing `<basename>.mtl` —
            // OBJ allows arbitrary names like `mtllib materials.mtl` —
            // and (b) skip the MTL request entirely when no `mtllib`
            // line is present. The previous blind attempt at `.mtl`
            // produced a console-polluting 500 from the asset protocol
            // (the silent fallback rendered the OBJ correctly, but the
            // log noise was confusing). Using `parse(text)` avoids a
            // second fetch by OBJLoader.
            let objText: string;
            try {
              const resp = await fetch(modelUrl);
              if (!resp.ok) throw new Error(`HTTP ${resp.status}`);
              objText = await resp.text();
            } catch (err) {
              onError(err);
              return;
            }
            if (runIdRef.current !== runId) return;

            const mtllibMatch = objText.match(/^mtllib\s+(.+?)\s*$/m);
            const objLoader = new OBJLoader(loadingManager);

            const finalize = () => {
              try {
                onLoad(objLoader.parse(objText));
              } catch (parseErr) {
                onError(parseErr);
              }
            };

            if (mtllibMatch) {
              const mtlName = mtllibMatch[1].trim().replace(/\\/g, "/");
              const mtlAbs = dir ? `${dir}/${mtlName}` : mtlName;
              const mtlUrl = convertFileSrc(mtlAbs);

              const mtlLoader = new MTLLoader(loadingManager);
              mtlLoader.setResourcePath(resourcePath);
              mtlLoader.load(
                mtlUrl,
                (materials) => {
                  materials.preload();
                  objLoader.setMaterials(materials);
                  finalize();
                },
                undefined,
                () => finalize()
              );
            } else {
              finalize();
            }
          } else if (ext === "fbx") {
            const loader = new FBXLoader(loadingManager);
            loader.setResourcePath(resourcePath);
            loader.load(modelUrl, onLoad, undefined, onError);
          } else if (ext === "dae") {
            const { ColladaLoader } = await import("three/addons/loaders/ColladaLoader.js");
            if (runIdRef.current !== runId) return;
            const loader = new ColladaLoader(loadingManager);
            loader.setResourcePath(resourcePath);
            loader.load(
              modelUrl,
              (collada) => onLoad(collada.scene),
              undefined,
              onError
            );
          } else if (ext === "3ds") {
            const { TDSLoader } = await import("three/addons/loaders/TDSLoader.js");
            if (runIdRef.current !== runId) return;
            const loader = new TDSLoader(loadingManager);
            loader.setResourcePath(resourcePath);
            loader.load(modelUrl, onLoad, undefined, onError);
          } else if (ext === "vox") {
            // VOXLoader (r182) returns `{ chunks, scene }`. Modern files
            // with nTRN/nGRP/nSHP nodes populate `scene` directly; older
            // v150 single-model exports (e.g. plain MagicaVoxel saves)
            // only carry SIZE/XYZI/RGBA chunks → `scene` is null at
            // runtime even though @types/three claims Object3D. Fall
            // back to manual `buildMesh` per chunk so both shapes load.
            // VOX is self-contained (palette + voxel data, no external
            // textures), so no setResourcePath is needed; buildMesh
            // already centers the geometry and emits a vertex-color
            // MeshStandardMaterial that survives fixMaterials intact.
            const { VOXLoader, buildMesh } = await import(
              "three/addons/loaders/VOXLoader.js"
            );
            if (runIdRef.current !== runId) return;
            const loader = new VOXLoader(loadingManager);
            loader.load(
              modelUrl,
              (result) => {
                if (runIdRef.current !== runId) return;
                let root: THREE.Object3D | null = result.scene;
                if (!root) {
                  if (!result.chunks || result.chunks.length === 0) {
                    onError(new Error("Empty VOX file"));
                    return;
                  }
                  const group = new THREE.Group();
                  for (const chunk of result.chunks) {
                    group.add(buildMesh(chunk));
                  }
                  root = group;
                }
                onLoad(root);
              },
              undefined,
              onError
            );
          } else if (ext === "blend") {
            // .blend is Blender's private binary format — no web loader
            // exists. We surface a clear "export to GLB" message rather
            // than fail mysteriously or fall through to "unsupported".
            if (runIdRef.current !== runId) return;
            setError({ key: "modelViewer.blendUnsupported" });
            setIsLoading(false);
          }
        } catch (err) {
          onError(err);
        }
      })();
    }

    // Animation loop
    const animate = () => {
      if (runIdRef.current !== runId) return;
      animationIdRef.current = requestAnimationFrame(animate);

      // Update animation mixer if present
      if (mixerRef.current) {
        const delta = clockRef.current.getDelta();
        mixerRef.current.update(delta);
      }

      if (controlsRef.current) {
        controlsRef.current.update();
      }
      if (rendererRef.current && sceneRef.current && cameraRef.current) {
        rendererRef.current.render(sceneRef.current, cameraRef.current);
      }
    };
    animate();

    // Handle resize. Observe the container (not just window) so the canvas
    // also tracks react-resizable-panels divider drags — those resize the
    // panel without firing a window `resize` event. A container observer
    // covers window resizes too, since the container is responsive.
    const handleResize = () => {
      if (!containerRef.current || !cameraRef.current || !rendererRef.current) return;
      const newWidth = containerRef.current.clientWidth || 250;
      const newHeight = containerRef.current.clientHeight || 250;
      cameraRef.current.aspect = newWidth / newHeight;
      cameraRef.current.updateProjectionMatrix();
      rendererRef.current.setSize(newWidth, newHeight);
    };

    const resizeObserver = new ResizeObserver(handleResize);
    resizeObserver.observe(container);

    return () => {
      resizeObserver.disconnect();
      cleanup();
    };
  }, [filePath, extension]);

  const resetCamera = () => {
    if (cameraRef.current && controlsRef.current) {
      cameraRef.current.position.set(2, 2, 2);
      controlsRef.current.reset();
    }
  };

  return (
    <div className="w-full bg-background rounded overflow-hidden">
      <div
        ref={containerRef}
        className="w-full aspect-square relative"
      >
        {isLoading && (
          <div className="absolute inset-0 flex items-center justify-center bg-[#1a1a1a] z-10">
            <div className="text-center text-text-secondary">
              <Box size={32} className="mx-auto mb-2 animate-pulse text-[var(--accent)]" />
              <span className="text-sm">{t("modelViewer.loading", "Loading model...")}</span>
            </div>
          </div>
        )}
        {error && (
          <div className="absolute inset-0 flex items-center justify-center bg-[#1a1a1a] z-10">
            <div className="text-center text-error px-4">
              <Box size={32} className="mx-auto mb-2 opacity-50" />
              <span className="text-sm">{error.fallback ? t(error.key, error.fallback) : t(error.key)}</span>
            </div>
          </div>
        )}
      </div>
      <div className="p-2 flex items-center justify-between border-t border-border">
        <div className="text-xs text-text-secondary space-y-0.5">
          <div>{t("modelViewer.controls", "Drag to rotate • Scroll to zoom")}</div>
          {stats && (
            <div className="text-[10px] text-text-secondary/70">
              {stats.format} • {(vertexCount ?? stats.vertexCount).toLocaleString()} vertices • {stats.meshCount} meshes
            </div>
          )}
        </div>
        <div className="flex items-center gap-1">
          {onFullscreen && (
            <button
              onClick={onFullscreen}
              className="p-1 rounded hover:bg-card-bg text-text-secondary hover:text-text-primary transition-colors"
              title={t("modelViewer.fullscreen", "Fullscreen")}
            >
              <Maximize2 size={14} />
            </button>
          )}
          <button
            onClick={resetCamera}
            className="p-1 rounded hover:bg-card-bg text-text-secondary hover:text-text-primary transition-colors"
            title={t("modelViewer.reset", "Reset view")}
          >
            <RotateCcw size={14} />
          </button>
        </div>
      </div>
    </div>
  );
}
