export type AssetType =
  | "texture"
  | "model"
  | "audio"
  | "animation"
  | "material"
  | "prefab"
  | "scene"
  | "script"
  | "data"
  | "other";

export type ProjectType = "unity" | "unreal" | "godot" | "generic";

export interface AssetMetadata {
  // Image metadata
  width?: number;
  height?: number;
  has_alpha?: boolean;
  // Model metadata
  vertex_count?: number;
  face_count?: number;
  material_count?: number;
  // Audio metadata
  duration_secs?: number;
  sample_rate?: number;
  channels?: number;
  bit_depth?: number;
}

export interface AssetInfo {
  path: string;
  name: string;
  extension: string;
  asset_type: AssetType;
  size: number;
  metadata?: AssetMetadata;
  unity_guid?: string;
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
  project_type?: ProjectType;
}

export type ScanPhase =
  | "discovering"
  | "parsing"
  | "building"
  | "completed"
  | "cancelled";

export interface ScanProgress {
  phase: ScanPhase;
  current: number;
  total?: number;
  current_file: string;
}

// Analysis types
export type Severity = "error" | "warning" | "info";

export interface Issue {
  rule_id: string;
  rule_name: string;
  severity: Severity;
  message: string;
  asset_path: string;
  suggestion?: string;
  auto_fixable: boolean;
}

export interface AnalysisResult {
  issues: Issue[];
  issue_count: number;
  error_count: number;
  warning_count: number;
  info_count: number;
  by_rule: Record<string, number>;
}
