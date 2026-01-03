import { useState, useEffect, useRef, useCallback } from "react";
import { X, ZoomIn, ZoomOut, RotateCw, Maximize2, Minimize2 } from "lucide-react";

interface ImageLightboxProps {
  isOpen: boolean;
  imageSrc: string;
  imageName: string;
  onClose: () => void;
}

export function ImageLightbox({ isOpen, imageSrc, imageName, onClose }: ImageLightboxProps) {
  const [scale, setScale] = useState(1);
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
    }
  }, [isOpen, imageSrc]);

  // Handle keyboard events
  useEffect(() => {
    if (!isOpen) return;

    const handleKeyDown = (e: KeyboardEvent) => {
      switch (e.key) {
        case "Escape":
          onClose();
          break;
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

  // Handle mouse wheel zoom
  const handleWheel = useCallback((e: React.WheelEvent) => {
    e.preventDefault();
    const delta = e.deltaY > 0 ? -0.1 : 0.1;
    setScale((prev) => Math.min(Math.max(0.1, prev + delta), 5));
    setIsFitToScreen(false);
  }, []);

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
            title="Zoom out (-)"
          >
            <ZoomOut size={18} />
          </button>
          <span className="text-sm w-16 text-center">{Math.round(scale * 100)}%</span>
          <button
            onClick={handleZoomIn}
            className="p-2 rounded hover:bg-white/10 transition-colors"
            title="Zoom in (+)"
          >
            <ZoomIn size={18} />
          </button>
          <button
            onClick={handleRotate}
            className="p-2 rounded hover:bg-white/10 transition-colors"
            title="Rotate (R)"
          >
            <RotateCw size={18} />
          </button>
          <button
            onClick={toggleFitToScreen}
            className="p-2 rounded hover:bg-white/10 transition-colors"
            title={isFitToScreen ? "Actual size" : "Fit to screen (0)"}
          >
            {isFitToScreen ? <Maximize2 size={18} /> : <Minimize2 size={18} />}
          </button>
          <button
            onClick={onClose}
            className="p-2 rounded hover:bg-white/10 transition-colors ml-2"
            title="Close (Esc)"
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
        onWheel={handleWheel}
      >
        <img
          src={imageSrc}
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
        Scroll to zoom • Drag to pan • R to rotate • Esc to close
      </div>
    </div>
  );
}
