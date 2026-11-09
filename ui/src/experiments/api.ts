import { call, post } from '../api';
import type { Experiment, ListedExperiment, NewExperiment } from './types';

const id = encodeURIComponent;

export const experiments = () => call<ListedExperiment[]>('/api/experiments');
export const experiment = (e: string) => call<Experiment>(`/api/experiments/${id(e)}`);
export const createExperiment = (e: NewExperiment) => post<Experiment>('/api/experiments', e);
export const stopExperiment = (e: string) => post<unknown>(`/api/experiments/${id(e)}/stop`, {});
