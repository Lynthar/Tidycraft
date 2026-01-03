import { useRef, useEffect, useState } from "react";
import * as THREE from "three";
import { OrbitControls } from "three/addons/controls/OrbitControls.js";
import { GLTFLoader } from "three/addons/loaders/GLTFLoader.js";
import { OBJLoader } from "three/addons/loaders/OBJLoader.js";
import { FBXLoader } from "three/addons/loaders/FBXLoader.js";
import { MTLLoader } from "three/addons/loaders/MTLLoader.js";
import { convertFileSrc } from "@tauri-apps/api/core";
import { RotateCcw, Box } from "lucide-react";
import { useTranslation } from "react-i18next";

// Supported 3D model formats
const SUPPORTED_FORMATS = ["gltf", "glb", "fbx", "obj", "dae"];

interface ModelViewer3DProps {
  filePath: string;
  extension: string;
}

interface LoadingStats {
  format: string;
  vertexCount: number;
  meshCount: number;
}

export function ModelViewer3D({ filePath, extension }: ModelViewer3DProps) {
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
    const modelDir = filePath.substring(0, filePath.lastIndexOf('/') + 1);
    // Note: convertFileSrc already returns path without trailing slash, we need to add one
    // so texture paths like "texture.png" become "asset://path/to/dir/texture.png"
    const resourcePath = convertFileSrc(modelDir);

    console.log(`[ModelViewer3D] Loading ${ext.toUpperCase()} model from: ${modelUrl}`);
    console.log(`[ModelViewer3D] Model directory: ${modelDir}`);
    console.log(`[ModelViewer3D] Resource path for textures: ${resourcePath}`);

    // Create a custom LoadingManager with URL modifier
    const loadingManager = new THREE.LoadingManager();
    loadingManager.setURLModifier((url: string) => {
      console.log(`[ModelViewer3D] LoadingManager URL modifier called with: ${url}`);

      // If already an asset:// URL, return as-is
      if (url.startsWith('asset://') || url.startsWith('data:') || url.startsWith('blob:')) {
        return url;
      }

      // Handle http/https URLs (external textures) - return as-is
      if (url.startsWith('http://') || url.startsWith('https://')) {
        return url;
      }

      // Handle absolute paths (from FBX/OBJ files)
      if (url.startsWith('/')) {
        const converted = convertFileSrc(url);
        console.log(`[ModelViewer3D] Converting absolute path: ${url} -> ${converted}`);
        return converted;
      }

      // Handle relative paths - resolve relative to model directory
      // Extract just the filename in case the path has directories we don't have
      const filename = url.split('/').pop() || url;
      const fullPath = modelDir + filename;
      const converted = convertFileSrc(fullPath);
      console.log(`[ModelViewer3D] Converting relative path: ${url} -> ${converted}`);
      return converted;
    });

    // Create a URL converter function for textures
    const convertTextureUrl = (url: string): string => {
      if (!url) return url;

      // If already an asset:// URL, return as-is
      if (url.startsWith('asset://') || url.startsWith('data:') || url.startsWith('blob:')) {
        return url;
      }

      // Handle http/https URLs (external textures) - return as-is
      if (url.startsWith('http://') || url.startsWith('https://')) {
        return url;
      }

      // Handle absolute paths
      if (url.startsWith('/')) {
        const converted = convertFileSrc(url);
        console.log(`[ModelViewer3D] Converting absolute texture path: ${url} -> ${converted}`);
        return converted;
      }

      // Handle relative paths - extract filename and resolve relative to model directory
      const filename = url.split(/[/\\]/).pop() || url;
      const fullPath = modelDir + filename;
      const converted = convertFileSrc(fullPath);
      console.log(`[ModelViewer3D] Converting relative texture path: ${url} -> ${converted}`);
      return converted;
    };

    // Override LoadingManager's resolveURL to intercept all URL resolutions
    loadingManager.resolveURL = (url: string): string => {
      console.log(`[ModelViewer3D] resolveURL called with: ${url}`);
      return convertTextureUrl(url);
    };

    const onLoad = (object: THREE.Object3D) => {
      if (!isMountedRef.current) return;

      console.log(`[ModelViewer3D] Model loaded successfully:`, object);

      // Fix materials and get stats
      const modelStats = fixMaterials(object);

      // Center the model
      const box = new THREE.Box3().setFromObject(object);
      const center = box.getCenter(new THREE.Vector3());
      const size = box.getSize(new THREE.Vector3());

      object.position.sub(center);

      // Scale to fit
      const maxDim = Math.max(size.x, size.y, size.z);
      if (maxDim > 0) {
        const scale = 2 / maxDim;
        object.scale.multiplyScalar(scale);
      }

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
          // Try to load MTL file if it exists (same name, .mtl extension)
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
          // COLLADA support - use dynamic import to avoid loading if not needed
          import("three/addons/loaders/ColladaLoader.js").then(({ ColladaLoader }) => {
            const loader = new ColladaLoader(loadingManager);
            loader.setResourcePath(resourcePath);
            loader.load(
              modelUrl,
              (collada) => onLoad(collada.scene),
              undefined,
              onError
            );
          }).catch(onError);
        }
      } catch (err) {
        onError(err);
      }
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
        <button
          onClick={resetCamera}
          className="p-1 rounded hover:bg-card-bg text-text-secondary hover:text-text-primary transition-colors"
          title={t("modelViewer.reset", "Reset view")}
        >
          <RotateCcw size={14} />
        </button>
      </div>
    </div>
  );
}
