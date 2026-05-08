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

// Supported 3D model formats
const SUPPORTED_FORMATS = ["gltf", "glb", "fbx", "obj", "dae"];

interface ModelViewer3DProps {
  filePath: string;
  extension: string;
  onFullscreen?: () => void;
}

interface LoadingStats {
  format: string;
  vertexCount: number;
  meshCount: number;
}

export function ModelViewer3D({ filePath, extension, onFullscreen }: ModelViewer3DProps) {
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

  const [isLoading, setIsLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [stats, setStats] = useState<LoadingStats | null>(null);

  // Clean up Three.js resources
  const cleanup = () => {
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

          // Fix invisible materials - MeshBasicMaterial without texture
          if (mat instanceof THREE.MeshBasicMaterial && !mat.map) {
            return new THREE.MeshStandardMaterial({
              color: mat.color || 0x888888,
              metalness: 0.3,
              roughness: 0.7,
              side: THREE.DoubleSide,
            });
          }

          // Convert MeshPhongMaterial (common in FBX) to MeshStandardMaterial for better rendering
          if (mat instanceof THREE.MeshPhongMaterial) {
            const stdMat = new THREE.MeshStandardMaterial({
              color: mat.color || 0x888888,
              map: mat.map,
              normalMap: mat.normalMap,
              metalness: 0.3,
              roughness: 0.7,
              side: THREE.DoubleSide,
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
    isMountedRef.current = true;

    if (!containerRef.current) return;

    // Cleanup previous instance
    cleanup();
    isMountedRef.current = true;

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
      if (!isMountedRef.current) return;

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
      if (!isMountedRef.current) return;
      console.error(`[ModelViewer3D] Failed to load ${ext.toUpperCase()} model:`, {
        filePath,
        modelUrl,
        error: err,
      });
      const message = err instanceof Error ? err.message : String(err);

      // Provide more helpful error messages
      if (message.includes("404") || message.includes("not found")) {
        setError(t("modelViewer.fileNotFound", "File not found"));
      } else if (message.includes("parse") || message.includes("invalid")) {
        setError(t("modelViewer.parseError", "Failed to parse model file"));
      } else {
        setError(t("modelViewer.loadError", "Failed to load model"));
      }
      setIsLoading(false);
    };

    if (!SUPPORTED_FORMATS.includes(ext)) {
      setError(t("modelViewer.unsupportedFormat", `Format .${ext} not supported. Use GLTF, GLB, FBX, or OBJ.`));
      setIsLoading(false);
    } else {
      // Kick off loading in an async IIFE so we can await the sibling-texture
      // scan before wiring the URL modifier. The scan is a single filesystem
      // walk of the model's directory, typically <10ms.
      (async () => {
        const urlModifier = await buildTextureUrlResolver(filePath);
        if (!isMountedRef.current) return;

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
            const mtlPath = filePath.replace(/\.obj$/i, ".mtl");
            const mtlUrl = convertFileSrc(mtlPath);

            const mtlLoader = new MTLLoader(loadingManager);
            mtlLoader.setResourcePath(resourcePath);
            mtlLoader.load(
              mtlUrl,
              (materials) => {
                materials.preload();
                const objLoader = new OBJLoader(loadingManager);
                objLoader.setMaterials(materials);
                objLoader.load(modelUrl, onLoad, undefined, onError);
              },
              undefined,
              () => {
                // MTL failed, load OBJ without materials
                const objLoader = new OBJLoader(loadingManager);
                objLoader.load(modelUrl, onLoad, undefined, onError);
              }
            );
          } else if (ext === "fbx") {
            const loader = new FBXLoader(loadingManager);
            loader.setResourcePath(resourcePath);
            loader.load(modelUrl, onLoad, undefined, onError);
          } else if (ext === "dae") {
            const { ColladaLoader } = await import("three/addons/loaders/ColladaLoader.js");
            if (!isMountedRef.current) return;
            const loader = new ColladaLoader(loadingManager);
            loader.setResourcePath(resourcePath);
            loader.load(
              modelUrl,
              (collada) => onLoad(collada.scene),
              undefined,
              onError
            );
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

    // Handle resize
    const handleResize = () => {
      if (!containerRef.current || !cameraRef.current || !rendererRef.current) return;
      const newWidth = containerRef.current.clientWidth || 250;
      const newHeight = containerRef.current.clientHeight || 250;
      cameraRef.current.aspect = newWidth / newHeight;
      cameraRef.current.updateProjectionMatrix();
      rendererRef.current.setSize(newWidth, newHeight);
    };

    window.addEventListener("resize", handleResize);

    return () => {
      window.removeEventListener("resize", handleResize);
      cleanup();
    };
  }, [filePath, extension, t]);

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
              <Box size={32} className="mx-auto mb-2 animate-pulse text-blue-400" />
              <span className="text-sm">{t("modelViewer.loading", "Loading model...")}</span>
            </div>
          </div>
        )}
        {error && (
          <div className="absolute inset-0 flex items-center justify-center bg-[#1a1a1a] z-10">
            <div className="text-center text-error px-4">
              <Box size={32} className="mx-auto mb-2 opacity-50" />
              <span className="text-sm">{error}</span>
            </div>
          </div>
        )}
      </div>
      <div className="p-2 flex items-center justify-between border-t border-border">
        <div className="text-xs text-text-secondary space-y-0.5">
          <div>{t("modelViewer.controls", "Drag to rotate • Scroll to zoom")}</div>
          {stats && (
            <div className="text-[10px] text-text-secondary/70">
              {stats.format} • {(stats.vertexCount / 1000).toFixed(1)}K vertices • {stats.meshCount} meshes
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
