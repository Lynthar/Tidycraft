import { beforeEach, describe, expect, it, vi } from "vitest";

// The store's imports read localStorage and navigator at module init (i18n language,
// settings); Node 20 has neither, so stand both in before those imports run.
vi.hoisted(() => {
  const data = new Map<string, string>();
  Object.defineProperty(globalThis, "localStorage", {
    value: {
      getItem: (key: string) => data.get(key) ?? null,
      setItem: (key: string, value: string) => void data.set(key, value),
      removeItem: (key: string) => void data.delete(key),
    },
  });
  if (typeof navigator === "undefined") {
    Object.defineProperty(globalThis, "navigator", {
      value: { language: "en-US", languages: ["en-US"], userAgent: "node" },
    });
  }
});

// tagsStore, selectionStore and recentsStore are deliberately NOT imported here:
// their bridges stay unregistered, which is the state the bridge design has to
// survive.
import { invoke } from "@tauri-apps/api/core";
import {
  MIRROR_FIELDS,
  mirrorOf,
  renamedTargetFor,
  useProjectStore,
  type ProjectData,
} from "./projectStore";
import type { AssetInfo, AssetMetadata, AssetType, RenamedPair, ScanResult } from "../types/asset";

// The one backend boundary. A switch asks two things: whether the folder is
// still there (`check_project_paths` — answering "missing" stops the flow
// before the register / scan / listen chain) and the git state. Teardown
// commands are best-effort. Anything else is a test reaching further than
// it meant to.
vi.mock("@tauri-apps/api/core", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@tauri-apps/api/core")>()),
  invoke: vi.fn(),
}));
const backend = vi.mocked(invoke);
const commands = () => backend.mock.calls.map(([command]) => command);
const probes = () => commands().filter((c) => c === "check_project_paths").length;
beforeEach(() => {
  backend.mockReset();
  backend.mockImplementation(async (command, args) => {
    if (command === "check_project_paths") {
      const { paths } = args as { paths: string[] };
      return paths.map((path) => ({ path, status: { kind: "missing" } }));
    }
    if (command === "get_git_info") return { is_repo: false };
    if (command === "stop_watching" || command === "unregister_project") return undefined;
    throw new Error(`unexpected backend command ${command}`);
  });
});

function project(id: string): ProjectData {
  return { ...mirrorOf(undefined), id, projectPath: `/projects/${id}` };
}

/// Every mirror field on the flat store is the active project's own value.
function expectMirrorOfActive() {
  const state = useProjectStore.getState();
  const active = state.projects.get(state.activeProjectId!)!;
  for (const field of MIRROR_FIELDS) expect(state[field], field).toBe(active[field]);
}

describe("the active project's mirror", () => {
  beforeEach(() => {
    const a = project("a");
    const b = project("b");
    useProjectStore.setState({
      projects: new Map([
        [a.id, a],
        [b.id, b],
      ]),
      activeProjectId: a.id,
      ...mirrorOf(a),
    });
  });

  it("starts as the defaults with no path", () => {
    const initial = useProjectStore.getInitialState();
    const empty = mirrorOf(undefined);
    expect(initial.projectPath).toBeNull();
    for (const field of MIRROR_FIELDS) expect(initial[field], field).toEqual(empty[field]);
    // Arrays are per project: two empty mirrors must not share one.
    expect(mirrorOf(undefined).advancedFilters).not.toBe(empty.advancedFilters);
  });

  it("carries exactly the mirror fields of a project", () => {
    const a = project("a");
    const view = mirrorOf(a);
    expect(Object.keys(view).sort()).toEqual([...MIRROR_FIELDS].sort());
    for (const field of MIRROR_FIELDS) expect(view[field], field).toBe(a[field]);
  });

  it("follows every setter and leaves the other project alone", () => {
    const store = useProjectStore.getState();
    const untouched = store.projects.get("b");
    store.setViewMode("issues");
    store.setSearchQuery("rock");
    store.setTypeFilter(["texture", "model"]);
    store.setSortField("size");
    store.toggleSortDirection();
    store.setAdvancedFilters({ minSize: 1024 });
    store.setSelectedDirectory("/projects/a/Textures");
    store.setHasCustomConfig(true);
    expectMirrorOfActive();
    const after = useProjectStore.getState();
    expect(after.viewMode).toBe("issues");
    expect(after.sortDirection).toBe("desc");
    expect(after.advancedFilters.minSize).toBe(1024);
    expect(after.projects.get("b")).toBe(untouched);
  });
});

// ---- fixtures ---------------------------------------------------------------

const ROOT = "/p";

function asset(path: string, asset_type: AssetType, size: number, metadata?: AssetMetadata): AssetInfo {
  const name = path.slice(path.lastIndexOf("/") + 1);
  return { path, name, extension: name.slice(name.lastIndexOf(".") + 1), asset_type, size, modified: 0, metadata };
}

/// Six assets, three directories deep, every metadata family present once.
function scan(): ScanResult {
  const assets = [
    asset(`${ROOT}/rock.fbx`, "model", 3000, { vertex_count: 1200, face_count: 600 }),
    asset(`${ROOT}/Textures/wood.png`, "texture", 2048, { width: 1024, height: 1024, has_alpha: false }),
    asset(`${ROOT}/Textures/UI/icon.png`, "texture", 512, { width: 64, height: 64, has_alpha: true }),
    asset(`${ROOT}/Audio/hit.wav`, "audio", 4096, { duration_secs: 1.5 }),
    asset(`${ROOT}/Audio/theme.ogg`, "audio", 8192, { duration_secs: 95 }),
    asset(`${ROOT}/Scripts/Player.cs`, "script", 700),
  ];
  return {
    root_path: ROOT,
    directory_tree: { name: "p", path: ROOT, children: [], file_count: assets.length, total_size: 0 },
    assets,
    total_count: assets.length,
    total_size: assets.reduce((sum, a) => sum + a.size, 0),
    type_counts: {},
    warnings: [],
  };
}

function scanned(id: string, overrides: Partial<ProjectData> = {}): ProjectData {
  return { ...project(id), projectPath: ROOT, scanResult: scan(), ...overrides };
}

function activate(...projects: ProjectData[]) {
  const [active] = projects;
  useProjectStore.setState({
    projects: new Map(projects.map((p) => [p.id, p])),
    activeProjectId: active.id,
    ...mirrorOf(active),
  });
}

const store = () => useProjectStore.getState();
const names = () => store().getFilteredAssets().map((a) => a.name);

// ---- renamedTargetFor -------------------------------------------------------

describe("renamedTargetFor", () => {
  const renamed: RenamedPair[] = [
    { from: "/p/Tex", to: "/p/Tex2", is_dir: true },
    { from: "/p/Textures", to: "/p/Images", is_dir: true },
    { from: "/p/rock.fbx", to: "/p/SM_rock.fbx", is_dir: false },
  ];

  it("rewrites an exact file, a directory and its contents, and nothing else", () => {
    expect(renamedTargetFor("/p/rock.fbx", renamed)).toBe("/p/SM_rock.fbx");
    expect(renamedTargetFor("/p/Textures", renamed)).toBe("/p/Images");
    expect(renamedTargetFor("/p/Textures/UI/icon.png", renamed)).toBe("/p/Images/UI/icon.png");
    expect(renamedTargetFor("/p/Audio/hit.wav", renamed)).toBeNull();
  });

  it("matches directories on component boundaries: `Tex` never captures `Textures/…`", () => {
    const tex = renamed.slice(0, 1);
    expect(renamedTargetFor("/p/Tex/a.png", tex)).toBe("/p/Tex2/a.png");
    expect(renamedTargetFor("/p/Textures/wood.png", tex)).toBeNull();
  });
});

// ---- selected directory and locate -----------------------------------------

describe("the selected directory", () => {
  beforeEach(() => activate(scanned("a")));

  it("stores the project root as null: filtering by the root is no filter", () => {
    store().setSelectedDirectory(`${ROOT}/Textures`);
    expect(store().selectedDirectory).toBe(`${ROOT}/Textures`);
    store().setSelectedDirectory(ROOT);
    expect(store().selectedDirectory).toBeNull();
    expect(names()).toHaveLength(6);
  });

  it("locate lands on the asset and clears whatever hid it, with no tag store registered", () => {
    store().setSearchQuery("nothing-matches");
    store().setSelectedDirectory(`${ROOT}/Audio`);
    store().setViewMode("issues");
    const pulse = store().locatePulse;

    store().locateAsset(`${ROOT}/rock.fbx`);

    expect(store().selectedAsset?.path).toBe(`${ROOT}/rock.fbx`);
    expect(store().selectedDirectory).toBeNull(); // its directory is the root
    expect(store().searchQuery).toBe("");
    expect(store().viewMode).toBe("assets");
    expect(store().locatePulse).toBe(pulse + 1);
    expect(names()).toContain("rock.fbx");
  });
});

// ---- getFilteredAssets ------------------------------------------------------

describe("getFilteredAssets", () => {
  beforeEach(() => activate(scanned("a")));

  it("keeps the assets under the selected directory, nested ones included", () => {
    store().setSelectedDirectory(`${ROOT}/Textures`);
    expect(names()).toEqual(["icon.png", "wood.png"]);
  });

  it("trims the search query, so a pasted trailing space cannot join the needle", () => {
    store().setSearchQuery("wood ");
    expect(names()).toEqual(["wood.png"]);
  });

  it("a duration ceiling matches nothing that has no duration", () => {
    store().setAdvancedFilters({ maxDuration: 10 });
    expect(names()).toEqual(["hit.wav"]);
  });

  it("a git status filter treats a file the backend did not mention as unchanged", () => {
    activate(scanned("a", { gitStatuses: { [`${ROOT}/rock.fbx`]: "modified" } }));
    store().setAdvancedFilters({ gitStatusFilter: ["modified", "new"] });
    expect(names()).toEqual(["rock.fbx"]);
  });

  it("sorts by the chosen field in either direction", () => {
    store().setSortField("size");
    expect(names()).toEqual(["icon.png", "Player.cs", "wood.png", "rock.fbx", "hit.wav", "theme.ogg"]);
    store().toggleSortDirection();
    expect(names()).toEqual(["theme.ogg", "hit.wav", "rock.fbx", "wood.png", "Player.cs", "icon.png"]);
  });

  it("hands back the same list until an input changes", () => {
    const first = store().getFilteredAssets();
    expect(store().getFilteredAssets()).toBe(first);
    store().setSearchQuery("rock");
    expect(store().getFilteredAssets()).not.toBe(first);
  });
});

// ---- switching, hydration, relocation, removal -----------------------------

describe("switching projects", () => {
  it("is what isStillActive answers, and it flips the moment the user switches", () => {
    activate(scanned("a"), scanned("b"));
    expect(store().isStillActive("a")).toBe(true);
    expect(store().isStillActive("b")).toBe(false);
    store().setActiveProject("b");
    expect(store().isStillActive("a")).toBe(false);
    expect(store().isStillActive("b")).toBe(true);
  });

  it("probes a never-scanned project once, then treats the failure as final", async () => {
    activate(scanned("a"), project("stub"));

    store().setActiveProject("stub");
    await vi.waitFor(() => expect(store().activeProjectId).toBe("stub"));
    expect(probes()).toBe(1);
    expect(backend.mock.calls[0][1]).toEqual({ paths: ["/projects/stub"] });
    expect(store().unavailable).toEqual({ kind: "missing" });
    expectMirrorOfActive();

    store().setActiveProject("a");
    store().setActiveProject("stub");
    expect(store().activeProjectId).toBe("stub");
    expect(probes()).toBe(1);
  });

  it("switches to a failed or busy project directly, without a probe", () => {
    activate(scanned("a"), { ...project("failed"), error: "boom" }, { ...project("busy"), isScanning: true });
    store().setActiveProject("failed");
    expect(store().activeProjectId).toBe("failed");
    store().setActiveProject("busy");
    expect(store().activeProjectId).toBe("busy");
    expect(probes()).toBe(0);
  });

  it("relocating keeps the six viewing preferences and drops what the old root defined", async () => {
    const a = scanned("a", {
      viewMode: "issues",
      searchQuery: "wood",
      typeFilter: ["texture"],
      sortField: "size",
      sortDirection: "desc",
      advancedFilters: { ...mirrorOf(undefined).advancedFilters, minSize: 1 },
      selectedDirectory: `${ROOT}/Textures`,
      gitStatuses: { [`${ROOT}/rock.fbx`]: "modified" },
      hasCustomConfig: true,
    });
    activate(a);

    await store().relocateProject("a", "/moved");

    const after = store().projects.get("a")!;
    expect(after.projectPath).toBe("/moved");
    for (const field of ["viewMode", "searchQuery", "typeFilter", "sortField", "sortDirection", "advancedFilters"] as const) {
      expect(after[field], field).toEqual(a[field]);
    }
    expect(after.scanResult).toBeNull();
    expect(after.selectedDirectory).toBeNull();
    expect(after.gitStatuses).toEqual({});
    expect(after.hasCustomConfig).toBe(false);
    expect(after.unavailable).toEqual({ kind: "missing" });
    expect(store().activeProjectId).toBe("a");
    expectMirrorOfActive();
    expect(commands()).toEqual(["stop_watching", "check_project_paths"]);
  });

  it("removes a project with no recents store registered", () => {
    activate(scanned("a"), scanned("b"));
    store().removeProject("b");
    expect(store().projects.has("b")).toBe(false);
    expect(store().activeProjectId).toBe("a");
    expect(commands()).toEqual(["stop_watching", "unregister_project"]);
  });
});
