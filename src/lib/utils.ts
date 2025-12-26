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

export function getAssetTypeLabel(type: string): string {
  const labels: Record<string, string> = {
    texture: "Texture",
    model: "Model",
    audio: "Audio",
    other: "Other",
  };
  return labels[type] || type;
}

export function getAssetTypeColor(type: string): string {
  const colors: Record<string, string> = {
    texture: "text-green-400",
    model: "text-blue-400",
    audio: "text-yellow-400",
    other: "text-gray-400",
  };
  return colors[type] || "text-gray-400";
}
