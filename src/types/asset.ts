export type AssetType = "texture" | "model" | "audio" | "other";

export interface AssetMetadata {
  width?: number;
  height?: number;
}

export interface AssetInfo {
  path: string;
  name: string;
  extension: string;
  asset_type: AssetType;
  size: number;
  metadata?: AssetMetadata;
}

export interface DirectoryNode {
  name: string;
  path: string;
  children: DirectoryNode[];
  file_count: number;
  total_size: number;
}

export interface ScanResult {
  root_path: string;
  directory_tree: DirectoryNode;
  assets: AssetInfo[];
  total_count: number;
  total_size: number;
  type_counts: Record<string, number>;
}
