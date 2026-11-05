import { useMemo, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { History, RotateCw, Search } from "lucide-react";
import { api, errorMessage, SessionInfo } from "../api";
import { relativeTime } from "../format";

/** Last folder of a path ("C:\\a\\proj" → "proj"). */
export const projectName = (cwd: string | null) => (cwd ? (cwd.split(/[\\/]/).filter(Boolean).pop() ?? cwd) : "Dossier inconnu");

/** Windows paths compare case-insensitively ("c:\\x" and "C:\\x" are one project). */
const projectKey = (cwd: string | null) => (cwd ?? "").replace(/[\\/]+$/, "").toLowerCase();

const sizeFmt = new Intl.NumberFormat("fr-FR", { maximumFractionDigits: 1 });
export function fileSize(bytes: number) {
  if (bytes < 1024) return `${bytes} o`;
  if (bytes < 1024 ** 2) return `${sizeFmt.format(bytes / 1024)} Ko`;
  return `${sizeFmt.format(bytes / 1024 ** 2)} Mo`;
}

export function filterSessions(list: SessionInfo[], query: string, project: string) {
  const q = query.trim().toLowerCase();
  return list.filter(
    (s) =>
      (!project || projectKey(s.cwd) === project) &&
      (!q || [s.title, s.firstPrompt, s.cwd].some((t) => t?.toLowerCase().includes(q))),
  );
}

export function SessionsSection(props: { accountId: string; onResume: (s: SessionInfo) => void }) {
  const sessions = useQuery({
    queryKey: ["sessions", props.accountId],
    queryFn: () => api.listSessions(props.accountId),
    refetchOnWindowFocus: true,
  });
  const [query, setQuery] = useState("");
  const [project, setProject] = useState("");
  const list = sessions.data ?? [];
  const projects = useMemo(() => {
    const m = new Map<string, string>();
    for (const s of list) if (s.cwd && !m.has(projectKey(s.cwd))) m.set(projectKey(s.cwd), s.cwd);
    return [...m].sort((a, b) => projectName(a[1]).localeCompare(projectName(b[1]), "fr"));
  }, [list]);
  const shown = filterSessions(list, query, project);

  return (
    <div className="sessions">
      <div className="sessions-head">
        <h2>
          <History size={18} /> Conversations
        </h2>
        <button className="btn plain" onClick={() => sessions.refetch()} disabled={sessions.isFetching} aria-label="Actualiser les conversations">
          <RotateCw size={14} className={sessions.isFetching ? "spin" : ""} />
        </button>
      </div>

      {list.length > 0 && (
        <div className="sessions-tools">
          <label className="sessions-search">
            <Search size={14} aria-hidden />
            <input type="search" value={query} onChange={(e) => setQuery(e.target.value)} placeholder="Rechercher un titre, un message, un dossier" aria-label="Rechercher une conversation" />
          </label>
          {projects.length > 1 && (
            <select value={project} onChange={(e) => setProject(e.target.value)} aria-label="Filtrer par projet">
              <option value="">Tous les projets</option>
              {projects.map(([key, cwd]) => (
                <option key={key} value={key} title={cwd}>
                  {projectName(cwd)}
                </option>
              ))}
            </select>
          )}
        </div>
      )}

      <div className="group list sessions-list">
        {sessions.isLoading && <div className="row subtle">Lecture des conversations…</div>}
        {sessions.isError && <div className="row warn">{errorMessage(sessions.error)}</div>}
        {sessions.isSuccess && list.length === 0 && <div className="row subtle">Aucune conversation Claude Code sur ce compte pour l'instant.</div>}
        {list.length > 0 && shown.length === 0 && <div className="row subtle">Aucune conversation ne correspond.</div>}
        {shown.map((s) => (
          <div className="row session-row" key={s.id}>
            <div className="session-main">
              <span className="session-title" title={s.firstPrompt ?? undefined}>
                {s.title ?? s.firstPrompt ?? "Conversation sans message"}
              </span>
              <span className="session-meta">
                <span title={s.cwd ?? undefined}>{projectName(s.cwd)}</span>
                {" · "}
                {relativeTime(s.lastActivity)}
                {" · "}
                {fileSize(s.sizeBytes)}
              </span>
            </div>
            <button
              className="btn"
              disabled={!s.resumable}
              title={s.resumable ? `claude --resume ${s.id}` : "Le dossier de ce projet n'existe plus"}
              onClick={() => props.onResume(s)}
            >
              Reprendre
            </button>
          </div>
        ))}
      </div>
    </div>
  );
}
