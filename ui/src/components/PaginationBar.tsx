import { RimSelect } from './RimSelect';

interface PaginationBarProps {
  label: string;
  page: number;
  pageSize: number;
  total: number;
  onPageChange: (nextPage: number) => void;
  pageSizeOptions?: number[];
  onPageSizeChange?: (nextSize: number) => void;
}

type PageSlot = number | 'ellipsis-left' | 'ellipsis-right';

function buildPageSlots(current: number, totalPages: number): PageSlot[] {
  if (totalPages <= 7) {
    return Array.from({ length: totalPages }, (_, i) => i + 1);
  }
  const slots: PageSlot[] = [1];
  const start = Math.max(2, current - 1);
  const end = Math.min(totalPages - 1, current + 1);
  if (start > 2) slots.push('ellipsis-left');
  for (let p = start; p <= end; p += 1) {
    slots.push(p);
  }
  if (end < totalPages - 1) slots.push('ellipsis-right');
  slots.push(totalPages);
  return slots;
}

export function PaginationBar({
  label,
  page,
  pageSize,
  total,
  onPageChange,
  pageSizeOptions,
  onPageSizeChange,
}: PaginationBarProps) {
  if (total <= 0) return null;

  const totalPages = Math.max(1, Math.ceil(total / pageSize));
  const safePage = Math.min(Math.max(1, page), totalPages);
  const startItem = (safePage - 1) * pageSize + 1;
  const endItem = Math.min(total, safePage * pageSize);
  const slots = buildPageSlots(safePage, totalPages);

  return (
    <div className="pagination-bar">
      <span className="pagination-meta measure">
        {startItem}–{endItem} of {total.toLocaleString()}
      </span>

      <div className="pagination-controls">
        {pageSizeOptions && onPageSizeChange ? (
          <div className="pagination-size">
            <RimSelect
              ariaLabel={`${label} per page`}
              value={String(pageSize)}
              options={pageSizeOptions.map((n) => ({ value: String(n), label: `${n} / page` }))}
              onChange={(v) => onPageSizeChange(Number(v))}
            />
          </div>
        ) : null}

        {totalPages > 1 ? (
          <nav className="pagination-pages" aria-label={label}>
            <button
              type="button"
              className="btn-subtle page-btn"
              disabled={safePage <= 1}
              onClick={() => onPageChange(safePage - 1)}
            >
              Prev
            </button>
            {slots.map((slot) =>
              typeof slot === 'number' ? (
                <button
                  key={slot}
                  type="button"
                  className="btn-subtle page-btn"
                  aria-current={slot === safePage ? 'page' : undefined}
                  onClick={() => onPageChange(slot)}
                >
                  {slot}
                </button>
              ) : (
                <span key={slot} className="pagination-ellipsis" aria-hidden="true">
                  …
                </span>
              ),
            )}
            <button
              type="button"
              className="btn-subtle page-btn"
              disabled={safePage >= totalPages}
              onClick={() => onPageChange(safePage + 1)}
            >
              Next
            </button>
          </nav>
        ) : null}
      </div>
    </div>
  );
}
