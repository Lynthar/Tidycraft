import { useRef, useState, useEffect, useCallback } from "react";
import { Play, Pause, Volume2, VolumeX } from "lucide-react";
import { convertFileSrc } from "@tauri-apps/api/core";

interface AudioPlayerProps {
  filePath: string;
}

export function AudioPlayer({ filePath }: AudioPlayerProps) {
  const audioRef = useRef<HTMLAudioElement>(null);
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const animationRef = useRef<number>(0);
  const audioContextRef = useRef<AudioContext | null>(null);
  const analyzerRef = useRef<AnalyserNode | null>(null);
  const sourceRef = useRef<MediaElementAudioSourceNode | null>(null);

  const [isPlaying, setIsPlaying] = useState(false);
  const [isMuted, setIsMuted] = useState(false);
  const [progress, setProgress] = useState(0);
  const [duration, setDuration] = useState(0);
  const [currentTime, setCurrentTime] = useState(0);
  const [volume, setVolume] = useState(1);

  const audioSrc = convertFileSrc(filePath);

  // Setup audio context and analyzer
  const setupAudioContext = useCallback(() => {
    if (!audioRef.current || audioContextRef.current) return;

    const audioContext = new (window.AudioContext || (window as unknown as { webkitAudioContext: typeof AudioContext }).webkitAudioContext)();
    const analyzer = audioContext.createAnalyser();
    analyzer.fftSize = 256;

    const source = audioContext.createMediaElementSource(audioRef.current);
    source.connect(analyzer);
    analyzer.connect(audioContext.destination);

    audioContextRef.current = audioContext;
    analyzerRef.current = analyzer;
    sourceRef.current = source;
  }, []);

  // Draw waveform visualization
  const drawWaveform = useCallback(() => {
    if (!canvasRef.current || !analyzerRef.current) return;

    const canvas = canvasRef.current;
    const ctx = canvas.getContext("2d");
    if (!ctx) return;

    const analyzer = analyzerRef.current;
    const bufferLength = analyzer.frequencyBinCount;
    const dataArray = new Uint8Array(bufferLength);

    const draw = () => {
      animationRef.current = requestAnimationFrame(draw);

      analyzer.getByteFrequencyData(dataArray);

      ctx.fillStyle = "rgb(30, 30, 30)";
      ctx.fillRect(0, 0, canvas.width, canvas.height);

      const barWidth = (canvas.width / bufferLength) * 2.5;
      let x = 0;

      for (let i = 0; i < bufferLength; i++) {
        const barHeight = (dataArray[i] / 255) * canvas.height;

        // Gradient color based on height
        const hue = (i / bufferLength) * 60 + 200; // Blue to purple range
        ctx.fillStyle = `hsl(${hue}, 70%, 50%)`;

        ctx.fillRect(x, canvas.height - barHeight, barWidth, barHeight);
        x += barWidth + 1;
      }
    };

    draw();
  }, []);

  // Cleanup on unmount or file change
  useEffect(() => {
    return () => {
      if (animationRef.current) {
        cancelAnimationFrame(animationRef.current);
      }
      if (audioContextRef.current && audioContextRef.current.state !== "closed") {
        audioContextRef.current.close();
      }
      audioContextRef.current = null;
      analyzerRef.current = null;
      sourceRef.current = null;
    };
  }, [filePath]);

  // Reset state when file changes
  useEffect(() => {
    setIsPlaying(false);
    setProgress(0);
    setCurrentTime(0);
    if (animationRef.current) {
      cancelAnimationFrame(animationRef.current);
    }
  }, [filePath]);

  const togglePlay = async () => {
    if (audioRef.current) {
      if (isPlaying) {
        audioRef.current.pause();
        if (animationRef.current) {
          cancelAnimationFrame(animationRef.current);
        }
      } else {
        // Setup audio context on first play (requires user interaction)
        if (!audioContextRef.current) {
          setupAudioContext();
        }
        if (audioContextRef.current?.state === "suspended") {
          await audioContextRef.current.resume();
        }
        await audioRef.current.play();
        drawWaveform();
      }
      setIsPlaying(!isPlaying);
    }
  };

  const toggleMute = () => {
    if (audioRef.current) {
      audioRef.current.muted = !isMuted;
      setIsMuted(!isMuted);
    }
  };

  const handleTimeUpdate = () => {
    if (audioRef.current) {
      const current = audioRef.current.currentTime;
      const total = audioRef.current.duration;
      setCurrentTime(current);
      setProgress((current / total) * 100);
    }
  };

  const handleLoadedMetadata = () => {
    if (audioRef.current) {
      setDuration(audioRef.current.duration);
    }
  };

  const handleSeek = (e: React.MouseEvent<HTMLDivElement>) => {
    if (audioRef.current) {
      const rect = e.currentTarget.getBoundingClientRect();
      const percent = (e.clientX - rect.left) / rect.width;
      const time = percent * audioRef.current.duration;
      audioRef.current.currentTime = time;
    }
  };

  const handleVolumeChange = (e: React.ChangeEvent<HTMLInputElement>) => {
    const newVolume = parseFloat(e.target.value);
    setVolume(newVolume);
    if (audioRef.current) {
      audioRef.current.volume = newVolume;
    }
  };

  const formatTime = (seconds: number): string => {
    if (!isFinite(seconds)) return "0:00";
    const mins = Math.floor(seconds / 60);
    const secs = Math.floor(seconds % 60);
    return `${mins}:${secs.toString().padStart(2, "0")}`;
  };

  const handleEnded = () => {
    setIsPlaying(false);
    if (animationRef.current) {
      cancelAnimationFrame(animationRef.current);
    }
  };

  return (
    <div className="w-full bg-background rounded overflow-hidden">
      {/* Audio Element (hidden) */}
      <audio
        ref={audioRef}
        src={audioSrc}
        onTimeUpdate={handleTimeUpdate}
        onLoadedMetadata={handleLoadedMetadata}
        onEnded={handleEnded}
        crossOrigin="anonymous"
      />

      {/* Waveform Visualization */}
      <div className="relative aspect-[2/1] bg-[rgb(30,30,30)] rounded-t">
        <canvas
          ref={canvasRef}
          className="w-full h-full"
          width={300}
          height={150}
        />
        {!isPlaying && (
          <div className="absolute inset-0 flex items-center justify-center">
            <div
              className="p-3 bg-primary/20 rounded-full cursor-pointer hover:bg-primary/30 transition-colors"
              onClick={togglePlay}
            >
              <Play size={32} className="text-primary" />
            </div>
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
            <input
              type="range"
              min="0"
              max="1"
              step="0.1"
              value={volume}
              onChange={handleVolumeChange}
              className="w-16 h-1 accent-primary"
            />
          </div>
          <span className="text-text-secondary">
            {formatTime(currentTime)} / {formatTime(duration)}
          </span>
        </div>
      </div>
    </div>
  );
}
