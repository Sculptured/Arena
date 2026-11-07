// The digital twin a benchmark draws (summit demo contract, section C): generic, so any world can
// give one. arenad records a `world_view` event after setup and after every step that wrote
// anything; `GET /api/branches/{id}/views` returns a branch's views, inherited ones included.

/** One thing in an area: a work order, a lot, an order, a lane, a change. */
export interface TwinItem {
  id: string;
  label: string;
  status: string;
  abnormal: boolean;
  detail?: string;
  qty?: number;
  unit?: string;
  due?: string;
  link?: string;
}

/** `plant`, `lab`, `warehouse`, `customers`, `carriers`, `approvals`, or a kind of another world. */
export type AreaKind = 'plant' | 'lab' | 'warehouse' | 'customers' | 'carriers' | 'approvals' | (string & {});

export interface TwinArea {
  id: string;
  label: string;
  kind: AreaKind;
  items: TwinItem[];
}

/** A connection between two items: for pharma, a booked shipment from a lot to an order. */
export interface TwinFlow {
  id: string;
  from: string;
  to: string;
  label: string;
  status: string;
  abnormal: boolean;
}

export interface TwinKpi {
  id: string;
  label: string;
  value: string;
  abnormal: boolean;
}

export interface TwinView {
  title: string;
  clock?: string;
  areas: TwinArea[];
  flows: TwinFlow[];
  kpis: TwinKpi[];
}

/** One entry of `GET /api/branches/{id}/views`. */
export interface BranchView {
  seq: number;
  view: TwinView;
}

/** Payload of a `world_view` event: the view after step `seq_of`. */
export interface WorldViewPayload {
  seq_of: number;
  view: TwinView;
}
