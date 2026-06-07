import {
  flexRender,
  getCoreRowModel,
  getSortedRowModel,
  useReactTable,
} from "@tanstack/react-table";
import type { ColumnDef, SortingState } from "@tanstack/react-table";
import classNames from "classnames";
import { ArrowDown, ArrowUp, ArrowUpDown } from "lucide-react";
import React, { useMemo, useState } from "react";

import { Icon } from "../Icon";
import styles from "./DataTable.module.css";

export interface DataTableColumn<T> {
  id: string;
  header: React.ReactNode;
  cell: (row: T) => React.ReactNode;
  sortValue?: (row: T) => string | number | boolean | null | undefined;
  align?: "start" | "center" | "end";
  hideLabelOnCard?: boolean;
}

export interface DataTableProps<T> extends Omit<React.ComponentProps<"div">, "children"> {
  columns: DataTableColumn<T>[];
  rows: T[];
  getRowId: (row: T, index: number) => string;
  emptyMessage?: React.ReactNode;
  enableSorting?: boolean;
  wide?: boolean;
  caption?: React.ReactNode;
}

export function DataTable<T>({
  caption,
  className,
  columns,
  emptyMessage = "No rows yet",
  enableSorting = false,
  getRowId,
  rows,
  wide = false,
  ...props
}: DataTableProps<T>) {
  const [sorting, setSorting] = useState<SortingState>([]);

  const tableColumns = useMemo<ColumnDef<T>[]>(
    () =>
      columns.map((column) => ({
        id: column.id,
        accessorFn: column.sortValue ?? (() => undefined),
        cell: ({ row }) => column.cell(row.original),
        enableSorting: enableSorting && Boolean(column.sortValue),
        header: () => column.header,
      })),
    [columns, enableSorting],
  );

  const table = useReactTable({
    columns: tableColumns,
    data: rows,
    getCoreRowModel: getCoreRowModel(),
    getRowId,
    getSortedRowModel: enableSorting ? getSortedRowModel() : undefined,
    onSortingChange: setSorting,
    state: { sorting },
  });

  const sortedRows = table.getRowModel().rows;

  return (
    <div
      {...props}
      className={classNames(styles.root, wide && styles.wide, className)}
      data-wide={wide ? true : undefined}
    >
      <div className={styles.tableWrap}>
        <table className={styles.table}>
          {caption ? <caption className={styles.caption}>{caption}</caption> : null}
          <thead>
            {table.getHeaderGroups().map((headerGroup) => (
              <tr key={headerGroup.id}>
                {headerGroup.headers.map((header) => {
                  const sorted = header.column.getIsSorted();
                  const canSort = header.column.getCanSort();

                  return (
                    <th
                      className={styles.headerCell}
                      data-align={columns.find((column) => column.id === header.column.id)?.align}
                      key={header.id}
                      scope="col"
                    >
                      {canSort ? (
                        <button
                          className={styles.sortButton}
                          type="button"
                          onClick={header.column.getToggleSortingHandler()}
                        >
                          <span className={styles.headerContent}>
                            {flexRender(header.column.columnDef.header, header.getContext())}
                          </span>
                          <Icon
                            icon={sorted === "asc" ? ArrowUp : sorted === "desc" ? ArrowDown : ArrowUpDown}
                            size="sm"
                            tone={sorted ? "accent" : "muted"}
                          />
                        </button>
                      ) : (
                        flexRender(header.column.columnDef.header, header.getContext())
                      )}
                    </th>
                  );
                })}
              </tr>
            ))}
          </thead>
          <tbody>
            {sortedRows.length ? (
              sortedRows.map((row) => (
                <tr key={row.id}>
                  {row.getVisibleCells().map((cell) => (
                    <td
                      className={styles.cell}
                      data-align={columns.find((column) => column.id === cell.column.id)?.align}
                      key={cell.id}
                    >
                      {flexRender(cell.column.columnDef.cell, cell.getContext())}
                    </td>
                  ))}
                </tr>
              ))
            ) : (
              <tr>
                <td className={styles.emptyCell} colSpan={columns.length}>
                  {emptyMessage}
                </td>
              </tr>
            )}
          </tbody>
        </table>
      </div>
      <div className={styles.cards} aria-label={typeof caption === "string" ? caption : undefined}>
        {sortedRows.length ? (
          sortedRows.map((row) => (
            <div className={styles.card} key={row.id}>
              {columns.map((column) => (
                <div className={styles.cardField} key={column.id} data-align={column.align}>
                  {column.hideLabelOnCard ? null : (
                    <div className={styles.cardLabel}>{column.header}</div>
                  )}
                  <div className={styles.cardValue}>{column.cell(row.original)}</div>
                </div>
              ))}
            </div>
          ))
        ) : (
          <div className={styles.emptyCard}>{emptyMessage}</div>
        )}
      </div>
    </div>
  );
}
