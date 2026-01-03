import { useRef, useEffect, useState } from "react";
import * as THREE from "three";
import { OrbitControls } from "three/addons/controls/OrbitControls.js";
import { GLTFLoader } from "three/addons/loaders/GLTFLoader.js";
import { OBJLoader } from "three/addons/loaders/OBJLoader.js";
import { convertFileSrc } from "@tauri-apps/api/core";
import { RotateCcw, Box } from "lucide-react";
import { useTranslation } from "react-i18next";

interface ModelViewer3DProps {
  filePath: string;
  extension: string;
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

  const [isLoading, setIsLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  // Clean up Three.js resources
  const cleanup = () => {
    isMountedRef.current = false;
    if (animationIdRef.current) {
      cancelAnimationFrame(animationIdRef.current);
      animationIdRef.current = 0;
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

    const onLoad = (object: THREE.Object3D) => {
      if (!isMountedRef.current) return;

      // Add default material to OBJ models if they don't have one
      object.traverse((child) => {
        if (child instanceof THREE.Mesh) {
          if (!child.material || (child.material instanceof THREE.MeshBasicMaterial && !child.material.map)) {
            child.material = new THREE.MeshStandardMaterial({
              color: 0x888888,
              metalness: 0.3,
              roughness: 0.7,
            });
          }
        }
      });

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
      setIsLoading(false);
    };

    const onError = (err: unknown) => {
      if (!isMountedRef.current) return;
      console.error("Failed to load model:", err);
      setError(t("modelViewer.loadError", "Failed to load model"));
      setIsLoading(false);
    };

    // Only support GLTF/GLB and OBJ - FBX requires additional dependencies that may cause issues
    const supportedFormats = ["gltf", "glb", "obj"];

    if (!supportedFormats.includes(ext)) {
      setError(t("modelViewer.unsupportedFormat", `Format .${ext} not supported. Use GLTF, GLB, or OBJ.`));
      setIsLoading(false);
    } else {
      try {
        if (ext === "gltf" || ext === "glb") {
          const loader = new GLTFLoader();
          loader.load(
            modelUrl,
            (gltf) => onLoad(gltf.scene),
            undefined,
            onError
          );
        } else if (ext === "obj") {
          const loader = new OBJLoader();
          loader.load(modelUrl, onLoad, undefined, onError);
        }
      } catch (err) {
        onError(err);
      }
    }

    // Animation loop
    const animate = () => {
      if (!isMountedRef.current) return;
      animationIdRef.current = requestAnimationFrame(animate);
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
        <span className="text-xs text-text-secondary">
          {t("modelViewer.controls", "Drag to rotate • Scroll to zoom")}
        </span>
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
