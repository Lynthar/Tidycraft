import { useRef, useEffect, useState } from "react";
import * as THREE from "three";
import { OrbitControls } from "three/addons/controls/OrbitControls.js";
import { RotateCcw, Box, Maximize2 } from "lucide-react";
import { useTranslation } from "react-i18next";
import {
  disposeObjectTree,
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
  // The loaded model, held apart from the scene's permanent contents (lights
  // and grid) so that swapping files disposes only the model.
  const modelRef = useRef<THREE.Object3D | null>(null);
  // Monotonic token identifying the current load. Loader callbacks capture
  // their run's value and re-check it before touching any state — a shared
  // boolean can't do this, because the next run resets it to "alive" and a
  // still-in-flight onLoad/onError from the previous model then passes the
  // guard (hijacking mixerRef, adding the stale mesh to the scene, or
  // painting "Failed to load" over a successfully rendered model). Both
  // effects below bump it, so unmount invalidates in-flight callbacks too.
  const runIdRef = useRef(0);
  const mixerRef = useRef<THREE.AnimationMixer | null>(null);
  const clockRef = useRef<THREE.Clock>(new THREE.Clock());

  const [isLoading, setIsLoading] = useState(true);
  const [error, setError] = useState<ModelError | null>(null);
  const [stats, setStats] = useState<LoadingStats | null>(null);

  // The viewer — renderer, scene, camera, controls, lights, grid — is built
  // once per mount and outlives every file shown in it.
  //
  // It used to be rebuilt for each file, which cost one WebGL context per
  // model previewed. `releaseRenderer` force-loses the old context, but
  // WebKit does not hand the slot back until GC, so walking a folder of
  // models climbs to the engine's ~16-context ceiling; from there on it logs
  // "There are too many active WebGL contexts on this page" on every
  // creation, and — because it force-loses the oldest context itself, which
  // is always one we had already abandoned — our own teardown then adds
  // "INVALID_OPERATION: loseContext: context already lost". Measured
  // 2026-08-15 on a fresh page, 16 models walked with the arrow keys: 17
  // contexts and that pair of errors before, 2 contexts and a clean console
  // after (the 2 is React.StrictMode double-invoking this effect in dev;
  // production creates one).
  //
  // `theme` is read here only for the starting backdrop; the effect further
  // down keeps it current without rebuilding any of this.
  useEffect(() => {
    const container = containerRef.current;
    if (!container) return;

    const width = container.clientWidth || 250;
    const height = container.clientHeight || 250;

    const scene = new THREE.Scene();
    scene.background = new THREE.Color(VIEWER_BACKDROP[theme]);

    const camera = new THREE.PerspectiveCamera(50, width / height, 0.1, 1000);
    camera.position.set(2, 2, 2);

    let renderer: THREE.WebGLRenderer;
    try {
      renderer = new THREE.WebGLRenderer({ antialias: true, alpha: true });
      renderer.setSize(width, height);
      renderer.setPixelRatio(Math.min(window.devicePixelRatio, 2));
      container.appendChild(renderer.domElement);
    } catch (err) {
      console.error("Failed to create WebGL renderer:", err);
      setError({ key: "modelViewer.webglError", fallback: "WebGL not supported" });
      setIsLoading(false);
      // Leave the refs null: the load effect below reads sceneRef to decide
      // whether there is anything to load into, and must not paint over this
      // error with a loading spinner.
      return;
    }

    const controls = new OrbitControls(camera, renderer.domElement);
    controls.enableDamping = true;
    controls.dampingFactor = 0.05;
    controls.enableZoom = true;
    controls.enablePan = true;

    const ambientLight = new THREE.AmbientLight(0xffffff, 0.8);
    scene.add(ambientLight);

    const directionalLight = new THREE.DirectionalLight(0xffffff, 1.0);
    directionalLight.position.set(5, 10, 7.5);
    scene.add(directionalLight);

    const directionalLight2 = new THREE.DirectionalLight(0xffffff, 0.5);
    directionalLight2.position.set(-5, -5, -5);
    scene.add(directionalLight2);

    const gridHelper = new THREE.GridHelper(10, 10, 0x444444, 0x333333);
    scene.add(gridHelper);

    sceneRef.current = scene;
    cameraRef.current = camera;
    rendererRef.current = renderer;
    controlsRef.current = controls;

    // The render loop belongs to this mount, not to a file — guarding it with
    // the load token would stop it the moment the user picked another model.
    let stopped = false;
    let frameId = 0;
    const animate = () => {
      if (stopped) return;
      frameId = requestAnimationFrame(animate);

      if (mixerRef.current) {
        mixerRef.current.update(clockRef.current.getDelta());
      }
      controls.update();
      renderer.render(scene, camera);
    };
    animate();

    // Handle resize. Observe the container (not just window) so the canvas
    // also tracks react-resizable-panels divider drags — those resize the
    // panel without firing a window `resize` event. A container observer
    // covers window resizes too, since the container is responsive.
    const handleResize = () => {
      const newWidth = container.clientWidth || 250;
      const newHeight = container.clientHeight || 250;
      camera.aspect = newWidth / newHeight;
      camera.updateProjectionMatrix();
      renderer.setSize(newWidth, newHeight);
    };

    const resizeObserver = new ResizeObserver(handleResize);
    resizeObserver.observe(container);

    return () => {
      stopped = true;
      if (frameId) cancelAnimationFrame(frameId);
      resizeObserver.disconnect();
      runIdRef.current++; // invalidate any loader callback still in flight
      if (mixerRef.current) {
        mixerRef.current.stopAllAction();
        mixerRef.current = null;
      }
      controls.dispose();
      releaseRenderer(renderer);
      disposeSceneContents(scene);
      modelRef.current = null;
      controlsRef.current = null;
      rendererRef.current = null;
      sceneRef.current = null;
      cameraRef.current = null;
    };
    // Deliberately empty: nothing about this viewer depends on the props.
  }, []);

  // Changing file swaps the model and nothing else.
  useEffect(() => {
    const scene = sceneRef.current;
    // No scene means the renderer above failed; its error is on screen and
    // there is nothing to load into.
    if (!scene) return;

    runIdRef.current++; // cut off any callback still in flight for the old file
    const runId = runIdRef.current;
    const isStale = () => runIdRef.current !== runId;

    // Each file starts from the default view, as it did when picking one
    // rebuilt the camera outright.
    controlsRef.current?.reset();

    setIsLoading(true);
    setError(null);

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
        modelRef.current = object;

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

    return () => {
      if (mixerRef.current) {
        mixerRef.current.stopAllAction();
        mixerRef.current = null;
      }
      // Drop this file's model, leaving the lights and grid standing. Guarded
      // because on unmount the effect above has already emptied the scene.
      const model = modelRef.current;
      if (model && sceneRef.current) {
        sceneRef.current.remove(model);
        disposeObjectTree(model);
      }
      modelRef.current = null;
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
            <div className="text-[10px] text-ink-3">
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
