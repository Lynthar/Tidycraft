import { beforeEach, describe, expect, it, vi } from "vitest";

// The store's imports read localStorage at module init (i18n language, settings);
// Node has none, so stand one in before those imports run.
vi.hoisted(() => {
  const data = new Map<string, string>();
  Object.defineProperty(globalThis, "localStorage", {
    value: {
      getItem: (key: string) => data.get(key) ?? null,
      setItem: (key: string, value: string) => void data.set(key, value),
      removeItem: (key: string) => void data.delete(key),
    },
  });
});

import { MIRROR_FIELDS, mirrorOf, useProjectStore, type ProjectData } from "./projectStore";

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
