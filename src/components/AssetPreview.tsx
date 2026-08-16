import { useEffect, useRef, useState } from "react";
import { invoke, convertFileSrc } from "@tauri-apps/api/core";
import { writeText } from "@tauri-apps/plugin-clipboard-manager";
import {
  Image,
  Box,
  Volume2,
  Video,
  File,
  FileText,
  X,
  Copy,
  Check,
  Maximize2,
  Plus,
  ExternalLink,
  FolderOpen,
  Network,
  Settings,
} from "lucide-react";
import { useTranslation } from "react-i18next";
import { useProjectStore } from "../stores/projectStore";
import { useUiStore } from "../stores/uiStore";
import { useTagsStore } from "../stores/tagsStore";
import { useSettingsStore } from "../stores/settingsStore";
import { formatFileSize, formatDuration } from "../lib/utils";
import { getExtension, getEditorDisplayName } from "../lib/pathUtils";
import { dccSourceLabel } from "../lib/dccSource";
import { VideoPlayer } from "./VideoPlayer";
import { AudioPlayer } from "./AudioPlayer";
import { ImageLightbox } from "./ImageLightbox";
import { ModelViewer3D } from "./ModelViewer3D";
import { ModelLightbox } from "./ModelLightbox";
import type { AssetInfo, AssetType, UnityFileInfo } from "../types/asset";

const VIDEO_EXTENSIONS = ["mp4", "webm", "mov", "avi", "mkv", "m4v"];
// `.3ds` and `.blend` route into ModelViewer3D too: 3ds renders via TDSLoader,
// and blend cannot render in the browser but the viewer surfaces a "please export
// to GLB" message instead of the silent box-icon fallback.
const MODEL_3D_EXTENSIONS = ["gltf", "glb", "fbx", "obj", "dae", "3ds", "blend", "vox"];

// Holding an arrow key repeats well under 100ms per row, and every repeat swaps
// `selectedAsset`. 150ms outlasts one repeat interval, so a held key coalesces
// into a single settle while a brief pause still reads as instant.
const SETTLED_ASSET_DEBOUNCE_MS = 150;

function GlyphIcon({ type, size = 11 }: { type: AssetType; size?: number }) {
  switch (type) {
    case "texture": return <Image size={size} />;
    case "model":   return <Box size={size} />;
    case "audio":   return <Volume2 size={size} />;
    case "video":   return <Video size={size} />;
    default:        return <File size={size} />;
  }
}

export function AssetPreview() {
  const { t } = useTranslation();
  const { selectedAsset, setSelectedAsset, scanResult } = useProjectStore();
  const { tags, assetTags, addTagToAsset, removeTagFromAsset } = useTagsStore();
  const externalEditors = useSettingsStore((s) => s.externalEditors);
  const setDepGraphOpen = useUiStore((s) => s.setDepGraphOpen);
  const setTagManagerOpen = useUiStore((s) => s.setTagManagerOpen);
  const [thumbnail, setThumbnail] = useState<string | null>(null);
  const [loadingThumbnail, setLoadingThumbnail] = useState(false);
  const [copiedPath, setCopiedPath] = useState(false);
  const [copiedGuid, setCopiedGuid] = useState(false);
  const [lightboxOpen, setLightboxOpen] = useState(false);
  const [modelLightboxOpen, setModelLightboxOpen] = useState(false);
  const [showTagPicker, setShowTagPicker] = useState(false);
  // Inline error for failed open/show actions. Auto-clears after 3s so
  // it stays out of the way without needing a global toast system.
  const [errorMsg, setErrorMsg] = useState<string | null>(null);

  useEffect(() => {
    if (!errorMsg) return;
    const handle = setTimeout(() => setErrorMsg(null), 3000);
    return () => clearTimeout(handle);
  }, [errorMsg]);

  // Mirror the fullscreen lightboxes into uiStore so global shortcut handlers are
  // gated by `isBlockingOverlayOpen` while one is up. Without it, Del inside a
  // lightbox opened a delete-confirm dialog hidden behind it.
  const setLightboxUiOpen = useUiStore((s) => s.setLightboxOpen);
  const anyLightboxOpen = lightboxOpen || modelLightboxOpen;
  useEffect(() => {
    setLightboxUiOpen(anyLightboxOpen);
    // Unmount while open (asset deselected / project switched): unblock.
    return () => setLightboxUiOpen(false);
  }, [anyLightboxOpen, setLightboxUiOpen]);

  // Settled copy of `selectedAsset`, read only by the heavy media area and its
  // lightboxes; everything else reads `selectedAsset` directly. The heavy area
  // lagging the metadata by one settle interval is the point, not a bug.
  const [settledAsset, setSettledAsset] = useState<AssetInfo | null>(null);
  // Mirrors settledAsset so the effect below can tell "nothing settled yet" from
  // "something settled, another change pending" without putting settledAsset in
  // the dependency array, which would re-fire the effect on every update.
  const settledAssetRef = useRef<AssetInfo | null>(null);
  useEffect(() => {
    if (!selectedAsset) {
      settledAssetRef.current = null;
      setSettledAsset(null);
      return;
    }

    if (settledAssetRef.current === null) {
      // Leading edge: the panel was empty, so the first pick appears
      // immediately rather than paying for a delay meant to absorb a run
      // of further changes that hasn't happened yet.
      settledAssetRef.current = selectedAsset;
      setSettledAsset(selectedAsset);
      return;
    }

    const handle = setTimeout(() => {
      settledAssetRef.current = selectedAsset;
      setSettledAsset(selectedAsset);
    }, SETTLED_ASSET_DEBOUNCE_MS);
    return () => clearTimeout(handle);
  }, [selectedAsset]);

  // Look up an editor mapping for the selected asset's extension. The
  // header ⤴ button's behavior switches between "open in <editor>" and
  // "open with default app" based on whether this is set.
  const selectedExt = selectedAsset ? getExtension(selectedAsset.path) : "";
  const mappedEditorPath = selectedExt ? externalEditors[selectedExt] : undefined;
  const mappedEditorName = mappedEditorPath
    ? getEditorDisplayName(mappedEditorPath)
    : undefined;

  const currentAssetTags = selectedAsset ? (assetTags[selectedAsset.path] || []) : [];

  // Derived from the settled asset, not the live selection — see the
  // settledAsset declaration above. Only consumed by the heavy media area
  // and its lightboxes below.
  const settledIsVideo =
    settledAsset && VIDEO_EXTENSIONS.includes(settledAsset.extension.toLowerCase());
  const settledIs3DModel =
    settledAsset && MODEL_3D_EXTENSIONS.includes(settledAsset.extension.toLowerCase());

  useEffect(() => {
    if (!settledAsset || settledAsset.asset_type !== "texture") {
      setThumbnail(null);
      setLoadingThumbnail(false);
      return;
    }

    // Stale-response guard: a slow thumbnail for the previously settled asset must
    // not land on top of the current one's — the ImageLightbox fallback consumes
    // this state too, so a stale write would survive into the lightbox.
    let cancelled = false;
    const loadThumbnail = async () => {
      setLoadingThumbnail(true);
      try {
        const base64 = await invoke<string>("get_thumbnail", {
          path: settledAsset.path,
          size: 256,
        });
        if (!cancelled) setThumbnail(base64);
      } catch (err) {
        // Thumbnail failure always falls back to the type-icon placeholder, so this
        // logs at debug regardless of cause: unsupported extension, codec gap,
        // corrupt file or IO error are none of them actionable from a console.
        console.debug("Thumbnail not available:", err);
        if (!cancelled) setThumbnail(null);
      } finally {
        if (!cancelled) setLoadingThumbnail(false);
      }
    };

    loadThumbnail();
    return () => {
      cancelled = true;
    };
    // `modified` re-fetches when the watcher re-parses the selected file after an
    // external edit; the backend disk cache is mtime-keyed, so this returns the
    // regenerated image rather than the stale one.
  }, [settledAsset?.path, settledAsset?.modified]);

  // Unity structure (component list + GUID reference count) for prefab and scene
  // files, parsed on demand. Same stale-response guard as the thumbnail effect;
  // `modified` re-parses after external edits.
  const [unityFileInfo, setUnityFileInfo] = useState<UnityFileInfo | null>(null);
  const isUnityStructureFile =
    scanResult?.project_type === "unity" &&
    !!selectedAsset &&
    ["prefab", "unity"].includes(selectedAsset.extension.toLowerCase());
  useEffect(() => {
    if (!isUnityStructureFile || !selectedAsset) {
      setUnityFileInfo(null);
      return;
    }
    // Clear immediately so a prefab→prefab switch never shows the previous
    // file's components during the (fast, but async) re-parse.
    setUnityFileInfo(null);
    let cancelled = false;
    (async () => {
      try {
        const info = await invoke<UnityFileInfo | null>("get_unity_file_info", {
          path: selectedAsset.path,
        });
        if (!cancelled) setUnityFileInfo(info);
      } catch {
        // Unparseable / unreadable file: the section simply doesn't render.
        if (!cancelled) setUnityFileInfo(null);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [isUnityStructureFile, selectedAsset?.path, selectedAsset?.modified]);

  // Goes through the clipboard plugin, like AssetList's context menu. Not a
  // portability nicety: WebKit gates `navigator.clipboard.writeText` on transient
  // activation, so one `await` in front of it moves the write past that window.
  const copyToClipboard = async (text: string, type: "path" | "guid") => {
    try {
      await writeText(text);
      if (type === "path") {
        setCopiedPath(true);
        setTimeout(() => setCopiedPath(false), 2000);
      } else {
        setCopiedGuid(true);
        setTimeout(() => setCopiedGuid(false), 2000);
      }
    } catch (err) {
      console.error("Failed to copy:", err);
      setErrorMsg(t("assetPreview.copyFailed", { reason: String(err) }));
    }
  };

  // The header ⤴ button: fall back through the editor mapping → default
  // app chain so a configured editor wins, but missing editors don't
  // become a dead end.
  const handleOpen = async () => {
    if (!selectedAsset) return;
    try {
      if (mappedEditorPath) {
        await invoke("open_in_editor", {
          path: selectedAsset.path,
          editor: mappedEditorPath,
        });
      } else {
        await invoke("open_with_default_app", { path: selectedAsset.path });
      }
    } catch (err) {
      console.error("Failed to open:", err);
      setErrorMsg(
        t("assetPreview.openFailed", { reason: String(err) })
      );
    }
  };

  const revealInFileManager = async () => {
    if (!selectedAsset) return;
    try {
      await invoke("show_in_file_manager", { path: selectedAsset.path });
    } catch (err) {
      console.error("Failed to show in file manager:", err);
      setErrorMsg(
        t("assetPreview.openFailed", { reason: String(err) })
      );
    }
  };

  const getTypeLabel = (type: string): string => {
    const key = `assetTypes.${type}` as const;
    return t(key);
  };

  if (!selectedAsset) {
    return (
      <aside className="tc-preview">
        <div className="tc-preview-empty">
          <FileText size={42} style={{ opacity: 0.3, marginBottom: 8 }} />
          <p style={{ fontSize: 12.5 }}>{t("assetPreview.selectAsset")}</p>
        </div>
      </aside>
    );
  }

  const renderPreview = () => {
    if (!settledAsset) {
      // Only reachable for the single render between the very first selection and
      // the leading-edge effect committing it — there is no previous asset to keep
      // showing yet.
      return null;
    }

    if (settledIsVideo) {
      return <VideoPlayer filePath={settledAsset.path} />;
    }

    if (settledAsset.asset_type === "texture") {
      if (loadingThumbnail) {
        return (
          <div
            style={{
              width: "100%",
              aspectRatio: "1 / 1",
              background: "var(--panel-2)",
              border: "1px solid var(--line)",
              borderRadius: 8,
              display: "grid",
              placeItems: "center",
              color: "var(--text-3)",
              fontSize: 12,
            }}
          >
            {t("assetPreview.loading")}
          </div>
        );
      }
      if (thumbnail) {
        return (
          <div className="relative group">
            <img
              src={`data:image/png;base64,${thumbnail}`}
              alt={settledAsset.name}
              style={{
                width: "100%",
                aspectRatio: "1 / 1",
                objectFit: "contain",
                background: "var(--panel-2)",
                border: "1px solid var(--line)",
                borderRadius: 8,
                cursor: "pointer",
              }}
              onClick={() => setLightboxOpen(true)}
            />
            <div
              className="absolute inset-0 flex items-center justify-center bg-black/30 opacity-0 group-hover:opacity-100 transition-opacity cursor-pointer"
              style={{ borderRadius: 8 }}
              onClick={() => setLightboxOpen(true)}
            >
              <Maximize2 size={22} className="text-white" />
            </div>
          </div>
        );
      }
      return (
        <div
          style={{
            width: "100%",
            aspectRatio: "1 / 1",
            background: "var(--panel-2)",
            border: "1px solid var(--line)",
            borderRadius: 8,
            display: "grid",
            placeItems: "center",
          }}
        >
          <Image size={42} style={{ color: "var(--c-texture)", opacity: 0.5 }} />
        </div>
      );
    }

    if (settledAsset.asset_type === "audio") {
      return <AudioPlayer filePath={settledAsset.path} />;
    }

    if (settledAsset.asset_type === "model") {
      if (settledIs3DModel) {
        return (
          <ModelViewer3D
            filePath={settledAsset.path}
            extension={settledAsset.extension}
            vertexCount={settledAsset.metadata?.vertex_count}
            onFullscreen={() => setModelLightboxOpen(true)}
          />
        );
      }
      return (
        <div
          style={{
            width: "100%",
            aspectRatio: "1 / 1",
            background: "var(--panel-2)",
            border: "1px solid var(--line)",
            borderRadius: 8,
            display: "grid",
            placeItems: "center",
          }}
        >
          <Box size={42} style={{ color: "var(--c-model)", opacity: 0.5 }} />
        </div>
      );
    }

    return (
      <div
        style={{
          width: "100%",
          aspectRatio: "1 / 1",
          background: "var(--panel-2)",
          border: "1px solid var(--line)",
          borderRadius: 8,
          display: "grid",
          placeItems: "center",
        }}
      >
        <FileText size={42} style={{ color: "var(--text-4)" }} />
      </div>
    );
  };

  const metadata = selectedAsset.metadata;
  const projectType = scanResult?.project_type;

  const getChannelLabel = (channels: number): string => {
    if (channels === 1) return t("assetPreview.mono");
    if (channels === 2) return t("assetPreview.stereo");
    return String(channels);
  };

  return (
    <aside className="tc-preview">
      <div className="tc-preview-head">
        <div className="tc-preview-title">
          <span
            className="tc-asset-glyph"
            data-type={selectedAsset.asset_type}
            style={{ width: 18, height: 18 }}
          >
            <GlyphIcon type={selectedAsset.asset_type} />
          </span>
          <span className="tc-name" title={selectedAsset.name}>
            {selectedAsset.name}
          </span>
        </div>
        <div className="tc-preview-actions">
          <button
            onClick={revealInFileManager}
            className="tc-icon-btn"
            style={{ width: 26, height: 26 }}
            title={t("contextMenu.revealInFileManager")}
          >
            <FolderOpen size={13} />
          </button>
          <button
            onClick={handleOpen}
            className="tc-icon-btn"
            style={{ width: 26, height: 26 }}
            title={
              mappedEditorName
                ? t("assetPreview.openInEditor", { name: mappedEditorName })
                : t("assetPreview.openWithDefaultApp")
            }
          >
            <ExternalLink size={13} />
          </button>
          <button
            onClick={() => setSelectedAsset(null)}
            className="tc-icon-btn"
            style={{ width: 26, height: 26 }}
            title={t("assetPreview.close")}
          >
            <X size={13} />
          </button>
        </div>
      </div>

      {errorMsg && (
        <div
          className="px-4 py-1.5 text-xs"
          style={{
            color: "var(--err)",
            borderBottom: "1px solid var(--line)",
            background:
              "color-mix(in oklch, var(--err) 8%, transparent)",
          }}
        >
          {errorMsg}
        </div>
      )}

      <div className="tc-preview-body">
        {/* Preview canvas */}
        <div style={{ padding: "12px 14px" }}>{renderPreview()}</div>

        {/* Basic info */}
        <div className="tc-meta-section">
          <div className="tc-meta-label">{t("assetPreview.basicInfo")}</div>
          <dl className="tc-kv-grid">
            <dt>{t("assetPreview.type")}</dt>
            <dd>{getTypeLabel(selectedAsset.asset_type)}</dd>
            <dt>{t("assetPreview.extension")}</dt>
            <dd>.{selectedAsset.extension}</dd>
            <dt>{t("assetPreview.size")}</dt>
            <dd>{formatFileSize(selectedAsset.size)}</dd>
            {metadata?.dcc_source_kind && (
              <>
                <dt>{t("assetPreview.dccSource")}</dt>
                <dd>{dccSourceLabel(metadata.dcc_source_kind)}</dd>
              </>
            )}
          </dl>
        </div>

        {/* Image metadata */}
        {selectedAsset.asset_type === "texture" && metadata && (
          <div className="tc-meta-section">
            <div className="tc-meta-label">{t("assetPreview.imageInfo")}</div>
            <dl className="tc-kv-grid">
              {metadata.width && metadata.height && (
                <>
                  <dt>{t("assetPreview.dimensions")}</dt>
                  <dd>
                    {metadata.width} × {metadata.height}
                  </dd>
                </>
              )}
              {metadata.has_alpha !== undefined && (
                <>
                  <dt>{t("assetPreview.hasAlpha")}</dt>
                  <dd>{metadata.has_alpha ? t("assetPreview.yes") : t("assetPreview.no")}</dd>
                </>
              )}
              {metadata.color_space && (
                <>
                  <dt>{t("assetPreview.colorSpace")}</dt>
                  <dd>{metadata.color_space}</dd>
                </>
              )}
              {metadata.mipmap_count !== undefined && (
                <>
                  <dt>{t("assetPreview.mipmaps")}</dt>
                  <dd>
                    {metadata.mipmap_count === 1
                      ? t("assetPreview.mipmapsNone")
                      : metadata.mipmap_count}
                  </dd>
                </>
              )}
            </dl>
          </div>
        )}

        {/* Model metadata */}
        {selectedAsset.asset_type === "model" && metadata && (
          <div className="tc-meta-section">
            <div className="tc-meta-label">{t("assetPreview.modelInfo")}</div>
            <dl className="tc-kv-grid">
              {metadata.vertex_count !== undefined && (
                <>
                  <dt>{t("assetPreview.vertices")}</dt>
                  <dd>{metadata.vertex_count.toLocaleString()}</dd>
                </>
              )}
              {metadata.face_count !== undefined && (
                <>
                  <dt>{t("assetPreview.faces")}</dt>
                  <dd>{metadata.face_count.toLocaleString()}</dd>
                </>
              )}
              {metadata.material_count !== undefined && (
                <>
                  <dt>{t("assetPreview.materials")}</dt>
                  <dd>{metadata.material_count}</dd>
                </>
              )}
            </dl>
          </div>
        )}

        {/* Video metadata */}
        {selectedAsset.asset_type === "video" && metadata && (
          <div className="tc-meta-section">
            <div className="tc-meta-label">{t("assetPreview.videoInfo")}</div>
            <dl className="tc-kv-grid">
              {metadata.duration_secs !== undefined && (
                <>
                  <dt>{t("assetPreview.duration")}</dt>
                  <dd>{formatDuration(metadata.duration_secs)}</dd>
                </>
              )}
              {metadata.width !== undefined && metadata.height !== undefined && (
                <>
                  <dt>{t("assetPreview.resolution")}</dt>
                  <dd>
                    {metadata.width} × {metadata.height}
                  </dd>
                </>
              )}
              {metadata.framerate !== undefined && metadata.framerate > 0 && (
                <>
                  <dt>{t("assetPreview.framerate")}</dt>
                  <dd>{metadata.framerate.toFixed(2)} fps</dd>
                </>
              )}
              {metadata.video_codec && (
                <>
                  <dt>{t("assetPreview.codec")}</dt>
                  <dd>{metadata.video_codec}</dd>
                </>
              )}
            </dl>
          </div>
        )}

        {/* Audio metadata */}
        {selectedAsset.asset_type === "audio" && metadata && (
          <div className="tc-meta-section">
            <div className="tc-meta-label">{t("assetPreview.audioInfo")}</div>
            <dl className="tc-kv-grid">
              {metadata.duration_secs !== undefined && (
                <>
                  <dt>{t("assetPreview.duration")}</dt>
                  <dd>{formatDuration(metadata.duration_secs)}</dd>
                </>
              )}
              {metadata.sample_rate !== undefined && (
                <>
                  <dt>{t("assetPreview.sampleRate")}</dt>
                  <dd>{(metadata.sample_rate / 1000).toFixed(1)} kHz</dd>
                </>
              )}
              {metadata.channels !== undefined && (
                <>
                  <dt>{t("assetPreview.channels")}</dt>
                  <dd>{getChannelLabel(metadata.channels)}</dd>
                </>
              )}
              {metadata.bit_depth !== undefined && (
                <>
                  <dt>{t("assetPreview.bitDepth")}</dt>
                  <dd>{metadata.bit_depth}-bit</dd>
                </>
              )}
            </dl>
          </div>
        )}

        {/* Tags */}
        <div className="tc-meta-section">
          <div className="tc-meta-label">{t("tags.title")}</div>
          <div style={{ display: "flex", flexDirection: "column", gap: 8 }}>
            <div style={{ display: "flex", flexWrap: "wrap", gap: 4 }}>
              {currentAssetTags.map((tag) => (
                <span
                  key={tag.id}
                  className="group"
                  style={{
                    display: "inline-flex",
                    alignItems: "center",
                    gap: 4,
                    padding: "2px 6px 2px 7px",
                    borderRadius: 999,
                    fontSize: 11,
                    backgroundColor: `${tag.color}1F`,
                    color: tag.color,
                    border: `1px solid ${tag.color}33`,
                  }}
                >
                  <span
                    style={{
                      width: 6,
                      height: 6,
                      borderRadius: "50%",
                      backgroundColor: tag.color,
                    }}
                  />
                  {tag.name}
                  <button
                    onClick={() => removeTagFromAsset(selectedAsset.path, tag.id)}
                    className="opacity-0 group-hover:opacity-100 transition-opacity"
                    style={{
                      background: "transparent",
                      border: 0,
                      color: "inherit",
                      cursor: "pointer",
                      padding: 0,
                      marginLeft: 2,
                      display: "inline-flex",
                    }}
                  >
                    <X size={10} />
                  </button>
                </span>
              ))}
              {currentAssetTags.length === 0 && (
                <span style={{ fontSize: 11, color: "var(--text-3)", fontStyle: "italic" }}>
                  {t("tags.noTags")}
                </span>
              )}
            </div>
            <div style={{ position: "relative" }}>
              <button
                onClick={() => setShowTagPicker(!showTagPicker)}
                className="tc-batch-action"
                style={{ height: 24, padding: "0 8px", fontSize: 11 }}
              >
                <Plus size={11} />
                {t("tags.addTag")}
              </button>
              {showTagPicker && (
                <div
                  style={{
                    position: "absolute",
                    left: 0,
                    top: "calc(100% + 4px)",
                    zIndex: 50,
                    minWidth: 160,
                    maxHeight: 200,
                    overflowY: "auto",
                    padding: "4px 0",
                    background: "var(--panel)",
                    border: "1px solid var(--line)",
                    borderRadius: 8,
                    boxShadow: "var(--shadow-pop)",
                  }}
                >
                  {tags.filter((tag) => !currentAssetTags.some((tt) => tt.id === tag.id)).length === 0 ? (
                    <div
                      style={{
                        padding: "6px 12px",
                        fontSize: 11,
                        color: "var(--text-3)",
                        fontStyle: "italic",
                      }}
                    >
                      {tags.length === 0 ? t("tags.noTags") : t("tags.allTagsAdded", "All tags added")}
                    </div>
                  ) : (
                    tags
                      .filter((tag) => !currentAssetTags.some((tt) => tt.id === tag.id))
                      .map((tag) => (
                        <button
                          key={tag.id}
                          onClick={() => {
                            addTagToAsset(selectedAsset.path, tag.id);
                            setShowTagPicker(false);
                          }}
                          style={{
                            display: "flex",
                            alignItems: "center",
                            gap: 8,
                            width: "100%",
                            padding: "6px 12px",
                            fontSize: 11.5,
                            textAlign: "left",
                            color: "var(--text)",
                            background: "transparent",
                            border: 0,
                            cursor: "pointer",
                          }}
                          onMouseEnter={(e) =>
                            (e.currentTarget.style.background = "var(--panel-hover)")
                          }
                          onMouseLeave={(e) => (e.currentTarget.style.background = "transparent")}
                        >
                          <span
                            style={{
                              width: 10,
                              height: 10,
                              borderRadius: "50%",
                              backgroundColor: tag.color,
                              flexShrink: 0,
                            }}
                          />
                          <span style={{ color: tag.color }}>{tag.name}</span>
                        </button>
                      ))
                  )}
                  {/* Always-available exit: with zero tags the list above is a
                      dead end ("No tags yet") unless the user can hop straight
                      into the manager to create one — mirrors TagSelector. */}
                  <div style={{ borderTop: "1px solid var(--line)", marginTop: 4, paddingTop: 4 }}>
                    <button
                      onClick={() => {
                        setShowTagPicker(false);
                        setTagManagerOpen(true);
                      }}
                      style={{
                        display: "flex",
                        alignItems: "center",
                        gap: 8,
                        width: "100%",
                        padding: "6px 12px",
                        fontSize: 11.5,
                        textAlign: "left",
                        color: "var(--text-2)",
                        background: "transparent",
                        border: 0,
                        cursor: "pointer",
                      }}
                      onMouseEnter={(e) =>
                        (e.currentTarget.style.background = "var(--panel-hover)")
                      }
                      onMouseLeave={(e) => (e.currentTarget.style.background = "transparent")}
                    >
                      <Settings size={11} />
                      {t("tags.manageTitle")}
                    </button>
                  </div>
                </div>
              )}
            </div>
          </div>
        </div>

        {/* Unity GUID */}
        {projectType === "unity" && selectedAsset.unity_guid && (
          <div className="tc-meta-section">
            <div className="tc-meta-label">{t("assetPreview.unity")}</div>
            <div className="tc-guid-row">
              <span style={{ color: "var(--text-3)" }}>{t("assetPreview.guid")}</span>
              <code>{selectedAsset.unity_guid}</code>
              <button
                onClick={() => copyToClipboard(selectedAsset.unity_guid!, "guid")}
                className="tc-guid-copy"
                title={t("assetPreview.copyGuid")}
              >
                {copiedGuid ? <Check size={12} style={{ color: "var(--ok)" }} /> : <Copy size={11} />}
              </button>
            </div>
          </div>
        )}

        {/* Unity structure: component types + GUID reference count for
            prefab/scene files (backend parses on demand, sorted). */}
        {unityFileInfo &&
          (unityFileInfo.components.length > 0 ||
            unityFileInfo.references.length > 0) && (
            <div className="tc-meta-section">
              <div className="tc-meta-label">{t("assetPreview.components")}</div>
              {unityFileInfo.components.length > 0 && (
                <div
                  style={{
                    display: "flex",
                    flexWrap: "wrap",
                    gap: 4,
                    marginBottom: 8,
                  }}
                >
                  {unityFileInfo.components.map((c) => (
                    <span key={c} className="tc-mini-chip">
                      {c}
                    </span>
                  ))}
                </div>
              )}
              <dl className="tc-kv-grid">
                <dt>{t("assetPreview.references")}</dt>
                <dd>{unityFileInfo.references.length}</dd>
              </dl>
            </div>
          )}

        {/* Dependency graph (Unity / Godot — both reference-based engines) */}
        {(projectType === "unity" || projectType === "godot") && (
          <div className="tc-meta-section">
            <button
              onClick={() => setDepGraphOpen(true, selectedAsset.path)}
              title={t("assetPreview.viewDependencies")}
              style={{
                display: "flex",
                alignItems: "center",
                justifyContent: "center",
                gap: 6,
                width: "100%",
                padding: "6px 10px",
                fontSize: 12,
                borderRadius: 6,
                border: "1px solid var(--line)",
                background: "var(--panel-2)",
                color: "var(--text-2)",
                cursor: "pointer",
              }}
            >
              <Network size={13} />
              {t("assetPreview.viewDependencies")}
            </button>
          </div>
        )}

        {/* Path */}
        <div className="tc-meta-section">
          <div className="tc-meta-label">{t("assetPreview.path")}</div>
          <div className="tc-path-row">
            <span>{selectedAsset.path}</span>
            <button
              onClick={() => copyToClipboard(selectedAsset.path, "path")}
              className="tc-guid-copy"
              title={t("assetPreview.copyPath")}
            >
              {copiedPath ? <Check size={12} style={{ color: "var(--ok)" }} /> : <Copy size={11} />}
            </button>
          </div>
        </div>
      </div>

      {/* Image Lightbox — mirrors the settled asset the inline thumbnail
          above (whose click opens this) is actually showing. */}
      {settledAsset && thumbnail && (
        <ImageLightbox
          isOpen={lightboxOpen}
          imageSrc={convertFileSrc(settledAsset.path)}
          fallbackSrc={`data:image/png;base64,${thumbnail}`}
          imageName={settledAsset.name}
          onClose={() => setLightboxOpen(false)}
        />
      )}

      {/* 3D Model Lightbox — same settled-asset mirroring: its fullscreen
          button lives on the settled asset's inline viewer. */}
      {settledAsset && settledIs3DModel && (
        <ModelLightbox
          isOpen={modelLightboxOpen}
          filePath={settledAsset.path}
          extension={settledAsset.extension}
          vertexCount={settledAsset.metadata?.vertex_count}
          modelName={settledAsset.name}
          onClose={() => setModelLightboxOpen(false)}
        />
      )}
    </aside>
  );
}
