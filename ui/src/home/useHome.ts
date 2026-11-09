import { useCallback, useEffect, useState } from 'react';
import { call, runs as fetchRuns, suggestedTeams, tasks as fetchTasks, views as fetchViews } from '../api';
import { loadTeam, saveTeam } from '../team';
import { taskName } from './summary';
import type { BenchmarkDescription, TaskSummary, Team } from '../types';
import type { TwinView } from '../world/types';

export interface Task extends TaskSummary {
  ticket: string;
}

/** The benchmark's tasks, with their tickets. */
export function useTasks() {
  const [tasks, setTasks] = useState<Task[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => { fetchTasks().then((t) => setTasks(t as Task[])).catch((e) => setError(String(e.message ?? e))); }, []);
  return { tasks, error };
}

/** A task id in plain words (its one-line reason); the id itself while the tasks load or once the benchmark no longer lists it. */
export function useTaskNames(): (id: string) => string {
  const { tasks } = useTasks();
  return useCallback((id: string) => { const t = tasks?.find((x) => x.id === id); return t ? taskName(t) : id; }, [tasks]);
}

/** The team new runs use (kept in this browser), and a way to replace it (an import). */
export function useTeam() {
  const [team, setTeam] = useState<Team | null>(null);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => { loadTeam().then(setTeam).catch((e) => setError(String(e.message ?? e))); }, []);
  const replace = useCallback((t: Team) => { saveTeam(t); setTeam(t); }, []);
  return { team, error, replace };
}

/** The teams the benchmark suggests, the default first (empty until they load, or when they can't). */
export function useSuggestedTeams(): Team[] {
  const [teams, setTeams] = useState<Team[]>([]);
  useEffect(() => { suggestedTeams().then(setTeams).catch(() => { /* the picker is optional */ }); }, []);
  return teams;
}

/**
 * The world before a run: the benchmark's name, and its digital twin as the newest run first saw it
 * (null when it draws none or nothing ran yet).
 */
export function useWorldPreview() {
  const [bench, setBench] = useState<BenchmarkDescription | null>(null);
  const [twin, setTwin] = useState<TwinView | null>(null);
  useEffect(() => {
    let alive = true;
    (async () => {
      const b = await call<BenchmarkDescription>('/api/benchmark');
      if (!alive) return;
      setBench(b);
      if (!b.view) return;
      const newest = (await fetchRuns()).sort((x, y) => y.created_at.localeCompare(x.created_at))[0];
      const first = newest?.branches.find((br) => br.parent_id == null);
      if (!first) return;
      const v = await fetchViews(first.id);
      if (alive) setTwin(v[0]?.view ?? null);
    })().catch(() => { /* the preview is optional: the world card shows its parts instead */ });
    return () => { alive = false; };
  }, []);
  return { bench, twin };
}

/** The current time, every `ms` while `on`: a live run's clock and timeline move with it. */
export function useNow(on: boolean, ms = 1000): number {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    if (!on) return;
    const id = window.setInterval(() => setNow(Date.now()), ms);
    return () => window.clearInterval(id);
  }, [on, ms]);
  return on ? now : Date.now();
}
