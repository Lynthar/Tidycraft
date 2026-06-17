import { useRef, useEffect, useState, useCallback } from "react";
import * as THREE from "three";
import { OrbitControls } from "three/addons/controls/OrbitControls.js";
import { GLTFLoader } from "three/addons/loaders/GLTFLoader.js";
import { OBJLoader } from "three/addons/loaders/OBJLoader.js";
import { FBXLoader } from "three/addons/loaders/FBXLoader.js";
import { MTLLoader } from "three/addons/loaders/MTLLoader.js";
import { convertFileSrc } from "@tauri-apps/api/core";
import { X, RotateCcw, Box, Grid3X3, Sun, Moon } from "lucide-react";
import { useTranslation } from "react-i18next";
import { buildTextureUrlResolver } from "../lib/modelUrlResolver";
import { dirname } from "../lib/pathUtils";

// Mirrors ModelViewer3D's list — see that file for why `.blend` is in
// here (it routes the user into a clear error message rather than a
// silent placeholder).
const SUPPORTED_FORMATS = ["gltf", "glb", "fbx", "obj", "dae", "3ds", "blend", "vox"];

interface ModelLightboxProps {
  isOpen: boolean;
  filePath: string;
  extension: string;
  modelName: string;
  onClose: () => void;
}

interface LoadingStats {
  format: string;
  vertexCount: number;
  meshCount: number;
}

export function ModelLightbox({ isOpen, filePath, extension, modelName, onClose }: ModelLightboxProps) {
  const { t } = useTranslation();
  const containerRef = useRef<HTMLDivElement>(null);
  const rendererRef = useRef<THREE.WebGLRenderer | null>(null);
  const sceneRef = useRef<THREE.Scene | null>(null);
  const cameraRef = useRef<THREE.PerspectiveCamera | null>(null);
  const controlsRef = useRef<OrbitControls | null>(null);
  const animationIdRef = useRef<number>(0);
  const isMountedRef = useRef(true);
  const mixerRef = useRef<THREE.AnimationMixer | null>(null);
  const clockRef = useRef<THREE.Clock>(new THREE.Clock());
  const gridRef = useRef<THREE.GridHelper | null>(null);
  const lightsRef = useRef<THREE.Light[]>([]);

  const [isLoading, setIsLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [stats, setStats] = useState<LoadingStats | null>(null);
  const [showGrid, setShowGrid] = useState(true);
  const [darkMode, setDarkMode] = useState(true);

  // Clean up Three.js resources
  const cleanup = useCallback(() => {
    isMountedRef.current = false;
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
      // browsers cap active contexts (~16/page), so without this, repeatedly
      // opening/closing the lightbox exhausts them and the browser force-loses
      // the oldest ("Too many active WebGL contexts") → black preview.
      // Mirrors ModelViewer3D's cleanup.
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
    gridRef.current = null;
    lightsRef.current = [];
  }, []);

  // Fix materials for models
  const fixMaterials = (object: THREE.Object3D): { meshCount: number; vertexCount: number } => {
    let meshCount = 0;
    let vertexCount = 0;

    object.traverse((child) => {
      if (child instanceof THREE.Mesh) {
        meshCount++;

        if (child.geometry) {
          const posAttr = child.geometry.getAttribute("position");
          if (posAttr) {
            vertexCount += posAttr.count;
          }
        }

        const ensureMaterial = (mat: THREE.Material | null): THREE.Material => {
          if (!mat) {
            return new THREE.MeshStandardMaterial({
              color: 0x888888,
              metalness: 0.3,
              roughness: 0.7,
              side: THREE.DoubleSide,
            });
          }

          // See ModelViewer3D.fixMaterials for why `vertexColors` is
          // preserved across these conversions — OBJLoader sets it true
          // for OBJs with inline `v x y z r g b` lines and we'd
          // otherwise render those (and unlit GLTFs) flat gray.
          if (mat instanceof THREE.MeshBasicMaterial && !mat.map) {
            return new THREE.MeshStandardMaterial({
              color: mat.color || 0x888888,
              metalness: 0.3,
              roughness: 0.7,
              side: THREE.DoubleSide,
              vertexColors: mat.vertexColors,
            });
          }

          if (mat instanceof THREE.MeshPhongMaterial) {
            return new THREE.MeshStandardMaterial({
              color: mat.color || 0x888888,
              map: mat.map,
              normalMap: mat.normalMap,
              metalness: 0.3,
              roughness: 0.7,
              side: THREE.DoubleSide,
              vertexColors: mat.vertexColors,
            });
          }

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

          if (mat.transparent && mat.opacity === 0) {
            mat.opacity = 1;
            mat.transparent = false;
          }

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

  // Setup animations
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

  // Handle keyboard events
  useEffect(() => {
    if (!isOpen) return;

    const handleKeyDown = (e: KeyboardEvent) => {
      switch (e.key) {
        case "Escape":
          onClose();
          break;
        case "g":
        case "G":
          setShowGrid((prev) => !prev);
          break;
        case "l":
        case "L":
          setDarkMode((prev) => !prev);
          break;
        case "r":
        case "R":
          resetCamera();
          break;
      }
    };

    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [isOpen, onClose]);

  // Toggle grid visibility
  useEffect(() => {
    if (gridRef.current) {
      gridRef.current.visible = showGrid;
    }
  }, [showGrid]);

  // Toggle dark/light mode
  useEffect(() => {
    if (sceneRef.current) {
      sceneRef.current.background = new THREE.Color(darkMode ? 0x1a1a1a : 0xf0f0f0);
    }
    if (gridRef.current) {
      gridRef.current.material.opacity = darkMode ? 0.3 : 0.5;
    }
  }, [darkMode]);

  useEffect(() => {
    if (!isOpen) {
      cleanup();
      return;
    }

    isMountedRef.current = true;

    if (!containerRef.current) return;

    cleanup();
    isMountedRef.current = true;

    setIsLoading(true);
    setError(null);

    const container = containerRef.current;
    const width = container.clientWidth || window.innerWidth;
    const height = container.clientHeight || window.innerHeight - 120;

    // Create scene
    const scene = new THREE.Scene();
    scene.background = new THREE.Color(darkMode ? 0x1a1a1a : 0xf0f0f0);
    sceneRef.current = scene;

    // Create camera
    const camera = new THREE.PerspectiveCamera(50, width / height, 0.1, 1000);
    camera.position.set(3, 3, 3);
    cameraRef.current = camera;

    // Create renderer
    let renderer: THREE.WebGLRenderer;
    try {
      renderer = new THREE.WebGLRenderer({ antialias: true, alpha: true });
      renderer.setSize(width, height);
      renderer.setPixelRatio(Math.min(window.devicePixelRatio, 2));
      renderer.shadowMap.enabled = true;
      renderer.shadowMap.type = THREE.PCFSoftShadowMap;
      container.appendChild(renderer.domElement);
      rendererRef.current = renderer;
    } catch (err) {
      console.error("Failed to create WebGL renderer:", err);
      setError(t("modelViewer.webglError", "WebGL not supported"));
      setIsLoading(false);
      return;
    }

    // Create controls
    const controls = new OrbitControls(camera, renderer.domElement);
    controls.enableDamping = true;
    controls.dampingFactor = 0.05;
    controls.enableZoom = true;
    controls.enablePan = true;
    controls.minDistance = 0.5;
    controls.maxDistance = 50;
    controlsRef.current = controls;

    // Add lights
    const ambientLight = new THREE.AmbientLight(0xffffff, 0.6);
    scene.add(ambientLight);
    lightsRef.current.push(ambientLight);

    const directionalLight = new THREE.DirectionalLight(0xffffff, 1.2);
    directionalLight.position.set(5, 10, 7.5);
    directionalLight.castShadow = true;
    directionalLight.shadow.mapSize.width = 2048;
    directionalLight.shadow.mapSize.height = 2048;
    scene.add(directionalLight);
    lightsRef.current.push(directionalLight);

    const directionalLight2 = new THREE.DirectionalLight(0xffffff, 0.4);
    directionalLight2.position.set(-5, -5, -5);
    scene.add(directionalLight2);
    lightsRef.current.push(directionalLight2);

    const fillLight = new THREE.DirectionalLight(0xffffff, 0.3);
    fillLight.position.set(0, -5, 0);
    scene.add(fillLight);
    lightsRef.current.push(fillLight);

    // Add grid helper
    const gridHelper = new THREE.GridHelper(20, 20, 0x444444, 0x333333);
    gridHelper.material.opacity = darkMode ? 0.3 : 0.5;
    gridHelper.material.transparent = true;
    gridHelper.visible = showGrid;
    scene.add(gridHelper);
    gridRef.current = gridHelper;

    // Load model
    const modelUrl = convertFileSrc(filePath);
    const ext = extension.toLowerCase();
    const dir = dirname(filePath);
    const modelDir = dir ? `${dir}/` : "";
    const resourcePath = convertFileSrc(modelDir);

    const onLoad = (object: THREE.Object3D) => {
      if (!isMountedRef.current) return;

      const modelStats = fixMaterials(object);

      // Center + fit. Order matters: scale BEFORE the position offset —
      // see the long-form comment in ModelViewer3D for the math. Without
      // this order, FBX/DAE/OBJ models whose mesh sits far from its
      // local origin (e.g. voxel exports with verts at y≈57) drift off
      // the grid by `(scale - 1) * center`.
      const box = new THREE.Box3().setFromObject(object);
      const center = box.getCenter(new THREE.Vector3());
      const size = box.getSize(new THREE.Vector3());
      const maxDim = Math.max(size.x, size.y, size.z);
      const scale = maxDim > 0 ? 3 / maxDim : 1;

      object.scale.multiplyScalar(scale);
      object.position.sub(center.multiplyScalar(scale));

      scene.add(object);

      // Setup animations
      const mixer = setupAnimations(object);
      if (mixer) {
        mixerRef.current = mixer;
      }

      setStats({
        format: ext.toUpperCase(),
        vertexCount: modelStats.vertexCount,
        meshCount: modelStats.meshCount,
      });

      setIsLoading(false);
    };

    const onError = (err: unknown) => {
      if (!isMountedRef.current) return;
      console.error("Failed to load model:", err);
      const message = err instanceof Error ? err.message : String(err);

      if (message.includes("404") || message.includes("not found")) {
        setError(t("modelViewer.fileNotFound", "File not found"));
      } else if (
        ext === "fbx" &&
        (message.includes("Cannot read properties of undefined") ||
          message.includes("parseUVs"))
      ) {
        setError(t("modelViewer.fbxIncompatible"));
      } else if (message.includes("parse") || message.includes("invalid")) {
        setError(t("modelViewer.parseError", "Failed to parse model file"));
      } else {
        setError(t("modelViewer.loadError", "Failed to load model"));
      }
      setIsLoading(false);
    };

    if (!SUPPORTED_FORMATS.includes(ext)) {
      setError(t("modelViewer.unsupportedFormat", `Format .${ext} not supported`));
      setIsLoading(false);
    } else {
      (async () => {
        const urlModifier = await buildTextureUrlResolver(filePath);
        if (!isMountedRef.current) return;

        const loadingManager = new THREE.LoadingManager();
        loadingManager.setURLModifier(urlModifier);
        loadingManager.resolveURL = urlModifier;

        try {
          if (ext === "gltf" || ext === "glb") {
            const loader = new GLTFLoader(loadingManager);
            loader.setResourcePath(resourcePath);
            loader.load(modelUrl, (gltf) => onLoad(gltf.scene), undefined, onError);
          } else if (ext === "obj") {
            // See ModelViewer3D for the full rationale — same idea:
            // pre-fetch the OBJ to honor its actual `mtllib` reference
            // and to skip MTL entirely (no spurious 500) when none is
            // declared.
            let objText: string;
            try {
              const resp = await fetch(modelUrl);
              if (!resp.ok) throw new Error(`HTTP ${resp.status}`);
              objText = await resp.text();
            } catch (err) {
              onError(err);
              return;
            }
            if (!isMountedRef.current) return;

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
            if (!isMountedRef.current) return;
            const loader = new ColladaLoader(loadingManager);
            loader.setResourcePath(resourcePath);
            loader.load(modelUrl, (collada) => onLoad(collada.scene), undefined, onError);
          } else if (ext === "3ds") {
            const { TDSLoader } = await import("three/addons/loaders/TDSLoader.js");
            if (!isMountedRef.current) return;
            const loader = new TDSLoader(loadingManager);
            loader.setResourcePath(resourcePath);
            loader.load(modelUrl, onLoad, undefined, onError);
          } else if (ext === "vox") {
            // See ModelViewer3D for the full rationale — r182 VOXLoader
            // returns `{ chunks, scene }` and v150 files have a null
            // scene that we must rebuild from chunks via `buildMesh`.
            const { VOXLoader, buildMesh } = await import(
              "three/addons/loaders/VOXLoader.js"
            );
            if (!isMountedRef.current) return;
            const loader = new VOXLoader(loadingManager);
            loader.load(
              modelUrl,
              (result) => {
                if (!isMountedRef.current) return;
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
            if (!isMountedRef.current) return;
            setError(t("modelViewer.blendUnsupported"));
            setIsLoading(false);
          }
        } catch (err) {
          onError(err);
        }
      })();
    }

    // Animation loop
    const animate = () => {
      if (!isMountedRef.current) return;
      animationIdRef.current = requestAnimationFrame(animate);

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

    // Handle resize
    const handleResize = () => {
      if (!containerRef.current || !cameraRef.current || !rendererRef.current) return;
      const newWidth = containerRef.current.clientWidth || window.innerWidth;
      const newHeight = containerRef.current.clientHeight || window.innerHeight - 120;
      cameraRef.current.aspect = newWidth / newHeight;
      cameraRef.current.updateProjectionMatrix();
      rendererRef.current.setSize(newWidth, newHeight);
    };

    window.addEventListener("resize", handleResize);

    return () => {
      window.removeEventListener("resize", handleResize);
      cleanup();
    };
  }, [isOpen, filePath, extension, t, cleanup, darkMode, showGrid]);

  const resetCamera = () => {
    if (cameraRef.current && controlsRef.current) {
      cameraRef.current.position.set(3, 3, 3);
      controlsRef.current.reset();
    }
  };

  if (!isOpen) return null;

  return (
    <div className="fixed inset-0 z-50 bg-black/95 flex flex-col">
      {/* Header */}
      <div className="flex items-center justify-between px-4 py-3 bg-black/50 text-white">
        <span className="text-sm font-medium truncate flex-1">{modelName}</span>
        <div className="flex items-center gap-1">
          {stats && (
            <span className="text-xs text-white/60 mr-4">
              {stats.format} • {(stats.vertexCount / 1000).toFixed(1)}K vertices • {stats.meshCount} meshes
            </span>
          )}
          <button
            onClick={() => setShowGrid(!showGrid)}
            className={`p-2 rounded transition-colors ${showGrid ? 'bg-white/20' : 'hover:bg-white/10'}`}
            title="Toggle grid (G)"
          >
            <Grid3X3 size={18} />
          </button>
          <button
            onClick={() => setDarkMode(!darkMode)}
            className="p-2 rounded hover:bg-white/10 transition-colors"
            title="Toggle background (L)"
          >
            {darkMode ? <Sun size={18} /> : <Moon size={18} />}
          </button>
          <button
            onClick={resetCamera}
            className="p-2 rounded hover:bg-white/10 transition-colors"
            title="Reset view (R)"
          >
            <RotateCcw size={18} />
          </button>
          <button
            onClick={onClose}
            className="p-2 rounded hover:bg-white/10 transition-colors ml-2"
            title="Close (Esc)"
          >
            <X size={18} />
          </button>
        </div>
      </div>

      {/* 3D Viewer Container */}
      <div
        ref={containerRef}
        className="flex-1 overflow-hidden relative"
      >
        {isLoading && (
          <div className="absolute inset-0 flex items-center justify-center bg-[#1a1a1a] z-10">
            <div className="text-center text-white/70">
              <Box size={48} className="mx-auto mb-3 animate-pulse text-[var(--accent)]" />
              <span className="text-sm">{t("modelViewer.loading", "Loading model...")}</span>
            </div>
          </div>
        )}
        {error && (
          <div className="absolute inset-0 flex items-center justify-center bg-[#1a1a1a] z-10">
            <div className="text-center text-red-400 px-4">
              <Box size={48} className="mx-auto mb-3 opacity-50" />
              <span className="text-sm">{error}</span>
            </div>
          </div>
        )}
      </div>

      {/* Footer hint */}
      <div className="text-center py-2 text-white/50 text-xs">
        {t("modelViewer.fullscreenHint", "Drag to rotate • Scroll to zoom • Right-click to pan • G for grid • L for light • Esc to close")}
      </div>
    </div>
  );
}
