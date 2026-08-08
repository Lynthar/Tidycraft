import { useRef, useEffect, useState } from "react";
import * as THREE from "three";
import { OrbitControls } from "three/addons/controls/OrbitControls.js";
import { RotateCcw, Box, Maximize2 } from "lucide-react";
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

/// Largest dimension the loaded model is scaled to, in world units. The
/// lightbox uses a larger value against its larger grid.
const FIT_SIZE = 2;

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

type LoadingStats = ModelStats & { format: string };

export function ModelViewer3D({ filePath, extension, vertexCount, onFullscreen }: ModelViewer3DProps) {
  const { t } = useTranslation();
  const theme = useThemeStore((s) => s.theme);
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
      releaseRenderer(rendererRef.current);
      rendererRef.current = null;
    }
    if (sceneRef.current) {
      disposeSceneContents(sceneRef.current);
      sceneRef.current = null;
    }
    cameraRef.current = null;
  };

  useEffect(() => {
    if (!containerRef.current) return;

    // Cleanup previous instance (bumps runIdRef, cutting off any loader
    // callback still in flight from the previous model).
    cleanup();
    const runId = runIdRef.current;
    const isStale = () => runIdRef.current !== runId;

    setIsLoading(true);
    setError(null);

    const container = containerRef.current;
    const width = container.clientWidth || 250;
    const height = container.clientHeight || 250;

    // Create scene
    const scene = new THREE.Scene();
    scene.background = new THREE.Color(VIEWER_BACKDROP[theme]);
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

    const ext = extension.toLowerCase();

    void loadModel({
      filePath,
      extension,
      isStale,
      label: "ModelViewer3D",
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

  // Theme is read at scene construction above but deliberately kept out of that
  // effect's dependencies: it would tear down and rebuild the renderer, reload
  // the model and reset the camera every time the user toggles the theme. The
  // backdrop is the only thing that has to follow, so it follows on its own.
  useEffect(() => {
    if (sceneRef.current) {
      sceneRef.current.background = new THREE.Color(VIEWER_BACKDROP[theme]);
    }
  }, [theme]);

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
          <div className="absolute inset-0 flex items-center justify-center bg-panel z-10">
            <div className="text-center text-text-secondary">
              <Box size={32} className="mx-auto mb-2 animate-pulse text-[var(--accent)]" />
              <span className="text-sm">{t("modelViewer.loading", "Loading model...")}</span>
            </div>
          </div>
        )}
        {error && (
          <div className="absolute inset-0 flex items-center justify-center bg-panel z-10">
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
              {stats.format} •{" "}
              {t("modelViewer.statsVertices", {
                count: vertexCount ?? stats.vertexCount,
              })}{" "}
              • {t("modelViewer.statsMeshes", { count: stats.meshCount })}
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
