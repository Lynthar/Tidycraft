import { useState, useEffect, useRef, useCallback } from "react";
import { X, ZoomIn, ZoomOut, RotateCw, Maximize2, Minimize2 } from "lucide-react";
import { useTranslation } from "react-i18next";

interface ImageLightboxProps {
  isOpen: boolean;
  imageSrc: string;
  /** Full-resolution source; falls back to this (a thumbnail data-URL) if
   *  imageSrc fails to load — the Tauri asset protocol can 404 on paths with
   *  spaces / non-ASCII characters. */
  fallbackSrc?: string;
  imageName: string;
  onClose: () => void;
}

export function ImageLightbox({ isOpen, imageSrc, fallbackSrc, imageName, onClose }: ImageLightboxProps) {
  const { t } = useTranslation();
  const [scale, setScale] = useState(1);
  const [useFallback, setUseFallback] = useState(false);
  const [position, setPosition] = useState({ x: 0, y: 0 });
  const [isDragging, setIsDragging] = useState(false);
  const [dragStart, setDragStart] = useState({ x: 0, y: 0 });
  const [rotation, setRotation] = useState(0);
  const [isFitToScreen, setIsFitToScreen] = useState(true);
  const containerRef = useRef<HTMLDivElement>(null);

  // Reset state when opening
  useEffect(() => {
    if (isOpen) {
      setScale(1);
      setPosition({ x: 0, y: 0 });
      setRotation(0);
      setIsFitToScreen(true);
      setUseFallback(false);
    }
  }, [isOpen, imageSrc]);

  // Handle keyboard events
  useEffect(() => {
    if (!isOpen) return;

    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        onClose();
        return;
      }
      // Letter/number shortcuts must be unmodified: Ctrl/Cmd+R is the
      // app-level rescan chord (gated while the lightbox is open) and
      // must not double as "rotate image" here.
      if (e.ctrlKey || e.metaKey || e.altKey) return;
      switch (e.key) {
        case "+":
        case "=":
          handleZoomIn();
          break;
        case "-":
          handleZoomOut();
          break;
        case "r":
        case "R":
          handleRotate();
          break;
        case "0":
          resetView();
          break;
      }
    };

    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [isOpen, onClose]);

  // Wheel-zoom uses a non-passive native listener: React's synthetic
  // onWheel has been registered passively since React 17, which silently
  // drops preventDefault and lets the page scroll behind the lightbox.
  useEffect(() => {
    if (!isOpen) return;
    const container = containerRef.current;
    if (!container) return;
    const handleWheel = (e: WheelEvent) => {
      e.preventDefault();
      const delta = e.deltaY > 0 ? -0.1 : 0.1;
      setScale((prev) => Math.min(Math.max(0.1, prev + delta), 5));
      setIsFitToScreen(false);
    };
    container.addEventListener("wheel", handleWheel, { passive: false });
    return () => container.removeEventListener("wheel", handleWheel);
  }, [isOpen]);

  // Handle mouse drag
  const handleMouseDown = (e: React.MouseEvent) => {
    if (e.button !== 0) return; // Only left click
    setIsDragging(true);
    setDragStart({ x: e.clientX - position.x, y: e.clientY - position.y });
  };

  const handleMouseMove = useCallback(
    (e: React.MouseEvent) => {
      if (!isDragging) return;
      setPosition({
        x: e.clientX - dragStart.x,
        y: e.clientY - dragStart.y,
      });
    },
    [isDragging, dragStart]
  );

  const handleMouseUp = () => {
    setIsDragging(false);
  };

  const handleZoomIn = () => {
    setScale((prev) => Math.min(prev + 0.25, 5));
    setIsFitToScreen(false);
  };

  const handleZoomOut = () => {
    setScale((prev) => Math.max(prev - 0.25, 0.1));
    setIsFitToScreen(false);
  };

  const handleRotate = () => {
    setRotation((prev) => (prev + 90) % 360);
  };

  const resetView = () => {
    setScale(1);
    setPosition({ x: 0, y: 0 });
    setRotation(0);
    setIsFitToScreen(true);
  };

  const toggleFitToScreen = () => {
    if (isFitToScreen) {
      setScale(2);
      setIsFitToScreen(false);
    } else {
      resetView();
    }
  };

  if (!isOpen) return null;

  return (
    <div
      className="fixed inset-0 z-50 bg-black/90 flex flex-col"
      onMouseMove={handleMouseMove}
      onMouseUp={handleMouseUp}
      onMouseLeave={handleMouseUp}
    >
      {/* Header */}
      <div className="flex items-center justify-between px-4 py-3 bg-black/50 text-white">
        <span className="text-sm font-medium truncate flex-1">{imageName}</span>
        <div className="flex items-center gap-1">
          <button
            onClick={handleZoomOut}
            className="p-2 rounded hover:bg-white/10 transition-colors"
            title={t("imageLightbox.zoomOut")}
          >
            <ZoomOut size={18} />
          </button>
          <span className="text-sm w-16 text-center">{Math.round(scale * 100)}%</span>
          <button
            onClick={handleZoomIn}
            className="p-2 rounded hover:bg-white/10 transition-colors"
            title={t("imageLightbox.zoomIn")}
          >
            <ZoomIn size={18} />
          </button>
          <button
            onClick={handleRotate}
            className="p-2 rounded hover:bg-white/10 transition-colors"
            title={t("imageLightbox.rotate")}
          >
            <RotateCw size={18} />
          </button>
          <button
            onClick={toggleFitToScreen}
            className="p-2 rounded hover:bg-white/10 transition-colors"
            title={isFitToScreen ? t("imageLightbox.actualSize") : t("imageLightbox.fitToScreen")}
          >
            {isFitToScreen ? <Maximize2 size={18} /> : <Minimize2 size={18} />}
          </button>
          <button
            onClick={onClose}
            className="p-2 rounded hover:bg-white/10 transition-colors ml-2"
            title={t("imageLightbox.close")}
          >
            <X size={18} />
          </button>
        </div>
      </div>

      {/* Image Container */}
      <div
        ref={containerRef}
        className="flex-1 overflow-hidden cursor-grab active:cursor-grabbing flex items-center justify-center"
        onMouseDown={handleMouseDown}
      >
        <img
          src={useFallback && fallbackSrc ? fallbackSrc : imageSrc}
          onError={() => {
            if (fallbackSrc && !useFallback) setUseFallback(true);
          }}
          alt={imageName}
          className="max-w-none select-none"
          style={{
            transform: `translate(${position.x}px, ${position.y}px) scale(${scale}) rotate(${rotation}deg)`,
            transition: isDragging ? "none" : "transform 0.1s ease-out",
            maxWidth: isFitToScreen ? "90vw" : "none",
            maxHeight: isFitToScreen ? "85vh" : "none",
            objectFit: isFitToScreen ? "contain" : "none",
          }}
          draggable={false}
        />
      </div>

      {/* Footer hint */}
      <div className="text-center py-2 text-white/50 text-xs">
        {t("imageLightbox.hint")}
      </div>
    </div>
  );
}
