import { describe, expect, it } from 'vitest';
import { scenarioOptions as opts } from './fixtures';
import {
  chosen, disruptionText, scenarioProblem, fill, initialSelection, outsideBase, paramProblem, previewText, recordedLine, scenarioOf,
  selectionProblem, toggled, withParam,
} from './scenario';

const retest = opts.disruptions[1];

describe('scenario builder', () => {
  it('starts from the defaults: default disruptions on, every parameter at its default', () => {
    const sel = initialSelection(opts);
    expect(chosen(opts, sel).map((d) => d.id)).toEqual(['courier-capacity']);
    expect(sel['retest-fails']).toEqual({ on: false, params: { lot: 'CVX-2608', after: 4 } });
    expect(initialSelection(null)).toEqual({});
  });

  it('previews the chosen disruptions in plain words and says the agents are not told', () => {
    const sel = initialSelection(opts);
    expect(previewText(opts, sel)).toBe('Tundra Medical Courier loses capacity after its first quote. The agents are not told.');
    const more = withParam(toggled(sel, 'transit-delay', true), 'transit-delay', opts.disruptions[2].params[1], '3');
    expect(previewText(opts, more)).toBe('Tundra Medical Courier loses capacity after its first quote. Transit on Borealis Cold Chain runs 3 days late. The agents are not told.');
    expect(previewText(opts, toggled(sel, 'courier-capacity', false))).toBe('No disruptions: the task runs as written.');
  });

  it('sends only what is on, with typed parameters, and nothing when all are off', () => {
    let sel = toggled(initialSelection(opts), 'retest-fails', true);
    sel = withParam(sel, 'retest-fails', retest.params[1], '7');
    sel = withParam(sel, 'retest-fails', retest.params[0], 'CVX-2604');
    expect(scenarioOf(opts, sel)).toEqual({ disruptions: [
      { id: 'courier-capacity', params: { carrier: 'Tundra Medical Courier' } },
      { id: 'retest-fails', params: { lot: 'CVX-2604', after: 7 } },
    ] });
    expect(scenarioOf(opts, toggled(initialSelection(opts), 'courier-capacity', false))).toBeUndefined();
  });

  it('fills templates as arenad does: a lone placeholder keeps its type, unknown ones stay', () => {
    expect(fill(retest.template, { lot: 'CVX-2608', after: 4 })).toEqual({
      id: 'retest-fails', when: { after_calls: 4 }, do: { kind: 'result_fails', lot: 'CVX-2608', test: 'Sterility' },
      text: "CVX-2608's sterility retest fails after 4 calls to the LIMS.",
    });
    expect(fill(['{x} and {y}'], { x: 1 })).toEqual(['1 and {y}']);
    expect(disruptionText({ ...retest, template: { text: 'no full stop' } }, {})).toBe('no full stop.');
    expect(disruptionText({ ...retest, template: {} }, {})).toBe("A lot's retest fails mid-run.");
  });

  it('checks parameters against their bounds and choices', () => {
    const after = retest.params[1];
    expect(paramProblem(after, 0)).toBe('After agent calls: at least 1');
    expect(paramProblem(after, 21)).toBe('After agent calls: at most 20');
    expect(paramProblem(after, 2.5)).toBe('After agent calls: a whole number');
    expect(paramProblem(retest.params[0], 'CVX-9999')).toBe('Lot: pick one of the choices');
    const bad = withParam(toggled(initialSelection(opts), 'retest-fails', true), 'retest-fails', after, '');
    expect(selectionProblem(opts, withParam(bad, 'retest-fails', after, '99'))).toBe("A lot's retest fails mid-run: After agent calls: at most 20");
    expect(selectionProblem(opts, initialSelection(opts))).toBeNull();
  });

  it('says why a run cannot start with the scenario', () => {
    const sel = initialSelection(opts);
    expect(scenarioProblem(opts, sel, ['cvx-2611-em-excursion'])).toBeNull();
    expect(scenarioProblem(opts, sel, ['carrier-no-capacity'])).toBe("The disruptions don't apply to carrier-no-capacity. Pick cvx-2611-em-excursion, short-dated-alternate, reallocation-rejected, or turn the disruptions off.");
    expect(scenarioProblem(opts, toggled(sel, 'courier-capacity', false), ['carrier-no-capacity'])).toBeNull();
  });

  it('names picked tasks the disruptions do not apply to', () => {
    expect(outsideBase(opts, ['cvx-2611-em-excursion', 'carrier-no-capacity'])).toEqual(['carrier-no-capacity']);
    expect(outsideBase(null, ['x'])).toEqual([]);
  });

  it('reads a recorded scenario back in plain words', () => {
    expect(recordedLine({ disruptions: [{ id: 'a', when: { after_calls: 1 }, do: { kind: 'k' }, text: 'Tundra loses capacity' }] }))
      .toBe('Tundra loses capacity. The agents are not told.');
    expect(recordedLine(undefined)).toBe('');
  });
});
