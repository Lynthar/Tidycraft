import { useRef, useEffect, useState, useCallback } from "react";
import * as THREE from "three";
import { OrbitControls } from "three/addons/controls/OrbitControls.js";
import { X, RotateCcw, Box, Grid3X3, Sun, Moon } from "lucide-react";
import { useTranslation } from "react-i18next";
import {
  disposeSceneContents,
  fitObjectToView,
  fixMaterials,
  loadModel,
  releaseRenderer,
  setupAnimations,
  VIEWER_BACKDROP,
  type ModelError,
  type ModelStats,
} from "../lib/modelLoading";
import { useThemeStore } from "../stores/themeStore";

/// Largest dimension the loaded model is scaled to, in world units. Bigger
/// than ModelViewer3D's because this grid is bigger and the view is further out.
const FIT_SIZE = 3;

interface ModelLightboxProps {
  isOpen: boolean;
  filePath: string;
  extension: string;
  /// Canonical unique-vertex count from backend metadata; preferred over
  /// three.js's loader-dependent count in the footer. See ModelViewer3D.
  vertexCount?: number;
  modelName: string;
  onClose: () => void;
}

type LoadingStats = ModelStats & { format: string };

export function ModelLightbox({ isOpen, filePath, extension, vertexCount, modelName, onClose }: ModelLightboxProps) {
  const { t } = useTranslation();
  const containerRef = useRef<HTMLDivElement>(null);
  const rendererRef = useRef<THREE.WebGLRenderer | null>(null);
  const sceneRef = useRef<THREE.Scene | null>(null);
  const cameraRef = useRef<THREE.PerspectiveCamera | null>(null);
  const controlsRef = useRef<OrbitControls | null>(null);
  const animationIdRef = useRef<number>(0);
  // Monotonic token identifying the current setup-effect run — see
  // ModelViewer3D for the full rationale (a shared "mounted" boolean lets
  // a previous model's slow onLoad/onError through once the next run
  // resets it). cleanup() bumps it; callbacks compare their captured
  // value against it before touching any state.
  const runIdRef = useRef(0);
  const mixerRef = useRef<THREE.AnimationMixer | null>(null);
  const clockRef = useRef<THREE.Clock>(new THREE.Clock());
  const gridRef = useRef<THREE.GridHelper | null>(null);
  const lightsRef = useRef<THREE.Light[]>([]);

  const [isLoading, setIsLoading] = useState(true);
  const [error, setError] = useState<ModelError | null>(null);
  const [stats, setStats] = useState<LoadingStats | null>(null);
  const [showGrid, setShowGrid] = useState(true);
  // The 3D backdrop is the user's to choose here (the Sun/Moon button below),
  // but it starts where the app already is: a light-theme user opening the
  // lightbox used to get an unconditionally black viewport under a button
  // offering to turn the lights ON.
  const appTheme = useThemeStore((s) => s.theme);
  const [darkMode, setDarkMode] = useState(appTheme === "dark");

  // Clean up Three.js resources
  const cleanup = useCallback(() => {
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
      releaseRenderer(rendererRef.current);
      rendererRef.current = null;
    }
    if (sceneRef.current) {
      disposeSceneContents(sceneRef.current);
      sceneRef.current = null;
    }
    cameraRef.current = null;
    gridRef.current = null;
    lightsRef.current = [];
  }, []);

  // Handle keyboard events
  useEffect(() => {
    if (!isOpen) return;

    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        onClose();
        return;
      }
      // Letter shortcuts must be unmodified: Ctrl/Cmd+R is the app-level
      // rescan chord (gated while the lightbox is open) and must not
      // double as "reset camera" here.
      if (e.ctrlKey || e.metaKey || e.altKey) return;
      switch (e.key) {
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
      sceneRef.current.background = new THREE.Color(
        VIEWER_BACKDROP[darkMode ? "dark" : "light"]
      );
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

    if (!containerRef.current) return;

    // Cleanup previous instance (bumps runIdRef, cutting off any loader
    // callback still in flight from the previous model).
    cleanup();
    const runId = runIdRef.current;
    const isStale = () => runIdRef.current !== runId;

    setIsLoading(true);
    setError(null);

    const container = containerRef.current;
    const width = container.clientWidth || window.innerWidth;
    const height = container.clientHeight || window.innerHeight - 120;

    // Create scene
    const scene = new THREE.Scene();
    scene.background = new THREE.Color(VIEWER_BACKDROP[darkMode ? "dark" : "light"]);
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

    const ext = extension.toLowerCase();

    void loadModel({
      filePath,
      extension,
      isStale,
      label: "ModelLightbox",
      onLoad: (object) => {
        if (isStale()) return;

        const modelStats = fixMaterials(object);
        fitObjectToView(object, FIT_SIZE);
        scene.add(object);

        const mixer = setupAnimations(object);
        if (mixer) {
          mixerRef.current = mixer;
        }

        setStats({ format: ext.toUpperCase(), ...modelStats });
        setIsLoading(false);
      },
      onFailure: (modelError) => {
        if (isStale()) return;
        setError(modelError);
        setIsLoading(false);
      },
    });

    // Animation loop
    const animate = () => {
      if (isStale()) return;
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
    // darkMode / showGrid are deliberately NOT dependencies: they're
    // patched in place by the two small effects above, and re-running
    // this effect would tear down the scene and reload the model (a
    // 50MB FBX takes seconds) just to change the background or grid.
    // The effect body still reads their current values whenever it does
    // re-run (open / model switch). `t` isn't one either — errors are
    // stored as i18n keys and translated at render time.
  }, [isOpen, filePath, extension, cleanup]);

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
              {stats.format} •{" "}
              {t("modelViewer.statsVertices", {
                count: vertexCount ?? stats.vertexCount,
              })}{" "}
              • {t("modelViewer.statsMeshes", { count: stats.meshCount })}
            </span>
          )}
          <button
            onClick={() => setShowGrid(!showGrid)}
            className={`p-2 rounded transition-colors ${showGrid ? 'bg-white/20' : 'hover:bg-white/10'}`}
            title={t("modelViewer.gridTitle")}
          >
            <Grid3X3 size={18} />
          </button>
          <button
            onClick={() => setDarkMode(!darkMode)}
            className="p-2 rounded hover:bg-white/10 transition-colors"
            title={t("modelViewer.backgroundTitle")}
          >
            {darkMode ? <Sun size={18} /> : <Moon size={18} />}
          </button>
          <button
            onClick={resetCamera}
            className="p-2 rounded hover:bg-white/10 transition-colors"
            title={t("modelViewer.resetTitle")}
          >
            <RotateCcw size={18} />
          </button>
          <button
            onClick={onClose}
            className="p-2 rounded hover:bg-white/10 transition-colors ml-2"
            title={t("modelViewer.closeTitle")}
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
          <div className="absolute inset-0 flex items-center justify-center bg-panel z-10">
            <div className="text-center text-ink-2">
              <Box size={48} className="mx-auto mb-3 animate-pulse text-[var(--accent)]" />
              <span className="text-sm">{t("modelViewer.loading", "Loading model...")}</span>
            </div>
          </div>
        )}
        {error && (
          <div className="absolute inset-0 flex items-center justify-center bg-panel z-10">
            <div className="text-center text-err px-4">
              <Box size={48} className="mx-auto mb-3 opacity-50" />
              <span className="text-sm">
                {error.fallback ? t(error.key, error.fallback) : t(error.key)}
              </span>
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
