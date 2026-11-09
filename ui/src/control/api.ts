import { call } from '../api';
import type { SuiteReport } from './types';

/** One suite's report: an experiment's setup (the AI comparison reads the runs of two). */
export const suite = (s: string) => call<SuiteReport>(`/api/suites/${encodeURIComponent(s)}`);
