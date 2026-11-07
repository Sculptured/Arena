import {
  type ColumnDef, type Row, type SortingState, type Table as TableT, flexRender, getCoreRowModel,
  getFilteredRowModel, getSortedRowModel, useReactTable,
} from '@tanstack/react-table';
import { ArrowDown, ArrowUp } from 'lucide-react';
import { type KeyboardEvent, type ReactNode, useEffect, useRef, useState } from 'react';
import { cn } from '@/lib/cn';
import { hasModifier, moveIndex, navOf } from '@/lib/keys';
import { EmptyState, ErrorState, LoadingState } from './state';
import { Table, Td, Th } from './table';

declare module '@tanstack/react-table' {
  // eslint-disable-next-line @typescript-eslint/no-unused-vars
  interface ColumnMeta<TData, TValue> {
    /** Classes for the column's cells and header (alignment, width, mono). */
    className?: string;
  }
}

export interface DataTableProps<T> {
  columns: ColumnDef<T, unknown>[];
  /** Null while loading. */
  data: T[] | null;
  getRowId: (row: T) => string;
  /** Enter on the active row, or a click. */
  onOpen?: (row: T) => void;
  /** Any filter value; `filterFn` decides which rows it keeps. */
  filter?: unknown;
  filterFn?: (row: T, filter: unknown) => boolean;
  sorting?: SortingState;
  loadingLabel: string;
  error?: string | null;
  onRetry?: () => void;
  /** Shown when there are no rows at all. */
  empty: ReactNode;
  /** Shown when rows exist but the filter hides them. */
  noMatch?: string;
  rowClassName?: (row: T) => string;
  label: string;
}

function HeaderRow<T>({ table }: { table: TableT<T> }) {
  return (
    <thead className="sticky top-0 z-10">
      {table.getHeaderGroups().map((g) => (
        <tr key={g.id}>
          {g.headers.map((h) => {
            const sort = h.column.getIsSorted();
            return (
              <Th key={h.id} className={h.column.columnDef.meta?.className}
                aria-sort={sort ? (sort === 'asc' ? 'ascending' : 'descending') : undefined}>
                {h.column.getCanSort()
                  ? <button className="inline-flex items-center gap-1 uppercase hover:text-fg" onClick={h.column.getToggleSortingHandler()}>
                    {flexRender(h.column.columnDef.header, h.getContext())}
                    {sort === 'asc' && <ArrowUp className="size-3" />}{sort === 'desc' && <ArrowDown className="size-3" />}
                  </button>
                  : flexRender(h.column.columnDef.header, h.getContext())}
              </Th>
            );
          })}
        </tr>
      ))}
    </thead>
  );
}

function BodyRow<T>({ row, active, onOpen, className }: { row: Row<T>; active: boolean; onOpen?: (r: T) => void; className?: string }) {
  return (
    <tr data-active={active || undefined} aria-selected={active}
      className={cn('cursor-default hover:bg-hover data-[active]:bg-accent-soft last:[&>td]:border-b-0', onOpen && 'cursor-pointer', className)}
      onClick={() => onOpen?.(row.original)}>
      {row.getVisibleCells().map((c) => (
        <Td key={c.id} className={c.column.columnDef.meta?.className}>{flexRender(c.column.columnDef.cell, c.getContext())}</Td>
      ))}
    </tr>
  );
}

/** Keyboard state: the active row, moved with ↑/↓ (j/k), Home/End; Enter opens it. */
function useActiveRow<T>(rows: Row<T>[], onOpen?: (r: T) => void) {
  const [active, setActive] = useState<number | null>(null);
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    ref.current?.querySelector('[data-active]')?.scrollIntoView({ block: 'nearest' });
  }, [active]);
  const onKeyDown = (e: KeyboardEvent) => {
    if (hasModifier(e)) return;
    if (e.key === 'Enter' && active != null && rows[active]) { onOpen?.(rows[active].original); return; }
    const nav = navOf(e.key);
    if (!nav) return;
    e.preventDefault();
    setActive((i) => moveIndex(i, rows.length, nav));
  };
  return { active, ref, onKeyDown };
}

/** A sortable, filterable grid with keyboard navigation and its own loading, error and empty states. */
export function DataTable<T>(p: DataTableProps<T>) {
  const [sorting, setSorting] = useState<SortingState>(p.sorting ?? []);
  const table = useReactTable({
    data: p.data ?? [], columns: p.columns, getRowId: p.getRowId,
    state: { sorting, globalFilter: p.filter }, onSortingChange: setSorting,
    getColumnCanGlobalFilter: () => !!p.filterFn,
    globalFilterFn: (row, _col, value) => p.filterFn?.(row.original, value) ?? true,
    getCoreRowModel: getCoreRowModel(), getSortedRowModel: getSortedRowModel(), getFilteredRowModel: getFilteredRowModel(),
  });
  const rows = table.getRowModel().rows;
  const nav = useActiveRow(rows, p.onOpen);
  if (p.error && !p.data) return <ErrorState error={p.error} onRetry={p.onRetry} className="m-3" />;
  if (!p.data) return <LoadingState label={p.loadingLabel} />;
  if (p.data.length === 0) return <>{p.empty}</>;
  return (
    <div ref={nav.ref} tabIndex={0} onKeyDown={nav.onKeyDown} aria-label={p.label} className="min-w-0 outline-none focus-visible:ring-2 focus-visible:ring-accent">
      <Table>
        <HeaderRow table={table} />
        <tbody>{rows.map((r, i) => <BodyRow key={r.id} row={r} active={nav.active === i} onOpen={p.onOpen} className={p.rowClassName?.(r.original)} />)}</tbody>
      </Table>
      {rows.length === 0 && <EmptyState title={p.noMatch ?? 'Nothing matches the filter.'} hint="Clear the filter to see every row." />}
    </div>
  );
}
