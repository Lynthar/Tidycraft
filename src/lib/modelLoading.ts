/// Everything ModelViewer3D and ModelLightbox do identically to get a
/// three.js `Object3D` on screen: format dispatch, material normalization,
/// framing, error classification, and teardown.
///
/// The two components diverge in what they ARE — an inline square panel
/// versus a fullscreen lightbox with grid/lighting toggles and keyboard
/// shortcuts — but not in how a model gets loaded. Before this module they
/// carried 467 byte-identical lines between them and stayed in step by hand,
/// via four "See ModelViewer3D…" comments. The interest that was accruing on
/// that is visible in the footer both files render: the vertex/mesh line was
/// written twice, and both copies are equally untranslated.
///
/// What deliberately did NOT move here: scene, camera, renderer, lights and
/// grid construction. Those look similar but differ in every constant that
/// matters (fit distance, light count and intensity, shadow maps, grid size,
/// zoom clamps), and folding them behind an options bag would produce exactly
/// the boolean-riddled shared function that is worse than the duplication.

import * as THREE from "three";
import { GLTFLoader } from "three/addons/loaders/GLTFLoader.js";
import { OBJLoader } from "three/addons/loaders/OBJLoader.js";
import { FBXLoader } from "three/addons/loaders/FBXLoader.js";
import { MTLLoader } from "three/addons/loaders/MTLLoader.js";
import { convertFileSrc } from "@tauri-apps/api/core";

import { buildTextureUrlResolver } from "./modelUrlResolver";
import { dirname } from "./pathUtils";

/// Formats routed into the 3D viewers. `.blend` is in the list so
/// AssetPreview sends the file here rather than to the box-icon fallback —
/// `loadModel` then short-circuits to an actionable "export to GLB" message.
/// Real loading is impossible: .blend is Blender's private binary format
/// with no web loader.
export const SUPPORTED_MODEL_FORMATS = [
  "gltf",
  "glb",
  "fbx",
  "obj",
  "dae",
  "3ds",
  "blend",
  "vox",
];

/// The 3D scene's backdrop — one pair of values for both viewers, as a CSS hex
/// string because `THREE.Color` takes one. The inline viewer picks by app theme;
/// the lightbox keeps its own light/dark toggle over the same pair.
///
/// Deliberately NOT design tokens, and deliberately not used by the loading /
/// error overlays either: this is the inside of a render target. The overlays
/// are app chrome and use theme tokens, which also keeps their text paired with
/// their own background — the lightbox's backdrop toggle is independent of the
/// theme, and no single red satisfies 4.5:1 against both backdrops at once.
export const VIEWER_BACKDROP = {
  dark: "#1a1a1a",
  light: "#f0f0f0",
} as const;

export interface ModelStats {
  meshCount: number;
  vertexCount: number;
}

/// An error held as an i18n key (+ optional fallback) rather than a
/// pre-translated string, so it re-translates on a language switch without
/// re-running the WebGL setup effect — which would otherwise tear down and
/// rebuild the whole scene just to relabel one message. Render with
/// `t(error.key, error.fallback)`.
export interface ModelError {
  key: string;
  fallback?: string;
}

/// Give every mesh a material that will actually be visible under the
/// viewers' lighting, and count meshes and vertices while walking the tree.
///
/// `vertexColors` is carried across every conversion: OBJLoader sets it true
/// for OBJs with inline `v x y z r g b` lines, and unlit GLTFs
/// (KHR_materials_unlit) arrive as MeshBasicMaterial. Dropping it renders
/// those flat gray.
export function fixMaterials(object: THREE.Object3D): ModelStats {
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

        // Invisible under our lights: MeshBasicMaterial with no texture.
        if (mat instanceof THREE.MeshBasicMaterial && !mat.map) {
          return new THREE.MeshStandardMaterial({
            color: mat.color || 0x888888,
            metalness: 0.3,
            roughness: 0.7,
            side: THREE.DoubleSide,
            vertexColors: mat.vertexColors,
          });
        }

        // Common in FBX, and what OBJLoader creates for an OBJ with no
        // `mtllib`.
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

        // Transparent with zero opacity renders as nothing at all.
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
}

/// Play the model's first animation clip, if it has one. Returns the mixer
/// so the caller's render loop can drive it, or null when there is nothing
/// to animate.
export function setupAnimations(
  object: THREE.Object3D
): THREE.AnimationMixer | null {
  const animations = (
    object as THREE.Object3D & { animations?: THREE.AnimationClip[] }
  ).animations;
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
}

/// Center `object` on the origin and scale its largest dimension to
/// `targetSize`.
///
/// Order matters: scale BEFORE the position offset, because the resulting
/// world transform is T·S, so a mesh ends up at `position + scale *
/// localCenter`. Translating first only happens to look right when
/// localCenter is already near the hierarchy root (typical of GLTF/GLB/OBJ);
/// FBX and DAE often place the mesh node far from its root, and the wrong
/// order then drifts the model off the grid by `(scale - 1) * center`.
export function fitObjectToView(
  object: THREE.Object3D,
  targetSize: number
): void {
  const box = new THREE.Box3().setFromObject(object);
  const center = box.getCenter(new THREE.Vector3());
  const size = box.getSize(new THREE.Vector3());
  const maxDim = Math.max(size.x, size.y, size.z);
  const scale = maxDim > 0 ? targetSize / maxDim : 1;

  object.scale.multiplyScalar(scale);
  object.position.sub(center.multiplyScalar(scale));
}

/// Dispose every geometry and material in the scene, then empty it. The
/// caller still drops its own reference — this only releases GPU memory.
export function disposeSceneContents(scene: THREE.Scene): void {
  scene.traverse((object) => {
    if (object instanceof THREE.Mesh) {
      object.geometry?.dispose();
      if (Array.isArray(object.material)) {
        object.material.forEach((m) => m.dispose());
      } else if (object.material) {
        object.material.dispose();
      }
    }
  });
  scene.clear();
}

/// Dispose a renderer, release its WebGL context, and detach its canvas.
///
/// `dispose()` frees GPU buffers but does NOT release the context; browsers
/// cap active contexts (~16/page), so without `forceContextLoss` swapping
/// between many model previews — or reopening the lightbox repeatedly —
/// exhausts them and the browser force-loses the oldest ("Too many active
/// WebGL contexts"), leaving a black preview.
export function releaseRenderer(renderer: THREE.WebGLRenderer): void {
  renderer.dispose();
  renderer.forceContextLoss();
  const domElement = renderer.domElement;
  if (domElement && domElement.parentNode) {
    domElement.parentNode.removeChild(domElement);
  }
}

/// Turn a loader failure into the most specific message we can offer.
function classifyModelError(err: unknown, ext: string): ModelError {
  const message = err instanceof Error ? err.message : String(err);

  if (message.includes("404") || message.includes("not found")) {
    return { key: "modelViewer.fileNotFound", fallback: "File not found" };
  }
  // three.js's FBXLoader is a reverse-engineered parser that doesn't cover
  // every UV/MappingInformationType combination Autodesk DCC tools emit. The
  // failure mode is a cryptic `Cannot read properties of undefined (reading
  // 'a')` from GeometryParser.parseUVs. We can't fix the parser, but we can
  // tell the user a path forward (re-export as GLB).
  if (
    ext === "fbx" &&
    (message.includes("Cannot read properties of undefined") ||
      message.includes("parseUVs"))
  ) {
    return { key: "modelViewer.fbxIncompatible" };
  }
  if (message.includes("parse") || message.includes("invalid")) {
    return {
      key: "modelViewer.parseError",
      fallback: "Failed to parse model file",
    };
  }
  return { key: "modelViewer.loadError", fallback: "Failed to load model" };
}

export interface LoadModelOptions {
  filePath: string;
  extension: string;
  /// True once this load has been superseded — the component unmounted, or a
  /// newer model started. Checked after every await and inside every loader
  /// callback: a plain "mounted" boolean cannot do this job, because the next
  /// run resets it to alive and a still-in-flight callback from the previous
  /// model then passes the guard, hijacking the mixer or painting "Failed to
  /// load" over a model that rendered fine. Callers pass a monotonic-token
  /// comparison.
  isStale: () => boolean;
  onLoad: (object: THREE.Object3D) => void;
  onFailure: (error: ModelError) => void;
  /// Component name, for the console line on failure.
  label: string;
}

/// Load `filePath` with the right three.js loader for its extension and hand
/// the resulting object to `onLoad`. Every failure — unsupported format,
/// missing file, unparseable geometry — arrives at `onFailure` already
/// classified into an i18n key.
///
/// The unsupported-format rejection happens before the first `await`, so it
/// still lands synchronously within the caller's effect, as it did when this
/// lived inline.
export async function loadModel({
  filePath,
  extension,
  isStale,
  onLoad,
  onFailure,
  label,
}: LoadModelOptions): Promise<void> {
  const ext = extension.toLowerCase();

  if (!SUPPORTED_MODEL_FORMATS.includes(ext)) {
    onFailure({
      key: "modelViewer.unsupportedFormat",
      // Unreachable while `modelViewer.unsupportedFormat` exists in a
      // shipped locale (it does, in both, and en is the fallback language).
      fallback: "Format not supported. Use GLTF, GLB, FBX, or OBJ.",
    });
    return;
  }

  const modelUrl = convertFileSrc(filePath);
  const dir = dirname(filePath);
  const resourcePath = convertFileSrc(dir ? `${dir}/` : "");

  const fail = (err: unknown) => {
    if (isStale()) return;
    console.error(`[${label}] Failed to load ${ext.toUpperCase()} model:`, {
      filePath,
      modelUrl,
      error: err,
    });
    onFailure(classifyModelError(err, ext));
  };

  // Await the sibling-texture scan before wiring the URL modifier. The scan
  // is a single filesystem walk of the model's directory, typically <10ms.
  const urlModifier = await buildTextureUrlResolver(filePath);
  if (isStale()) return;

  const loadingManager = new THREE.LoadingManager();
  loadingManager.setURLModifier(urlModifier);
  // Some three.js loaders use resolveURL instead of the URL modifier; set both.
  loadingManager.resolveURL = urlModifier;

  try {
    if (ext === "gltf" || ext === "glb") {
      const loader = new GLTFLoader(loadingManager);
      loader.setResourcePath(resourcePath);
      loader.load(modelUrl, (gltf) => onLoad(gltf.scene), undefined, fail);
    } else if (ext === "obj") {
      // Pre-fetch the OBJ text so we can (a) honor the actual `mtllib`
      // filename instead of guessing `<basename>.mtl` — OBJ allows arbitrary
      // names like `mtllib materials.mtl` — and (b) skip the MTL request
      // entirely when no `mtllib` line is present. The previous blind attempt
      // at `.mtl` produced a console-polluting 500 from the asset protocol
      // (the silent fallback rendered the OBJ correctly, but the log noise
      // was confusing). Using `parse(text)` avoids a second fetch by
      // OBJLoader.
      let objText: string;
      try {
        const resp = await fetch(modelUrl);
        if (!resp.ok) throw new Error(`HTTP ${resp.status}`);
        objText = await resp.text();
      } catch (err) {
        fail(err);
        return;
      }
      if (isStale()) return;

      const mtllibMatch = objText.match(/^mtllib\s+(.+?)\s*$/m);
      const objLoader = new OBJLoader(loadingManager);

      const finalize = () => {
        try {
          onLoad(objLoader.parse(objText));
        } catch (parseErr) {
          fail(parseErr);
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
          // An unreadable MTL is not fatal: render the geometry untextured
          // rather than showing an error over a model that loads fine.
          () => finalize()
        );
      } else {
        finalize();
      }
    } else if (ext === "fbx") {
      const loader = new FBXLoader(loadingManager);
      loader.setResourcePath(resourcePath);
      loader.load(modelUrl, onLoad, undefined, fail);
    } else if (ext === "dae") {
      const { ColladaLoader } = await import(
        "three/addons/loaders/ColladaLoader.js"
      );
      if (isStale()) return;
      const loader = new ColladaLoader(loadingManager);
      loader.setResourcePath(resourcePath);
      loader.load(modelUrl, (collada) => onLoad(collada.scene), undefined, fail);
    } else if (ext === "3ds") {
      const { TDSLoader } = await import("three/addons/loaders/TDSLoader.js");
      if (isStale()) return;
      const loader = new TDSLoader(loadingManager);
      loader.setResourcePath(resourcePath);
      loader.load(modelUrl, onLoad, undefined, fail);
    } else if (ext === "vox") {
      // VOXLoader (r182) returns `{ chunks, scene }`. Modern files with
      // nTRN/nGRP/nSHP nodes populate `scene` directly; older v150
      // single-model exports (e.g. plain MagicaVoxel saves) only carry
      // SIZE/XYZI/RGBA chunks → `scene` is null at runtime even though
      // @types/three claims Object3D. Fall back to manual `buildMesh` per
      // chunk so both shapes load. VOX is self-contained (palette + voxel
      // data, no external textures), so no setResourcePath is needed;
      // buildMesh already centers the geometry and emits a vertex-color
      // MeshStandardMaterial that survives fixMaterials intact.
      const { VOXLoader, buildMesh } = await import(
        "three/addons/loaders/VOXLoader.js"
      );
      if (isStale()) return;
      const loader = new VOXLoader(loadingManager);
      loader.load(
        modelUrl,
        (result) => {
          if (isStale()) return;
          let root: THREE.Object3D | null = result.scene;
          if (!root) {
            if (!result.chunks || result.chunks.length === 0) {
              fail(new Error("Empty VOX file"));
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
        fail
      );
    } else if (ext === "blend") {
      // .blend is Blender's private binary format — no web loader exists. We
      // surface a clear "export to GLB" message rather than fail mysteriously
      // or fall through to "unsupported".
      if (isStale()) return;
      onFailure({ key: "modelViewer.blendUnsupported" });
    }
  } catch (err) {
    fail(err);
  }
}
