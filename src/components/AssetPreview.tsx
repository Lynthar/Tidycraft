import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
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
} from "lucide-react";
import { useTranslation } from "react-i18next";
import { useProjectStore } from "../stores/projectStore";
import { useTagsStore } from "../stores/tagsStore";
import { formatFileSize, formatDuration } from "../lib/utils";
import { VideoPlayer } from "./VideoPlayer";
import { AudioPlayer } from "./AudioPlayer";
import { ImageLightbox } from "./ImageLightbox";
import { ModelViewer3D } from "./ModelViewer3D";
import { ModelLightbox } from "./ModelLightbox";
import type { AssetType } from "../types/asset";

const VIDEO_EXTENSIONS = ["mp4", "webm", "mov", "avi", "mkv", "m4v"];
const MODEL_3D_EXTENSIONS = ["gltf", "glb", "fbx", "obj", "dae"];

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
  const [thumbnail, setThumbnail] = useState<string | null>(null);
  const [loadingThumbnail, setLoadingThumbnail] = useState(false);
  const [copiedPath, setCopiedPath] = useState(false);
  const [copiedGuid, setCopiedGuid] = useState(false);
  const [lightboxOpen, setLightboxOpen] = useState(false);
  const [modelLightboxOpen, setModelLightboxOpen] = useState(false);
  const [showTagPicker, setShowTagPicker] = useState(false);

  const currentAssetTags = selectedAsset ? (assetTags[selectedAsset.path] || []) : [];

  const isVideo =
    selectedAsset && VIDEO_EXTENSIONS.includes(selectedAsset.extension.toLowerCase());
  const is3DModel =
    selectedAsset && MODEL_3D_EXTENSIONS.includes(selectedAsset.extension.toLowerCase());

  useEffect(() => {
    if (!selectedAsset || selectedAsset.asset_type !== "texture") {
      setThumbnail(null);
      return;
    }

    const loadThumbnail = async () => {
      setLoadingThumbnail(true);
      try {
        const base64 = await invoke<string>("get_thumbnail", {
          path: selectedAsset.path,
          size: 256,
        });
        setThumbnail(base64);
      } catch (err) {
        console.error("Failed to load thumbnail:", err);
        setThumbnail(null);
      } finally {
        setLoadingThumbnail(false);
      }
    };

    loadThumbnail();
  }, [selectedAsset?.path]);

  const copyToClipboard = async (text: string, type: "path" | "guid") => {
    try {
      await navigator.clipboard.writeText(text);
      if (type === "path") {
        setCopiedPath(true);
        setTimeout(() => setCopiedPath(false), 2000);
      } else {
        setCopiedGuid(true);
        setTimeout(() => setCopiedGuid(false), 2000);
      }
    } catch (err) {
      console.error("Failed to copy:", err);
    }
  };

  const openWithDefaultApp = async () => {
    if (!selectedAsset) return;
    try {
      await invoke("open_with_default_app", { path: selectedAsset.path });
    } catch (err) {
      console.error("Failed to open with default app:", err);
    }
  };

  const revealInFinder = async () => {
    if (!selectedAsset) return;
    try {
      await invoke("reveal_in_finder", { path: selectedAsset.path });
    } catch (err) {
      console.error("Failed to reveal in finder:", err);
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
    if (isVideo) {
      return <VideoPlayer filePath={selectedAsset.path} />;
    }

    if (selectedAsset.asset_type === "texture") {
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
              alt={selectedAsset.name}
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

    if (selectedAsset.asset_type === "audio") {
      return <AudioPlayer filePath={selectedAsset.path} />;
    }

    if (selectedAsset.asset_type === "model") {
      if (is3DModel) {
        return (
          <ModelViewer3D
            filePath={selectedAsset.path}
            extension={selectedAsset.extension}
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
            onClick={revealInFinder}
            className="tc-icon-btn"
            style={{ width: 26, height: 26 }}
            title={t("contextMenu.revealInFinder")}
          >
            <FolderOpen size={13} />
          </button>
          <button
            onClick={openWithDefaultApp}
            className="tc-icon-btn"
            style={{ width: 26, height: 26 }}
            title={t("assetPreview.openWithDefaultApp")}
          >
            <ExternalLink size={13} />
          </button>
          <button
            onClick={() => setSelectedAsset(null)}
            className="tc-icon-btn"
            style={{ width: 26, height: 26 }}
            title="Close"
          >
            <X size={13} />
          </button>
        </div>
      </div>

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

      {/* Image Lightbox */}
      {thumbnail && (
        <ImageLightbox
          isOpen={lightboxOpen}
          imageSrc={`data:image/png;base64,${thumbnail}`}
          imageName={selectedAsset.name}
          onClose={() => setLightboxOpen(false)}
        />
      )}

      {/* 3D Model Lightbox */}
      {is3DModel && (
        <ModelLightbox
          isOpen={modelLightboxOpen}
          filePath={selectedAsset.path}
          extension={selectedAsset.extension}
          modelName={selectedAsset.name}
          onClose={() => setModelLightboxOpen(false)}
        />
      )}
    </aside>
  );
}
