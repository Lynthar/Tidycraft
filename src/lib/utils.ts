import { clsx, type ClassValue } from "clsx";
import { twMerge } from "tailwind-merge";

export function cn(...inputs: ClassValue[]) {
  return twMerge(clsx(inputs));
}

export function formatFileSize(bytes: number): string {
  if (bytes === 0) return "0 B";

  const units = ["B", "KB", "MB", "GB", "TB"];
  const k = 1024;
  const i = Math.floor(Math.log(bytes) / Math.log(k));

  return `${(bytes / Math.pow(k, i)).toFixed(i > 0 ? 1 : 0)} ${units[i]}`;
}

export function getAssetTypeColor(type: string): string {
  const colors: Record<string, string> = {
    texture: "text-green-400",
    model: "text-blue-400",
    audio: "text-yellow-400",
    animation: "text-purple-400",
    material: "text-pink-400",
    prefab: "text-cyan-400",
    scene: "text-orange-400",
    script: "text-red-400",
    data: "text-gray-400",
    other: "text-gray-400",
  };
  return colors[type] || "text-gray-400";
}

export function formatDuration(seconds: number): string {
  const mins = Math.floor(seconds / 60);
  const secs = Math.floor(seconds % 60);
  return `${mins}:${secs.toString().padStart(2, "0")}`;
}
