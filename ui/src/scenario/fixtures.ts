// The pharma benchmark's `scenario_options` as the contract (section B) describes them, for tests
// and the dev fixture API.
import type { ScenarioOptions } from './types';

export const scenarioOptions: ScenarioOptions = {
  base: ['cvx-2611-em-excursion', 'short-dated-alternate', 'reallocation-rejected'],
  disruptions: [
    {
      id: 'courier-capacity', label: 'Usual courier loses capacity', part: 'logistics', default: true,
      description: 'After the courier has given its first quote, its lane has no capacity left this week.',
      params: [{ name: 'carrier', label: 'Courier', kind: 'choice', choices: ['Tundra Medical Courier', 'Borealis Cold Chain'], default: 'Tundra Medical Courier' }],
      template: {
        id: 'courier-capacity', when: { after_calls: 1, tool: 'cold_chain_quotes' },
        do: { kind: 'lane_capacity', carrier: '{carrier}', origin: 'US', destination: 'US', capacity: 0 },
        text: '{carrier} loses capacity after its first quote.',
      },
    },
    {
      id: 'retest-fails', label: "A lot's retest fails mid-run", part: 'lims',
      description: 'A release test that was passing fails partway through, and the lot goes on hold.',
      params: [
        { name: 'lot', label: 'Lot', kind: 'choice', choices: ['CVX-2608', 'CVX-2604'], default: 'CVX-2608' },
        { name: 'after', label: 'After agent calls', kind: 'int', min: 1, max: 20, default: 4 },
      ],
      template: {
        id: 'retest-fails', when: { after_calls: '{after}' },
        do: { kind: 'result_fails', lot: '{lot}', test: 'Sterility' },
        text: "{lot}'s sterility retest fails after {after} calls to the LIMS.",
      },
    },
    {
      id: 'transit-delay', label: 'Transit delayed', part: 'logistics',
      description: 'Shipments on a lane arrive later than quoted.',
      params: [
        { name: 'carrier', label: 'Carrier', kind: 'choice', choices: ['Borealis Cold Chain', 'Tundra Medical Courier', 'Glacier Line Air'], default: 'Borealis Cold Chain' },
        { name: 'days', label: 'Extra days', kind: 'int', min: 1, max: 7, default: 2 },
      ],
      template: {
        id: 'transit-delay', when: { after_calls: 0 },
        do: { kind: 'transit_delay', carrier: '{carrier}', origin: 'US', destination: 'US', extra_days: '{days}' },
        text: 'Transit on {carrier} runs {days} days late.',
      },
    },
    {
      id: 'reallocation-rejected', label: 'First reallocation rejected', part: 'logistics',
      description: 'The supply chain director rejects the first stock reallocation proposed.',
      params: [],
      template: {
        id: 'reallocation-rejected', when: { after_calls: 0 },
        do: { kind: 'reject_next', action: 'stock_reallocation' },
        text: 'The supply chain director rejects the first stock reallocation.',
      },
    },
  ],
};
