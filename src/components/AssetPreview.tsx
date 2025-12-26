import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Image, Box, Volume2, FileText, X, Copy, Check } from "lucide-react";
import { useProjectStore } from "../stores/projectStore";
import { formatFileSize, getAssetTypeLabel, formatDuration } from "../lib/utils";

export function AssetPreview() {
  const { selectedAsset, setSelectedAsset, scanResult } = useProjectStore();
  const [thumbnail, setThumbnail] = useState<string | null>(null);
  const [loadingThumbnail, setLoadingThumbnail] = useState(false);
  const [copiedPath, setCopiedPath] = useState(false);
  const [copiedGuid, setCopiedGuid] = useState(false);

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

  if (!selectedAsset) {
    return (
      <div className="w-72 bg-card-bg border-l border-border flex flex-col items-center justify-center text-text-secondary p-4">
        <FileText size={48} className="opacity-30 mb-2" />
        <p className="text-sm text-center">Select an asset to view details</p>
      </div>
    );
  }

  const renderPreview = () => {
    if (selectedAsset.asset_type === "texture") {
      if (loadingThumbnail) {
        return (
          <div className="w-full aspect-square bg-background rounded flex items-center justify-center">
            <span className="text-text-secondary text-sm">Loading...</span>
          </div>
        );
      }
      if (thumbnail) {
        return (
          <img
            src={`data:image/png;base64,${thumbnail}`}
            alt={selectedAsset.name}
            className="w-full aspect-square object-contain bg-background rounded"
          />
        );
      }
      return (
        <div className="w-full aspect-square bg-background rounded flex items-center justify-center">
          <Image size={48} className="text-green-400/50" />
        </div>
      );
    }

    if (selectedAsset.asset_type === "model") {
      return (
        <div className="w-full aspect-square bg-background rounded flex items-center justify-center">
          <Box size={48} className="text-blue-400/50" />
        </div>
      );
    }

    if (selectedAsset.asset_type === "audio") {
      return (
        <div className="w-full aspect-square bg-background rounded flex items-center justify-center">
          <Volume2 size={48} className="text-yellow-400/50" />
        </div>
      );
    }

    return (
      <div className="w-full aspect-square bg-background rounded flex items-center justify-center">
        <FileText size={48} className="text-gray-400/50" />
      </div>
    );
  };

  const metadata = selectedAsset.metadata;
  const projectType = scanResult?.project_type;

  return (
    <div className="w-72 bg-card-bg border-l border-border flex flex-col overflow-hidden">
      {/* Header */}
      <div className="flex items-center justify-between p-3 border-b border-border">
        <h3 className="font-medium text-sm truncate flex-1">{selectedAsset.name}</h3>
        <button
          onClick={() => setSelectedAsset(null)}
          className="p-1 hover:bg-background rounded text-text-secondary hover:text-text-primary"
        >
          <X size={14} />
        </button>
      </div>

      {/* Preview */}
      <div className="p-3 border-b border-border">{renderPreview()}</div>

      {/* Details */}
      <div className="flex-1 overflow-auto p-3">
        <div className="space-y-3 text-sm">
          {/* Basic Info */}
          <div>
            <h4 className="text-text-secondary text-xs uppercase mb-2">Basic Info</h4>
            <div className="space-y-1.5">
              <div className="flex justify-between">
                <span className="text-text-secondary">Type:</span>
                <span className="text-text-primary">{getAssetTypeLabel(selectedAsset.asset_type)}</span>
              </div>
              <div className="flex justify-between">
                <span className="text-text-secondary">Extension:</span>
                <span className="text-text-primary">.{selectedAsset.extension}</span>
              </div>
              <div className="flex justify-between">
                <span className="text-text-secondary">Size:</span>
                <span className="text-text-primary">{formatFileSize(selectedAsset.size)}</span>
              </div>
            </div>
          </div>

          {/* Image Metadata */}
          {selectedAsset.asset_type === "texture" && metadata && (
            <div>
              <h4 className="text-text-secondary text-xs uppercase mb-2">Image Info</h4>
              <div className="space-y-1.5">
                {metadata.width && metadata.height && (
                  <div className="flex justify-between">
                    <span className="text-text-secondary">Dimensions:</span>
                    <span className="text-text-primary">
                      {metadata.width} x {metadata.height}
                    </span>
                  </div>
                )}
                {metadata.has_alpha !== undefined && (
                  <div className="flex justify-between">
                    <span className="text-text-secondary">Has Alpha:</span>
                    <span className="text-text-primary">{metadata.has_alpha ? "Yes" : "No"}</span>
                  </div>
                )}
              </div>
            </div>
          )}

          {/* Model Metadata */}
          {selectedAsset.asset_type === "model" && metadata && (
            <div>
              <h4 className="text-text-secondary text-xs uppercase mb-2">Model Info</h4>
              <div className="space-y-1.5">
                {metadata.vertex_count !== undefined && (
                  <div className="flex justify-between">
                    <span className="text-text-secondary">Vertices:</span>
                    <span className="text-text-primary">
                      {metadata.vertex_count.toLocaleString()}
                    </span>
                  </div>
                )}
                {metadata.face_count !== undefined && (
                  <div className="flex justify-between">
                    <span className="text-text-secondary">Faces:</span>
                    <span className="text-text-primary">
                      {metadata.face_count.toLocaleString()}
                    </span>
                  </div>
                )}
                {metadata.material_count !== undefined && (
                  <div className="flex justify-between">
                    <span className="text-text-secondary">Materials:</span>
                    <span className="text-text-primary">{metadata.material_count}</span>
                  </div>
                )}
              </div>
            </div>
          )}

          {/* Audio Metadata */}
          {selectedAsset.asset_type === "audio" && metadata && (
            <div>
              <h4 className="text-text-secondary text-xs uppercase mb-2">Audio Info</h4>
              <div className="space-y-1.5">
                {metadata.duration_secs !== undefined && (
                  <div className="flex justify-between">
                    <span className="text-text-secondary">Duration:</span>
                    <span className="text-text-primary">
                      {formatDuration(metadata.duration_secs)}
                    </span>
                  </div>
                )}
                {metadata.sample_rate !== undefined && (
                  <div className="flex justify-between">
                    <span className="text-text-secondary">Sample Rate:</span>
                    <span className="text-text-primary">
                      {(metadata.sample_rate / 1000).toFixed(1)} kHz
                    </span>
                  </div>
                )}
                {metadata.channels !== undefined && (
                  <div className="flex justify-between">
                    <span className="text-text-secondary">Channels:</span>
                    <span className="text-text-primary">
                      {metadata.channels === 1 ? "Mono" : metadata.channels === 2 ? "Stereo" : metadata.channels}
                    </span>
                  </div>
                )}
                {metadata.bit_depth !== undefined && (
                  <div className="flex justify-between">
                    <span className="text-text-secondary">Bit Depth:</span>
                    <span className="text-text-primary">{metadata.bit_depth}-bit</span>
                  </div>
                )}
              </div>
            </div>
          )}

          {/* Unity GUID */}
          {projectType === "unity" && selectedAsset.unity_guid && (
            <div>
              <h4 className="text-text-secondary text-xs uppercase mb-2">Unity</h4>
              <div className="space-y-1.5">
                <div className="flex items-center gap-2">
                  <span className="text-text-secondary shrink-0">GUID:</span>
                  <span className="text-text-primary font-mono text-xs truncate flex-1">
                    {selectedAsset.unity_guid}
                  </span>
                  <button
                    onClick={() => copyToClipboard(selectedAsset.unity_guid!, "guid")}
                    className="p-1 hover:bg-background rounded text-text-secondary hover:text-text-primary shrink-0"
                    title="Copy GUID"
                  >
                    {copiedGuid ? <Check size={12} className="text-success" /> : <Copy size={12} />}
                  </button>
                </div>
              </div>
            </div>
          )}

          {/* Path */}
          <div>
            <h4 className="text-text-secondary text-xs uppercase mb-2">Path</h4>
            <div className="flex items-start gap-2">
              <span className="text-text-primary text-xs font-mono break-all flex-1">
                {selectedAsset.path}
              </span>
              <button
                onClick={() => copyToClipboard(selectedAsset.path, "path")}
                className="p-1 hover:bg-background rounded text-text-secondary hover:text-text-primary shrink-0"
                title="Copy path"
              >
                {copiedPath ? <Check size={12} className="text-success" /> : <Copy size={12} />}
              </button>
            </div>
          </div>
        </div>
      </div>
    </div>
  );
}
