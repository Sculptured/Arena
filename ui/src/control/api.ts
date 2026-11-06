import { call, post } from '../api';
import type { ListedSuite, NewSuite, SuiteComparison, SuiteReport } from './types';

const id = encodeURIComponent;

export const suites = () => call<ListedSuite[]>('/api/suites');
export const suite = (s: string) => call<SuiteReport>(`/api/suites/${id(s)}`);
export const createSuite = (s: NewSuite) => post<SuiteReport>('/api/suites', s);
export const compareSuites = (a: string, b: string) =>
  call<SuiteComparison>(`/api/suites/compare?a=${id(a)}&b=${id(b)}`);
