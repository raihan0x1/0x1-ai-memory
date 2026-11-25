interface IconProps {
  size?: number;
  className?: string;
}

export function IconStream({ size = 15, className }: IconProps) {
  return (
    <svg width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.75" className={className} aria-hidden="true">
      <path d="M4 12h4l3-7 4 14 3-7h4" strokeLinecap="square" strokeLinejoin="miter" />
    </svg>
  );
}

export function IconMemory({ size = 15, className }: IconProps) {
  return (
    <svg width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.75" className={className} aria-hidden="true">
      <rect x="3" y="4" width="18" height="16" />
      <path d="M7 9h10M7 13h7" strokeLinecap="square" />
    </svg>
  );
}

export function IconPlug({ size = 15, className }: IconProps) {
  return (
    <svg width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.75" className={className} aria-hidden="true">
      <path d="M9 3v5M15 3v5M7 8h10v4a5 5 0 0 1-10 0V8ZM12 17v4" strokeLinecap="square" strokeLinejoin="miter" />
    </svg>
  );
}

export function IconEngine({ size = 15, className }: IconProps) {
  return (
    <svg width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.75" className={className} aria-hidden="true">
      <rect x="9" y="9" width="6" height="6" />
      <path d="M12 2v3M12 19v3M4.9 4.9l2.1 2.1M17 17l2.1 2.1M2 12h3M19 12h3M4.9 19.1 7 17M17 7l2.1-2.1" strokeLinecap="square" />
    </svg>
  );
}

export function IconChevronDown({ size = 14, className }: IconProps) {
  return (
    <svg width={size} height={size} viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.6" className={className} aria-hidden="true">
      <path d="M4 6l4 4 4-4" strokeLinecap="square" strokeLinejoin="miter" />
    </svg>
  );
}

export function IconCheck({ size = 13, className }: IconProps) {
  return (
    <svg width={size} height={size} viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.9" className={className} aria-hidden="true">
      <path d="M3.2 8.4L6.4 11.6L13 4.6" strokeLinecap="square" strokeLinejoin="miter" />
    </svg>
  );
}

export function IconSearch({ size = 14, className }: IconProps) {
  return (
    <svg width={size} height={size} viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.6" className={className} aria-hidden="true">
      <circle cx="7" cy="7" r="4.5" />
      <path d="M10.5 10.5L14 14" strokeLinecap="square" />
    </svg>
  );
}

export function IconList({ size = 14, className }: IconProps) {
  return (
    <svg width={size} height={size} viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.6" className={className} aria-hidden="true">
      <path d="M5 4h8M5 8h8M5 12h8M2.5 4h.5M2.5 8h.5M2.5 12h.5" strokeLinecap="square" />
    </svg>
  );
}

export function IconGraph({ size = 14, className }: IconProps) {
  return (
    <svg width={size} height={size} viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.5" className={className} aria-hidden="true">
      <circle cx="3.5" cy="8" r="2" />
      <circle cx="12.5" cy="3.5" r="2" />
      <circle cx="12.5" cy="12.5" r="2" />
      <path d="M5.3 7.1L10.7 4.4M5.3 8.9L10.7 11.6" />
    </svg>
  );
}

export function IconTimeline({ size = 14, className }: IconProps) {
  return (
    <svg width={size} height={size} viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.5" className={className} aria-hidden="true">
      <path d="M4 2.5v11M4 5h8M4 8.5h6M4 12h9" strokeLinecap="square" />
    </svg>
  );
}

export function IconPin({ size = 13, className }: IconProps) {
  return (
    <svg width={size} height={size} viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.5" className={className} aria-hidden="true">
      <path d="M5 2.5h6l-1 4.5 2 2v1H4V9l2-2-1-4.5ZM8 10v4" strokeLinecap="square" strokeLinejoin="miter" />
    </svg>
  );
}

export function IconCopy({ size = 13, className }: IconProps) {
  return (
    <svg width={size} height={size} viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.5" className={className} aria-hidden="true">
      <rect x="5.5" y="5.5" width="8" height="8" />
      <path d="M10.5 5.5V2.5h-8v8h3" strokeLinecap="square" />
    </svg>
  );
}

export function IconClose({ size = 14, className }: IconProps) {
  return (
    <svg width={size} height={size} viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.6" className={className} aria-hidden="true">
      <path d="M3.5 3.5l9 9M12.5 3.5l-9 9" strokeLinecap="square" />
    </svg>
  );
}

export function IconPause({ size = 13, className }: IconProps) {
  return (
    <svg width={size} height={size} viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.75" className={className} aria-hidden="true">
      <path d="M5.5 3.5v9M10.5 3.5v9" strokeLinecap="square" />
    </svg>
  );
}

export function IconPlay({ size = 13, className }: IconProps) {
  return (
    <svg width={size} height={size} viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.6" className={className} aria-hidden="true">
      <path d="M4.5 3l8 5-8 5V3Z" strokeLinejoin="miter" />
    </svg>
  );
}

export function IconBolt({ size = 13, className }: IconProps) {
  return (
    <svg width={size} height={size} viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.6" className={className} aria-hidden="true">
      <path d="M9 1.5L3.5 9H8l-1 5.5L12.5 7H8l1-5.5Z" strokeLinejoin="miter" />
    </svg>
  );
}

export function IconTag({ size = 12, className }: IconProps) {
  return (
    <svg width={size} height={size} viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.5" className={className} aria-hidden="true">
      <path d="M2.5 8.2V2.5h5.7l5.3 5.3-5.7 5.7-5.3-5.3Z" strokeLinejoin="miter" />
      <circle cx="5.5" cy="5.5" r="0.9" fill="currentColor" stroke="none" />
    </svg>
  );
}

export function IconFile({ size = 12, className }: IconProps) {
  return (
    <svg width={size} height={size} viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.5" className={className} aria-hidden="true">
      <path d="M3.5 2h6l3 3v9h-9V2Z" strokeLinejoin="miter" />
      <path d="M9.5 2v3h3" strokeLinejoin="miter" />
    </svg>
  );
}

export function IconLink({ size = 14, className }: IconProps) {
  return (
    <svg width={size} height={size} viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.5" className={className} aria-hidden="true">
      <path d="M6.5 9.5l3-3M5.5 7L4 8.5a2.1 2.1 0 0 0 3 3L8.5 10M7.5 6L9 4.5a2.1 2.1 0 0 1 3 3L10.5 9" strokeLinecap="square" />
    </svg>
  );
}

export function IconZoomIn({ size = 13, className }: IconProps) {
  return (
    <svg width={size} height={size} viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.6" className={className} aria-hidden="true">
      <circle cx="7" cy="7" r="4.5" />
      <path d="M7 5v4M5 7h4M10.5 10.5L14 14" strokeLinecap="square" />
    </svg>
  );
}

export function IconZoomOut({ size = 13, className }: IconProps) {
  return (
    <svg width={size} height={size} viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.6" className={className} aria-hidden="true">
      <circle cx="7" cy="7" r="4.5" />
      <path d="M5 7h4M10.5 10.5L14 14" strokeLinecap="square" />
    </svg>
  );
}

export function IconReset({ size = 13, className }: IconProps) {
  return (
    <svg width={size} height={size} viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.6" className={className} aria-hidden="true">
      <path d="M2.5 7.5A5.5 5.5 0 1 1 4 11.4M2.5 4v3.5H6" strokeLinecap="square" strokeLinejoin="miter" />
    </svg>
  );
}

export function IconPanelRight({ size = 14, className }: IconProps) {
  return (
    <svg width={size} height={size} viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.5" className={className} aria-hidden="true">
      <rect x="2" y="2.5" width="12" height="11" />
      <path d="M10 2.5v11" />
    </svg>
  );
}
