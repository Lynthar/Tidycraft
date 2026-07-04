import { useRef, useState, useEffect } from "react";
import { Play, Pause, Volume2, VolumeX, Maximize2 } from "lucide-react";
import { convertFileSrc } from "@tauri-apps/api/core";
import { useTranslation } from "react-i18next";

interface VideoPlayerProps {
  filePath: string;
}

export function VideoPlayer({ filePath }: VideoPlayerProps) {
  const { t } = useTranslation();
  const videoRef = useRef<HTMLVideoElement>(null);
  const [isPlaying, setIsPlaying] = useState(false);
  const [isMuted, setIsMuted] = useState(false);
  const [progress, setProgress] = useState(0);
  const [duration, setDuration] = useState(0);
  const [currentTime, setCurrentTime] = useState(0);
  const [loadError, setLoadError] = useState(false);

  const videoSrc = convertFileSrc(filePath);

  useEffect(() => {
    // Reset state when file changes
    setIsPlaying(false);
    setProgress(0);
    setCurrentTime(0);
    setLoadError(false);
    if (videoRef.current) {
      videoRef.current.currentTime = 0;
    }
  }, [filePath]);

  const togglePlay = () => {
    if (videoRef.current) {
      if (isPlaying) {
        videoRef.current.pause();
        setIsPlaying(false);
      } else {
        // play() returns a promise that REJECTS for unsupported codecs /
        // decode failures / autoplay policy. Only flip the state on
        // success — the old unconditional flip left a "playing" UI over a
        // frozen frame, plus an unhandled-rejection console error.
        videoRef.current
          .play()
          .then(() => setIsPlaying(true))
          .catch((err) => {
            console.warn("[VideoPlayer] play() failed:", err);
            setIsPlaying(false);
          });
      }
    }
  };

  const toggleMute = () => {
    if (videoRef.current) {
      videoRef.current.muted = !isMuted;
      setIsMuted(!isMuted);
    }
  };

  const handleTimeUpdate = () => {
    if (videoRef.current) {
      const current = videoRef.current.currentTime;
      const total = videoRef.current.duration;
      setCurrentTime(current);
      // duration is NaN until metadata loads (and Infinity for streams) —
      // don't let the progress width go NaN%.
      setProgress(Number.isFinite(total) && total > 0 ? (current / total) * 100 : 0);
    }
  };

  const handleLoadedMetadata = () => {
    if (videoRef.current) {
      setDuration(videoRef.current.duration);
    }
  };

  const handleSeek = (e: React.MouseEvent<HTMLDivElement>) => {
    if (videoRef.current) {
      const total = videoRef.current.duration;
      // Before metadata loads duration is NaN — seeking to NaN throws.
      if (!Number.isFinite(total) || total <= 0) return;
      const rect = e.currentTarget.getBoundingClientRect();
      const percent = (e.clientX - rect.left) / rect.width;
      videoRef.current.currentTime = percent * total;
    }
  };

  const handleFullscreen = () => {
    if (videoRef.current) {
      if (document.fullscreenElement) {
        document.exitFullscreen();
      } else {
        videoRef.current.requestFullscreen();
      }
    }
  };

  const formatTime = (seconds: number): string => {
    if (!Number.isFinite(seconds)) return "0:00";
    const mins = Math.floor(seconds / 60);
    const secs = Math.floor(seconds % 60);
    return `${mins}:${secs.toString().padStart(2, "0")}`;
  };

  const handleEnded = () => {
    setIsPlaying(false);
  };

  return (
    <div className="w-full bg-background rounded overflow-hidden">
      {/* Video Element */}
      <div className="relative aspect-video bg-black">
        <video
          ref={videoRef}
          src={videoSrc}
          className="w-full h-full object-contain"
          onTimeUpdate={handleTimeUpdate}
          onLoadedMetadata={handleLoadedMetadata}
          onEnded={handleEnded}
          onClick={togglePlay}
          onError={() => setLoadError(true)}
        />
        {loadError && (
          <div className="absolute inset-0 flex items-center justify-center bg-black/60">
            <span className="text-xs px-4 text-center" style={{ color: "var(--err)" }}>
              {t("mediaPlayer.loadError")}
            </span>
          </div>
        )}
        {/* Play overlay when paused */}
        {!isPlaying && (
          <div
            className="absolute inset-0 flex items-center justify-center bg-black/30 cursor-pointer"
            onClick={togglePlay}
          >
            <Play size={48} className="text-white/80" />
          </div>
        )}
      </div>

      {/* Controls */}
      <div className="p-2 space-y-2">
        {/* Progress bar */}
        <div
          className="h-1.5 bg-border rounded-full cursor-pointer group"
          onClick={handleSeek}
        >
          <div
            className="h-full bg-primary rounded-full transition-all group-hover:h-2 relative"
            style={{ width: `${progress}%` }}
          >
            <div className="absolute right-0 top-1/2 -translate-y-1/2 w-3 h-3 bg-primary rounded-full opacity-0 group-hover:opacity-100 transition-opacity" />
          </div>
        </div>

        {/* Control buttons */}
        <div className="flex items-center justify-between text-xs">
          <div className="flex items-center gap-2">
            <button
              onClick={togglePlay}
              className="p-1 rounded hover:bg-card-bg text-text-secondary hover:text-text-primary transition-colors"
            >
              {isPlaying ? <Pause size={16} /> : <Play size={16} />}
            </button>
            <button
              onClick={toggleMute}
              className="p-1 rounded hover:bg-card-bg text-text-secondary hover:text-text-primary transition-colors"
            >
              {isMuted ? <VolumeX size={16} /> : <Volume2 size={16} />}
            </button>
            <span className="text-text-secondary">
              {formatTime(currentTime)} / {formatTime(duration)}
            </span>
          </div>
          <button
            onClick={handleFullscreen}
            className="p-1 rounded hover:bg-card-bg text-text-secondary hover:text-text-primary transition-colors"
          >
            <Maximize2 size={16} />
          </button>
        </div>
      </div>
    </div>
  );
}
