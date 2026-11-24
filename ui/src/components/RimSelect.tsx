import { useCallback, useEffect, useId, useLayoutEffect, useRef, useState } from 'react';
import { IconCheck, IconChevronDown } from './Icons';

export interface SelectOption {
  value: string;
  label: string;
  meta?: string;
}

interface RimSelectProps {
  id?: string;
  ariaLabel?: string;
  value: string;
  options: SelectOption[];
  onChange: (value: string) => void;
  disabled?: boolean;
  fullWidth?: boolean;
  placeholder?: string;
}

/**
 * Rim 0px hairline custom select:
 * - Renders menu in browser top-layer via native Popover API (`popover="manual"`)
 *   so it sits cleanly above `<dialog>` modals and never clips inside overflow containers.
 * - Automatically flips upward near viewport bottom and matches trigger width.
 * - Full WAI-ARIA listbox keyboard navigation (ArrowUp/Down, Home/End, Enter, Space, Escape, typeahead).
 */
export function RimSelect({
  id,
  ariaLabel,
  value,
  options,
  onChange,
  disabled = false,
  fullWidth = false,
  placeholder = 'Select option',
}: RimSelectProps) {
  const autoId = useId();
  const triggerId = id ?? autoId;
  const listboxId = `${triggerId}-listbox`;

  const triggerRef = useRef<HTMLButtonElement>(null);
  const menuRef = useRef<HTMLDivElement>(null);
  const typeaheadRef = useRef({ buf: '', timer: 0 as ReturnType<typeof setTimeout> | number });

  const [open, setOpen] = useState(false);
  const [activeIndex, setActiveIndex] = useState<number>(() =>
    Math.max(0, options.findIndex((o) => o.value === value)),
  );
  const [menuStyle, setMenuStyle] = useState<{ top: number; left: number; minWidth: number; maxHeight: number }>({
    top: 0,
    left: 0,
    minWidth: 180,
    maxHeight: 280,
  });

  const selected = options.find((o) => o.value === value);

  const reposition = useCallback(() => {
    const btn = triggerRef.current;
    if (!btn) return;
    const rect = btn.getBoundingClientRect();
    const gap = 4;
    const estHeight = Math.min(340, options.length * 36 + 10);
    const spaceBelow = window.innerHeight - rect.bottom - gap - 8;
    const spaceAbove = rect.top - gap - 8;
    const flipUp = spaceBelow < estHeight && spaceAbove > spaceBelow;
    const maxHeight = Math.max(120, Math.min(340, flipUp ? spaceAbove : spaceBelow));
    const menuH = Math.min(estHeight, maxHeight);
    const top = flipUp ? Math.max(8, rect.top - menuH - gap) : rect.bottom + gap;
    const minWidth = Math.max(rect.width, 196);
    const left = Math.max(8, Math.min(rect.left, window.innerWidth - minWidth - 8));
    setMenuStyle({ top, left, minWidth, maxHeight });
  }, [options.length]);

  const closeMenu = useCallback((returnFocus = false) => {
    const el = menuRef.current as (HTMLDivElement & { hidePopover?: () => void }) | null;
    if (el && el.matches(':popover-open') && typeof el.hidePopover === 'function') {
      try {
        el.hidePopover();
      } catch {
        // Ignore if already closed
      }
    }
    setOpen(false);
    if (returnFocus) {
      triggerRef.current?.focus();
    }
  }, []);

  const openMenu = useCallback(() => {
    if (disabled || options.length === 0) return;
    const idx = options.findIndex((o) => o.value === value);
    setActiveIndex(idx >= 0 ? idx : 0);
    reposition();
    setOpen(true);
  }, [disabled, options, reposition, value]);

  useLayoutEffect(() => {
    const el = menuRef.current as (HTMLDivElement & { showPopover?: () => void; hidePopover?: () => void }) | null;
    if (!el) return;
    if (open) {
      reposition();
      if (typeof el.showPopover === 'function' && !el.matches(':popover-open')) {
        try {
          el.showPopover();
        } catch {
          // Fallback fixed positioning still renders cleanly
        }
      }
      el.focus({ preventScroll: true });
    } else if (typeof el.hidePopover === 'function' && el.matches(':popover-open')) {
      try {
        el.hidePopover();
      } catch {
        // Ignore
      }
    }
  }, [open, reposition]);

  // Reposition on scroll/resize and close on outside pointerdown
  useEffect(() => {
    if (!open) return;
    const onPointerDown = (e: PointerEvent) => {
      const target = e.target as Node | null;
      if (triggerRef.current?.contains(target) || menuRef.current?.contains(target)) return;
      closeMenu(false);
    };
    const onResizeOrScroll = () => reposition();
    window.addEventListener('pointerdown', onPointerDown, true);
    window.addEventListener('resize', onResizeOrScroll);
    window.addEventListener('scroll', onResizeOrScroll, true);
    return () => {
      window.removeEventListener('pointerdown', onPointerDown, true);
      window.removeEventListener('resize', onResizeOrScroll);
      window.removeEventListener('scroll', onResizeOrScroll, true);
    };
  }, [open, closeMenu, reposition]);

  // Keep active option visible inside menu scroll container
  useEffect(() => {
    if (!open || !menuRef.current) return;
    const activeEl = menuRef.current.querySelector<HTMLElement>('[data-active="true"]');
    activeEl?.scrollIntoView({ block: 'nearest' });
  }, [open, activeIndex]);

  function commit(idx: number) {
    const opt = options[idx];
    if (!opt) return;
    onChange(opt.value);
    closeMenu(true);
  }

  function handleTriggerKeyDown(e: React.KeyboardEvent<HTMLButtonElement>) {
    if (disabled) return;
    if (e.key === 'ArrowDown' || e.key === 'ArrowUp' || e.key === 'Enter' || e.key === ' ') {
      e.preventDefault();
      openMenu();
    }
  }

  function handleMenuKeyDown(e: React.KeyboardEvent<HTMLDivElement>) {
    if (e.key === 'Escape') {
      e.preventDefault();
      e.stopPropagation();
      closeMenu(true);
      return;
    }
    if (e.key === 'Tab') {
      closeMenu(false);
      return;
    }
    if (e.key === 'ArrowDown') {
      e.preventDefault();
      setActiveIndex((i) => (i + 1) % options.length);
      return;
    }
    if (e.key === 'ArrowUp') {
      e.preventDefault();
      setActiveIndex((i) => (i - 1 + options.length) % options.length);
      return;
    }
    if (e.key === 'Home') {
      e.preventDefault();
      setActiveIndex(0);
      return;
    }
    if (e.key === 'End') {
      e.preventDefault();
      setActiveIndex(options.length - 1);
      return;
    }
    if (e.key === 'Enter' || e.key === ' ') {
      e.preventDefault();
      commit(activeIndex);
      return;
    }
    if (e.key.length === 1 && !e.ctrlKey && !e.metaKey && !e.altKey) {
      const state = typeaheadRef.current;
      clearTimeout(state.timer as number);
      state.buf = (state.buf + e.key).toLowerCase();
      state.timer = window.setTimeout(() => {
        state.buf = '';
      }, 500);
      const hit = options.findIndex((o) => o.label.toLowerCase().startsWith(state.buf));
      if (hit >= 0) setActiveIndex(hit);
    }
  }

  return (
    <div className={`rim-select-wrap${fullWidth ? ' rim-select-wrap--full' : ''}`}>
      <button
        ref={triggerRef}
        id={triggerId}
        type="button"
        role="combobox"
        aria-label={ariaLabel}
        aria-haspopup="listbox"
        aria-expanded={open}
        aria-controls={listboxId}
        disabled={disabled}
        className={`rim-select-trigger${open ? ' is-open' : ''}`}
        onClick={() => (open ? closeMenu(true) : openMenu())}
        onKeyDown={handleTriggerKeyDown}
      >
        <span className="rim-select-value">{selected?.label ?? placeholder}</span>
        <IconChevronDown className="rim-select-chevron" />
      </button>

      <div
        ref={menuRef}
        id={listboxId}
        role="listbox"
        tabIndex={-1}
        aria-label={ariaLabel}
        aria-activedescendant={open && options[activeIndex] ? `${listboxId}-opt-${activeIndex}` : undefined}
        hidden={!open}
        className="rim-select-menu"
        style={{
          top: `${menuStyle.top}px`,
          left: `${menuStyle.left}px`,
          minWidth: `${menuStyle.minWidth}px`,
          maxHeight: `${menuStyle.maxHeight}px`,
        }}
        {...({ popover: 'manual' } as Record<string, string>)}
        onKeyDown={handleMenuKeyDown}
      >
        {options.map((opt, idx) => {
          const isSelected = opt.value === value;
          const isActive = idx === activeIndex;
          return (
            <div
              key={opt.value}
              id={`${listboxId}-opt-${idx}`}
              role="option"
              aria-selected={isSelected}
              data-selected={isSelected ? 'true' : undefined}
              data-active={isActive ? 'true' : undefined}
              className="rim-select-option"
              onMouseEnter={() => setActiveIndex(idx)}
              onMouseDown={(e) => {
                e.preventDefault();
                commit(idx);
              }}
            >
              <span className="rim-select-opt-mark" aria-hidden="true">
                {isSelected ? <IconCheck /> : null}
              </span>
              <span className="rim-select-opt-label">{opt.label}</span>
              {opt.meta ? <span className="rim-select-opt-meta measure">{opt.meta}</span> : null}
            </div>
          );
        })}
      </div>
    </div>
  );
}
