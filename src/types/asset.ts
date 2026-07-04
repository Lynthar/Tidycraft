export type AssetType =
  | "texture"
  | "model"
  | "audio"
  | "video"
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
  // Audio / video metadata (duration is shared)
  duration_secs?: number;
  sample_rate?: number;
  channels?: number;
  bit_depth?: number;
  // Video-specific
  framerate?: number;
  video_codec?: string;
  // Texture extras
  color_space?: string;
  mipmap_count?: number;
  /** When set, identifies this file as an authoring/source file from
   *  a DCC tool ("blender" / "maya_ascii" / "maya_binary" / "max" /
   *  "zbrush" / "substance_painter" / "substance_designer" / "marvelous"
   *  / "photoshop" / "modo" / "houdini" / "cinema4d"). The dcc_source
   *  analyzer pairs sources with their exports and warns when the
   *  source's mtime is newer. Mirror of Rust `AssetMetadata.dcc_source_kind`. */
  dcc_source_kind?: string;
}

export interface AssetInfo {
  path: string;
  name: string;
  extension: string;
  asset_type: AssetType;
  size: number;
  /** File mtime as unix seconds (0 when unreadable). Change signal for
   *  mounted thumbnail views: CardThumb / AssetPreview key their fetch
   *  effects on it so an externally edited file refreshes in place. */
  modified: number;
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

// ============ Unreal Engine Types ============

export interface UnrealProjectInfo {
  path: string;
  project_name: string;
  engine_association?: string;
  category?: string;
  description?: string;
  plugins: UnrealPlugin[];
  target_platforms: string[];
  modules: UnrealModule[];
  is_enterprise_project: boolean;
}

export interface UnrealPlugin {
  name: string;
  enabled: boolean;
}

export interface UnrealModule {
  name: string;
  module_type: string;
  loading_phase?: string;
}

// ============ Godot Types ============

export interface GodotProjectInfo {
  path: string;
  project_name: string;
  godot_version?: string;
  main_scene?: string;
  icon?: string;
  features: string[];
  autoloads: GodotAutoload[];
  input_actions: string[];
  renderer?: string;
}

export interface GodotAutoload {
  name: string;
  path: string;
  singleton: boolean;
}

// ============ Dependency Graph Types ============

/** Mirrors Rust `DependencyNode` (src-tauri/src/lib.rs). `id` is the
 *  engine-neutral graph key (Unity GUID / Godot res:// path); `path` is the
 *  absolute filesystem path used to locate the asset. */
export interface DependencyNode {
  id: string;
  path: string;
  name: string;
  file_type: string;
}

export interface DependencyEdge {
  from: string;
  to: string;
}

export interface DependencyGraph {
  nodes: DependencyNode[];
  edges: DependencyEdge[];
}

// ============ Undo Types ============

export type OperationType = "rename" | "move" | "delete";

export interface UndoResult {
  success: boolean;
  reverted_count: number;
  failed_count: number;
  errors: string[];
  operation_description: string;
}

export interface HistoryEntry {
  id: string;
  description: string;
  file_count: number;
  timestamp: number;
  can_undo: boolean;
}

// ============ Git Types ============

export type GitFileStatus =
  | "new"
  | "modified"
  | "deleted"
  | "renamed"
  | "typechange"
  | "untracked"
  | "ignored"
  | "conflicted"
  | "unchanged";

export interface GitInfo {
  is_repo: boolean;
  branch?: string;
  has_changes: boolean;
  ahead: number;
  behind: number;
}

export type GitStatusMap = Record<string, GitFileStatus>;

// ============ Tag Types ============

export interface Tag {
  id: string;
  name: string;
  color: string;
  /** Optional user-written semantic context. Fed to the LLM as part of
   *  the project context bundle when AI tagging is invoked. Empty string
   *  is normalized to undefined backend-side. */
  description?: string;
}

export type AssetTagsMap = Record<string, Tag[]>;

// ============ Delete Types ============

export interface DeleteError {
  path: string;
  message: string;
}

export interface DeleteResult {
  success_paths: string[];
  errors: DeleteError[];
}

// ============ Move / Copy / Duplicate ============

export interface FileOpError {
  path: string;
  message: string;
}

export interface FileOpSuccess {
  original_path: string;
  new_path: string;
}

export interface FileOpResult {
  successes: FileOpSuccess[];
  errors: FileOpError[];
}

// ============ Filesystem Watcher Types ============

/// Payload of the `fs-change-{projectId}` Tauri event.
export interface FsChangeEvent {
  /** Assets that were added or modified. Merge into scanResult.assets by `path`. */
  updated: AssetInfo[];
  /** Paths that were deleted. Remove from scanResult.assets. */
  removed: string[];
  /** Freshly rebuilt directory tree — swap wholesale. */
  directory_tree: DirectoryNode;
  total_count: number;
  total_size: number;
  type_counts: Record<string, number>;
}
