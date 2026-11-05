import { describe, expect, it } from "vitest";
import type { SessionInfo } from "../api";
import { fileSize, filterSessions, projectName } from "./SessionsSection";

const s = (id: string, cwd: string | null, title: string | null, firstPrompt: string | null): SessionInfo => ({
  id,
  cwd,
  title,
  firstPrompt,
  lastActivity: null,
  sizeBytes: 0,
  resumable: true,
});

const list = [
  s("a", "C:\\Users\\me\\Projets\\Atelier", "Refonte", "lis la passation"),
  s("b", "c:\\users\\me\\projets\\atelier\\", null, "Build Tauri"),
  s("c", "C:/Clients/facturation", "Migration", null),
];

describe("sessions", () => {
  it("names projects by their last folder", () => {
    expect(projectName("C:\\Users\\me\\Projets\\Atelier")).toBe("Atelier");
    expect(projectName("C:/Clients/facturation/")).toBe("facturation");
    expect(projectName(null)).toBe("Dossier inconnu");
  });

  it("searches titles, prompts and folders case-insensitively", () => {
    expect(filterSessions(list, "tauri", "").map((x) => x.id)).toEqual(["b"]);
    expect(filterSessions(list, "FACTURATION", "").map((x) => x.id)).toEqual(["c"]);
    expect(filterSessions(list, "  ", "")).toHaveLength(3);
  });

  it("filters by project regardless of case and trailing separator", () => {
    const key = "c:\\users\\me\\projets\\atelier";
    expect(filterSessions(list, "", key).map((x) => x.id)).toEqual(["a", "b"]);
    expect(filterSessions(list, "refonte", key).map((x) => x.id)).toEqual(["a"]);
  });

  it("formats sizes", () => {
    expect(fileSize(512)).toBe("512 o");
    expect(fileSize(2048)).toBe("2 Ko");
    expect(fileSize(21_904_000)).toBe("20,9 Mo");
  });
});
