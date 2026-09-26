import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

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
  createDefaultAdvancedFilters,
  mirrorOf,
  registerRecentsBridge,
  registerTagsSyncBridge,
  renamedTargetFor,
  useProjectStore,
  type AdvancedFilters,
  type ProjectData,
  type SortField,
} from "./projectStore";
import { useToastStore } from "./toastStore";
import type { AssetInfo, AssetMetadata, AssetType, HistoryEntry, RenamedPair, ScanResult } from "../types/asset";

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
    expect(initial.projectPath).toBeNull();
    expect(initial.activeProjectId).toBeNull();
    expect(initial.projects.size).toBe(0);
    // Idle, unfiltered, unflagged and nothing undoable — spelled out, not compared
    // with the function that produced it.
    expect(initial).toMatchObject({
      scanResult: null,
      isScanning: false,
      error: null,
      scanProgress: null,
      analysisResult: null,
      analysisStale: false,
      isAnalyzing: false,
      viewMode: "assets",
      selectedDirectory: null,
      selectedAsset: null,
      searchQuery: "",
      typeFilter: null,
      sortField: "name",
      sortDirection: "asc",
      gitInfo: null,
      gitStatuses: {},
      hasCustomConfig: false,
      projectWarnings: [],
      unavailable: null,
      canUndo: false,
      undoHistory: [],
    });
    expect(initial.advancedFilters).toEqual(createDefaultAdvancedFilters());
    // Arrays are per project: two empty mirrors must not share one.
    expect(mirrorOf(undefined).advancedFilters).not.toBe(mirrorOf(undefined).advancedFilters);
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
    expect(after.searchQuery).toBe("rock");
    expect(after.typeFilter).toEqual(["texture", "model"]);
    expect(after.sortField).toBe("size");
    expect(after.sortDirection).toBe("desc");
    expect(after.advancedFilters.minSize).toBe(1024);
    expect(after.selectedDirectory).toBe("/projects/a/Textures");
    expect(after.hasCustomConfig).toBe(true);
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
    asset(`${ROOT}/Textures/wood.png`, "texture", 2048, { width: 1024, height: 1024, has_alpha: false, color_space: "sRGB" }),
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
    activate(
      scanned("a", {
        gitStatuses: { [`${ROOT}/rock.fbx`]: "modified", [`${ROOT}/Textures/wood.png`]: "ignored" },
      })
    );
    store().setAdvancedFilters({ gitStatusFilter: ["modified", "new"] });
    expect(names()).toEqual(["rock.fbx"]);
  });

  it("sorts by the chosen field in either direction", () => {
    store().setSortField("size");
    expect(names()).toEqual(["icon.png", "Player.cs", "wood.png", "rock.fbx", "hit.wav", "theme.ogg"]);
    store().toggleSortDirection();
    expect(names()).toEqual(["theme.ogg", "hit.wav", "rock.fbx", "wood.png", "Player.cs", "icon.png"]);
    store().toggleSortDirection();
    expect(names()).toEqual(["icon.png", "Player.cs", "wood.png", "rock.fbx", "hit.wav", "theme.ogg"]);
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
    // A direct switch re-fetches git for the target: its cached state may be stale.
    expect(backend.mock.calls.filter(([c]) => c === "get_git_info").map(([, args]) => args)).toEqual([
      { projectId: "failed", path: "/projects/failed" },
      { projectId: "busy", path: "/projects/busy" },
    ]);
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
    expect(backend.mock.calls).toEqual([
      ["stop_watching", { projectId: "b" }],
      ["unregister_project", { projectId: "b" }],
    ]);
  });
});

// ---- nothing open, unknown ids ---------------------------------------------

/// The store as the app starts: nothing open, nothing undoable.
function emptyStore() {
  useProjectStore.setState(useProjectStore.getInitialState(), true);
}

describe("with nothing open", () => {
  beforeEach(emptyStore);

  it("setters, close and locate change nothing and send no command", () => {
    store().setSearchQuery("rock");
    store().setViewMode("issues");
    store().setHasCustomConfig(true);
    store().closeProject();
    store().locateAsset(`${ROOT}/rock.fbx`);
    expect(store().projects.size).toBe(0);
    expect(store().activeProjectId).toBeNull();
    expect(store().searchQuery).toBe("");
    expect(store().viewMode).toBe("assets");
    expect(store().getFilteredAssets()).toEqual([]);
    expect(commands()).toEqual([]);
  });

  it("an unknown id is ignored by switch and relocate", async () => {
    activate(scanned("a"));
    store().setActiveProject("ghost");
    await store().relocateProject("ghost", "/elsewhere");
    expect(store().activeProjectId).toBe("a");
    expect([...store().projects.keys()]).toEqual(["a"]);
    expect(commands()).toEqual([]);
  });

  it("re-selecting the active project sends nothing", () => {
    activate(scanned("a"), scanned("b"));
    store().setActiveProject("a");
    expect(commands()).toEqual([]);
  });
});

// ---- closeProject -----------------------------------------------------------

describe("closeProject", () => {
  it("closing the active project promotes the next one, mirror included", () => {
    activate(scanned("a"), scanned("b", { viewMode: "issues" }));
    store().closeProject("a");
    expect(store().projects.has("a")).toBe(false);
    expect(store().activeProjectId).toBe("b");
    expect(store().viewMode).toBe("issues");
    expectMirrorOfActive();
  });

  it("closing a non-active project leaves the active one alone, whatever the order", () => {
    activate(scanned("a"), scanned("b"), scanned("c"));
    useProjectStore.setState({ activeProjectId: "b", ...mirrorOf(store().projects.get("b")) });
    store().closeProject("c");
    expect(store().activeProjectId).toBe("b");
    expect([...store().projects.keys()]).toEqual(["a", "b"]);
  });

  it("closing the last project leaves nothing active and the mirror at its defaults", () => {
    activate(scanned("a", { searchQuery: "wood" }));
    store().closeProject();
    expect(store().activeProjectId).toBeNull();
    expect(store().projectPath).toBeNull();
    expect(store().scanResult).toBeNull();
    expect(store().searchQuery).toBe("");
  });

  it("a promoted stub is hydrated at once, so it cannot sit blank", async () => {
    activate(scanned("a"), project("stub"));
    store().closeProject("a");
    await vi.waitFor(() => expect(store().unavailable).toEqual({ kind: "missing" }));
    expect(store().activeProjectId).toBe("stub");
    expect(probes()).toBe(1);
  });
});

// ---- openProject, registerProjectStub, markProjectHealth -------------------

describe("openProject", () => {
  beforeEach(emptyStore);

  it("normalizes backslashes and records a folder that is gone as unavailable", async () => {
    await store().openProject("C:\\art\\proj");
    const [entry] = [...store().projects.values()];
    expect(entry.projectPath).toBe("C:/art/proj");
    expect(entry.unavailable).toEqual({ kind: "missing" });
    expect(store().activeProjectId).toBe(entry.id);
    expect(store().unavailable).toEqual({ kind: "missing" });
    expect(backend.mock.calls).toEqual([["check_project_paths", { paths: ["C:/art/proj"] }]]);
  });

  it("two racing opens of one new folder end as a single entry", async () => {
    await Promise.all([store().openProject("/new"), store().openProject("/new")]);
    expect([...store().projects.values()].map((p) => p.projectPath)).toEqual(["/new"]);
  });
});

describe("registerProjectStub", () => {
  beforeEach(() => {
    emptyStore();
    backend.mockImplementation(async (command) => {
      if (command === "register_project" || command === "unregister_project") return undefined;
      throw new Error(`unexpected backend command ${command}`);
    });
  });

  it("registers a path once and adds an unscanned, inactive stub", async () => {
    await store().registerProjectStub("C:\\a\\b");
    await store().registerProjectStub("C:/a/b");
    const stubs = [...store().projects.values()];
    expect(stubs.map((p) => p.projectPath)).toEqual(["C:/a/b"]);
    expect(stubs[0].scanResult).toBeNull();
    expect(store().activeProjectId).toBeNull();
    expect(backend.mock.calls).toEqual([["register_project", { projectId: stubs[0].id, path: "C:/a/b" }]]);
  });

  it("two racing registrations of one path keep one stub and unregister the loser", async () => {
    await Promise.all([store().registerProjectStub("/r"), store().registerProjectStub("/r")]);
    expect([...store().projects.values()].map((p) => p.projectPath)).toEqual(["/r"]);
    expect(commands()).toEqual(["register_project", "register_project", "unregister_project"]);
  });

  it("a registration the backend refuses adds nothing", async () => {
    backend.mockImplementation(async () => {
      throw new Error("refused");
    });
    const quiet = vi.spyOn(console, "error").mockImplementation(() => {});
    await store().registerProjectStub("/x");
    quiet.mockRestore();
    expect(store().projects.size).toBe(0);
  });
});

describe("markProjectHealth", () => {
  it("stamps only projects that know nothing yet", () => {
    activate(scanned("done"), project("fresh"), { ...project("failed"), error: "boom" });
    store().markProjectHealth(
      new Map([
        [ROOT, { kind: "missing" }],
        ["/projects/fresh", { kind: "missing" }],
        ["/projects/failed", { kind: "not_a_directory" }],
      ])
    );
    expect(store().projects.get("fresh")!.unavailable).toEqual({ kind: "missing" });
    expect(store().projects.get("done")!.unavailable).toBeNull();
    expect(store().projects.get("failed")!.unavailable).toBeNull();
    expect(store().unavailable).toBeNull();
  });

  it("an `ok` verdict is stored as null, the same as not checked", () => {
    activate(scanned("a"), project("fresh"));
    store().markProjectHealth(new Map([["/projects/fresh", { kind: "ok" }]]));
    expect(store().projects.get("fresh")!.unavailable).toBeNull();
  });

  it("the active project's mirror follows its verdict", () => {
    activate(project("fresh"));
    store().markProjectHealth(new Map([["/projects/fresh", { kind: "missing" }]]));
    expect(store().unavailable).toEqual({ kind: "missing" });
    expectMirrorOfActive();
  });
});

// ---- relocation and removal through the bridges ----------------------------

describe("relocateProject", () => {
  afterEach(() => useToastStore.setState({ toasts: [] }));

  it("refuses a folder another project already has open", async () => {
    activate(scanned("a"), scanned("b", { projectPath: "/projects/b" }));
    await store().relocateProject("a", "/projects/b");
    expect(store().projects.get("a")!.projectPath).toBe(ROOT);
    expect(commands()).toEqual([]);
    expect(useToastStore.getState().toasts.map((t) => t.kind)).toEqual(["error"]);
  });

  it("normalizes the new path", async () => {
    activate(scanned("a"));
    await store().relocateProject("a", "C:\\moved\\a");
    expect(store().projects.get("a")!.projectPath).toBe("C:/moved/a");
    expect(store().projectPath).toBe("C:/moved/a");
    expectMirrorOfActive();
  });
});

// Registered here, after every test that relies on the bridges being absent.
describe("the bridges", () => {
  it("relocation pushes the tag mirror through the bridge once the new root is in place", async () => {
    const reloads: (string | null)[] = [];
    registerTagsSyncBridge({ reloadTags: () => reloads.push(store().activeProjectId) });
    activate(scanned("a"));
    await store().relocateProject("a", "/moved-a");
    expect(reloads).toEqual(["a"]);
  });

  it("a project closed while its relocation was in flight is neither resurrected nor reloaded", async () => {
    const reloads: (string | null)[] = [];
    registerTagsSyncBridge({ reloadTags: () => reloads.push(store().activeProjectId) });
    let release!: () => void;
    backend.mockImplementation(async (command, args) => {
      if (command === "check_project_paths") {
        await new Promise<void>((resolve) => (release = resolve));
        const { paths } = args as { paths: string[] };
        return paths.map((path) => ({ path, status: { kind: "missing" } }));
      }
      if (command === "stop_watching" || command === "unregister_project") return undefined;
      throw new Error(`unexpected backend command ${command}`);
    });
    activate(scanned("a"));
    const relocating = store().relocateProject("a", "/moved-a");
    await vi.waitFor(() => expect(commands()).toContain("check_project_paths"));
    store().closeProject("a");
    release();
    await relocating;
    expect(store().projects.has("a")).toBe(false);
    expect(store().activeProjectId).toBeNull();
    expect(reloads).toEqual([]);
  });

  it("removeProject drops the project's path from recents", () => {
    const removed: string[] = [];
    registerRecentsBridge({ remove: (path) => removed.push(path) });
    activate(scanned("a"), scanned("b", { projectPath: "/projects/b" }));
    store().removeProject("b");
    expect(removed).toEqual(["/projects/b"]);
  });
});

// ---- the rest of the filters and sorts -------------------------------------

describe("getFilteredAssets, the remaining filters", () => {
  beforeEach(() => activate(scanned("a")));

  it("matches the query against the path too, case-insensitively", () => {
    store().setSearchQuery("AUDIO");
    expect(names()).toEqual(["hit.wav", "theme.ogg"]);
  });

  it("a type filter is the union of the chosen types", () => {
    store().setTypeFilter(["audio", "script"]);
    expect(names()).toEqual(["hit.wav", "Player.cs", "theme.ogg"]);
  });

  it("[] and toggling the last type away both mean no filter", () => {
    store().setTypeFilter([]);
    expect(store().typeFilter).toBeNull();
    store().toggleTypeFilter("audio");
    store().toggleTypeFilter("model");
    expect(names()).toEqual(["hit.wav", "rock.fbx", "theme.ogg"]);
    store().toggleTypeFilter("audio");
    store().toggleTypeFilter("model");
    expect(store().typeFilter).toBeNull();
    expect(names()).toHaveLength(6);
  });

  it("a directory scope stops at the component boundary", () => {
    const extra = asset(`${ROOT}/TexturesOld/old.png`, "texture", 1);
    activate(scanned("a", { scanResult: { ...scan(), assets: [...scan().assets, extra] } }));
    store().setSelectedDirectory(`${ROOT}/Textures`);
    expect(names()).toEqual(["icon.png", "wood.png"]);
  });

  it.each<[Partial<AdvancedFilters>, string[]]>([
    [{ minSize: 3000 }, ["hit.wav", "rock.fbx", "theme.ogg"]],
    [{ maxSize: 700 }, ["icon.png", "Player.cs"]],
    [{ minWidth: 100 }, ["wood.png"]],
    [{ maxWidth: 100 }, ["icon.png"]],
    [{ minHeight: 100 }, ["wood.png"]],
    [{ maxHeight: 100 }, ["icon.png"]],
    [{ minVertices: 1000 }, ["rock.fbx"]],
    [{ maxVertices: 1000 }, []],
    [{ minFaces: 600 }, ["rock.fbx"]],
    [{ maxFaces: 100 }, []],
    [{ minDuration: 10 }, ["theme.ogg"]],
    [{ maxDuration: 1.5 }, ["hit.wav"]],
    [{ hasAlpha: true }, ["icon.png"]],
    [{ hasAlpha: false }, ["wood.png"]],
    [{ colorSpace: "sRGB" }, ["wood.png"]],
    [{ extensions: ["wav", "ogg"] }, ["hit.wav", "theme.ogg"]],
  ])("advanced filter %j keeps exactly the assets it describes", (filters, expected) => {
    store().setAdvancedFilters(filters);
    expect(names()).toEqual(expected);
  });

  // Ties keep the fixture's order (sort is stable); a missing field sorts as 0.
  it.each<[SortField, string[]]>([
    ["type", ["hit.wav", "theme.ogg", "rock.fbx", "Player.cs", "wood.png", "icon.png"]],
    ["dimensions", ["rock.fbx", "hit.wav", "theme.ogg", "Player.cs", "icon.png", "wood.png"]],
    ["vertices", ["wood.png", "icon.png", "hit.wav", "theme.ogg", "Player.cs", "rock.fbx"]],
    ["faces", ["wood.png", "icon.png", "hit.wav", "theme.ogg", "Player.cs", "rock.fbx"]],
    ["duration", ["rock.fbx", "wood.png", "icon.png", "Player.cs", "hit.wav", "theme.ogg"]],
    ["sampleRate", ["rock.fbx", "wood.png", "icon.png", "hit.wav", "theme.ogg", "Player.cs"]],
    ["extension", ["Player.cs", "rock.fbx", "theme.ogg", "wood.png", "icon.png", "hit.wav"]],
  ])("sorts by %s", (field, expected) => {
    store().setSortField(field);
    expect(names()).toEqual(expected);
  });

  it("choosing the current sort field again flips the direction; a new field starts ascending", () => {
    store().setSortField("size");
    expect(store().sortDirection).toBe("asc");
    store().setSortField("size");
    expect(store().sortDirection).toBe("desc");
    store().setSortField("name");
    expect(store().sortDirection).toBe("asc");
  });
});

describe("locateAsset", () => {
  beforeEach(() => activate(scanned("a")));

  it("keeps filters that already show the asset", () => {
    store().setSearchQuery("rock");
    store().locateAsset(`${ROOT}/rock.fbx`);
    expect(store().searchQuery).toBe("rock");
  });

  it("resets advanced filters that hide the asset", () => {
    store().setAdvancedFilters({ maxDuration: 10 });
    store().locateAsset(`${ROOT}/rock.fbx`);
    expect(store().advancedFilters).toEqual(createDefaultAdvancedFilters());
    expect(names()).toContain("rock.fbx");
  });

  it("ignores a path that is not in the scan", () => {
    store().setSelectedAsset(store().scanResult!.assets[1]);
    const pulse = store().locatePulse;
    store().locateAsset(`${ROOT}/nowhere.png`);
    expect(store().selectedAsset?.name).toBe("wood.png");
    expect(store().locatePulse).toBe(pulse);
  });
});

// ---- git and undo state -----------------------------------------------------

describe("refreshGitInfo", () => {
  const repo = { is_repo: true, branch: "main", has_changes: true, ahead: 1, behind: 0 };
  const answering = (statuses: Record<string, string>) => {
    backend.mockImplementation(async (command) => {
      if (command === "get_git_info") return repo;
      if (command === "get_git_statuses") return { statuses };
      throw new Error(`unexpected backend command ${command}`);
    });
  };

  it("stores the branch and the per-file statuses of a repository", async () => {
    activate(scanned("a"));
    answering({ [`${ROOT}/rock.fbx`]: "modified" });
    await store().refreshGitInfo();
    expect(store().gitInfo).toEqual(repo);
    expect(store().gitStatuses).toEqual({ [`${ROOT}/rock.fbx`]: "modified" });
    expect(commands()).toEqual(["get_git_info", "get_git_statuses"]);
  });

  it("a folder that is not a repository keeps no statuses", async () => {
    activate(scanned("a", { gitStatuses: { [`${ROOT}/rock.fbx`]: "modified" } }));
    await store().refreshGitInfo();
    expect(store().gitInfo).toEqual({ is_repo: false });
    expect(store().gitStatuses).toEqual({});
    expect(commands()).toEqual(["get_git_info"]);
  });

  it("writes a non-active project's result into its entry, not the mirror", async () => {
    activate(scanned("a"), scanned("b", { projectPath: "/projects/b" }));
    answering({});
    await store().refreshGitInfo("b");
    expect(store().projects.get("b")!.gitInfo).toEqual(repo);
    expect(store().gitInfo).toBeNull();
    expect(backend.mock.calls[0]).toEqual(["get_git_info", { projectId: "b", path: "/projects/b" }]);
  });

  it("a failed refresh clears git state rather than keeping stale badges", async () => {
    activate(scanned("a", { gitInfo: repo, gitStatuses: { [`${ROOT}/rock.fbx`]: "new" } }));
    backend.mockImplementation(async () => {
      throw new Error("boom");
    });
    const quiet = vi.spyOn(console, "error").mockImplementation(() => {});
    await store().refreshGitInfo();
    quiet.mockRestore();
    expect(store().gitInfo).toBeNull();
    expect(store().gitStatuses).toEqual({});
  });
});

describe("undo state", () => {
  const entry: HistoryEntry = { id: "1", description: "rename", file_count: 2, timestamp: 0, can_undo: true };

  it("refreshUndoState mirrors the backend's answer for the active project", async () => {
    backend.mockImplementation(async (command) => {
      if (command === "can_undo") return true;
      if (command === "get_undo_history") return [entry];
      throw new Error(`unexpected backend command ${command}`);
    });
    activate(scanned("a"));
    await store().refreshUndoState();
    expect(store().canUndo).toBe(true);
    expect(store().undoHistory).toEqual([entry]);
  });

  it("with nothing open the undo state is cleared without asking the backend", async () => {
    emptyStore();
    useProjectStore.setState({ canUndo: true, undoHistory: [entry] });
    await store().refreshUndoState();
    expect(store().canUndo).toBe(false);
    expect(store().undoHistory).toEqual([]);
    expect(commands()).toEqual([]);
  });

  it("an answer for a project the user has left does not land on the new one", async () => {
    let release!: () => void;
    backend.mockImplementation(async (command) => {
      if (command === "can_undo") {
        await new Promise<void>((resolve) => (release = resolve));
        return true;
      }
      if (command === "get_undo_history") return [entry];
      if (command === "get_git_info") return { is_repo: false };
      throw new Error(`unexpected backend command ${command}`);
    });
    activate(scanned("a"), scanned("b", { projectPath: "/projects/b" }));
    const pending = store().refreshUndoState();
    store().setActiveProject("b");
    release();
    await pending;
    expect(store().canUndo).toBe(false);
    expect(store().undoHistory).toEqual([]);
  });

  it("clearUndoHistory empties the mirror once the backend confirms", async () => {
    backend.mockImplementation(async (command) => {
      if (command === "clear_undo_history") return undefined;
      throw new Error(`unexpected backend command ${command}`);
    });
    activate(scanned("a"));
    useProjectStore.setState({ canUndo: true, undoHistory: [entry] });
    await store().clearUndoHistory();
    expect(store().canUndo).toBe(false);
    expect(store().undoHistory).toEqual([]);
    expect(backend.mock.calls).toEqual([["clear_undo_history", { projectId: "a" }]]);
  });
});

describe("getProjectList", () => {
  it("describes each project the way the sidebar shows it", () => {
    activate(scanned("a"), { ...project("b"), unavailable: { kind: "missing" } });
    expect(store().getProjectList()).toEqual([
      { id: "a", name: "p", path: ROOT, isActive: true, assetCount: 6, issueCount: null, engine: null, unavailable: null },
      {
        id: "b",
        name: "b",
        path: "/projects/b",
        isActive: false,
        assetCount: null,
        issueCount: null,
        engine: null,
        unavailable: { kind: "missing" },
      },
    ]);
  });
});
