// The scenario builder's model: which disruptions are on, their parameters, the templates filled
// in, the plain-words preview and the `scenario` a run or suite is started with.
import type { Disruption, DisruptionOption, ParamSpec, ParamValue, RecordedScenario, ScenarioChoice, ScenarioOptions } from './types';

/** Per offered disruption: on or off, and its parameter values. */
export type Selection = Record<string, { on: boolean; params: Record<string, ParamValue> }>;

/** Every offered disruption at its defaults: on when it says `default`, each parameter at its default. */
export function initialSelection(options: ScenarioOptions | null): Selection {
  const out: Selection = {};
  for (const d of options?.disruptions ?? []) {
    out[d.id] = { on: !!d.default, params: Object.fromEntries(d.params.map((p) => [p.name, p.default])) };
  }
  return out;
}

/** The selection with one disruption turned on or off. */
export function toggled(sel: Selection, id: string, on: boolean): Selection {
  return sel[id] ? { ...sel, [id]: { ...sel[id], on } } : sel;
}

/** The selection with one parameter set; an int parameter keeps a number. */
export function withParam(sel: Selection, id: string, spec: ParamSpec, raw: string): Selection {
  if (!sel[id]) return sel;
  const value = spec.kind === 'int' ? Number(raw) : choiceValue(spec, raw);
  return { ...sel, [id]: { ...sel[id], params: { ...sel[id].params, [spec.name]: value } } };
}

/** A choice back in its own type (a select gives strings). */
function choiceValue(spec: ParamSpec, raw: string): ParamValue {
  return spec.choices?.find((c) => String(c) === raw) ?? raw;
}

/** Why a parameter value can't be sent, or null. */
export function paramProblem(spec: ParamSpec, value: ParamValue | undefined): string | null {
  if (spec.kind === 'choice') return spec.choices && !spec.choices.some((c) => c === value) ? `${spec.label}: pick one of the choices` : null;
  if (typeof value !== 'number' || !Number.isInteger(value)) return `${spec.label}: a whole number`;
  if (spec.min != null && value < spec.min) return `${spec.label}: at least ${spec.min}`;
  if (spec.max != null && value > spec.max) return `${spec.label}: at most ${spec.max}`;
  return null;
}

/** The disruptions turned on, in the order offered. */
export function chosen(options: ScenarioOptions | null, sel: Selection): DisruptionOption[] {
  return (options?.disruptions ?? []).filter((d) => sel[d.id]?.on);
}

/** The first problem among the chosen disruptions' parameters, or null. */
export function selectionProblem(options: ScenarioOptions | null, sel: Selection): string | null {
  for (const d of chosen(options, sel)) {
    for (const p of d.params) {
      const why = paramProblem(p, sel[d.id].params[p.name]);
      if (why) return `${d.label}: ${why}`;
    }
  }
  return null;
}

/** What POST /api/runs and /api/suites take; undefined when nothing is on. */
export function scenarioOf(options: ScenarioOptions | null, sel: Selection): ScenarioChoice | undefined {
  const list = chosen(options, sel).map((d) => ({ id: d.id, params: { ...sel[d.id].params } }));
  return list.length ? { disruptions: list } : undefined;
}

const PLACEHOLDER = /\{([a-z0-9_]+)\}/gi;

/**
 * A template with its `{param}` placeholders filled, as arenad expands it. A string that is only a
 * placeholder takes the parameter's own type (`"{calls}"` becomes 2); unknown placeholders stay.
 */
export function fill(template: unknown, params: Record<string, ParamValue>): unknown {
  if (typeof template === 'string') {
    const whole = /^\{([a-z0-9_]+)\}$/i.exec(template);
    if (whole && whole[1] in params) return params[whole[1]];
    return template.replace(PLACEHOLDER, (m, name: string) => (name in params ? String(params[name]) : m));
  }
  if (Array.isArray(template)) return template.map((t) => fill(t, params));
  if (template && typeof template === 'object') {
    return Object.fromEntries(Object.entries(template).map(([k, v]) => [k, fill(v, params)]));
  }
  return template;
}

/** An option's label and description with its `{param}` placeholders filled, as arenad fills them. */
export function optionWords(d: DisruptionOption, params: Record<string, ParamValue>): { label: string; description: string } {
  return { label: String(fill(d.label, params)), description: String(fill(d.description, params)) };
}

/** One chosen disruption in plain words: its filled template's `text`, else its label. */
export function disruptionText(d: DisruptionOption, params: Record<string, ParamValue>): string {
  const filled = fill(d.template, params) as Partial<Disruption> | null;
  const text = typeof filled?.text === 'string' && filled.text.trim() ? filled.text.trim() : optionWords(d, params).label;
  return /[.!?]$/.test(text) ? text : `${text}.`;
}

export const NOT_TOLD = 'The agents are not told.';

/** "Tundra Medical Courier loses capacity after its first quote. The agents are not told." */
export function previewText(options: ScenarioOptions | null, sel: Selection): string {
  const lines = chosen(options, sel).map((d) => disruptionText(d, sel[d.id].params));
  if (!lines.length) return 'No disruptions: the task runs as written.';
  return `${lines.join(' ')} ${NOT_TOLD}`;
}

/** Tasks the disruptions don't apply to, of those picked (empty: all fine). */
export function outsideBase(options: ScenarioOptions | null, tasks: string[]): string[] {
  if (!options) return [];
  return tasks.filter((t) => !options.base.includes(t));
}

/** A recorded scenario in plain words, for the run header; empty when there was none. */
export function recordedLine(s: RecordedScenario | undefined): string {
  const texts = (s?.disruptions ?? []).map((d) => (d.disruption?.text ?? d.label)?.trim()).filter((t): t is string => !!t);
  if (!texts.length) return '';
  return `${texts.map((t) => (/[.!?]$/.test(t) ? t : `${t}.`)).join(' ')} ${NOT_TOLD}`;
}

/** Why the run or suite can't start with this scenario on these tasks, or null. */
export function scenarioProblem(options: ScenarioOptions | null, sel: Selection, tasks: string[]): string | null {
  const bad = selectionProblem(options, sel);
  if (bad) return bad;
  const outside = scenarioOf(options, sel) ? outsideBase(options, tasks) : [];
  if (!outside.length) return null;
  return `The disruptions don't apply to ${outside.join(', ')}. Pick ${options!.base.join(', ')}, or turn the disruptions off.`;
}
