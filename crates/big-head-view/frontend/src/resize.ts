import { useCallback, useState, type PointerEvent as ReactPointerEvent } from 'react';

export function usePersistentNumber(key: string, initialValue: number) {
  const [value, setValue] = useState(() => {
    if (typeof window === 'undefined') return initialValue;
    const stored = window.localStorage.getItem(key);
    const parsed = stored ? Number(stored) : Number.NaN;
    return Number.isFinite(parsed) ? parsed : initialValue;
  });

  const setPersistentValue = useCallback((next: number | ((current: number) => number)) => {
    setValue((current) => {
      const resolved = typeof next === 'function' ? next(current) : next;
      if (typeof window !== 'undefined') {
        window.localStorage.setItem(key, String(resolved));
      }
      return resolved;
    });
  }, [key]);

  return [value, setPersistentValue] as const;
}

export function dragRatio(
  event: ReactPointerEvent,
  container: HTMLElement,
  axis: 'x' | 'y',
  min: number,
  max: number,
  setValue: (value: number) => void,
) {
  const update = (clientX: number, clientY: number) => {
    const rect = container.getBoundingClientRect();
    const raw =
      axis === 'x'
        ? ((clientX - rect.left) / Math.max(rect.width, 1)) * 100
        : ((clientY - rect.top) / Math.max(rect.height, 1)) * 100;
    setValue(clamp(raw, min, max));
  };

  const handleMove = (moveEvent: PointerEvent) => {
    moveEvent.preventDefault();
    update(moveEvent.clientX, moveEvent.clientY);
  };

  const handleUp = () => {
    document.body.classList.remove('is-resizing');
    window.removeEventListener('pointermove', handleMove);
    window.removeEventListener('pointerup', handleUp);
    window.removeEventListener('pointercancel', handleUp);
  };

  document.body.classList.add('is-resizing');
  update(event.clientX, event.clientY);
  window.addEventListener('pointermove', handleMove);
  window.addEventListener('pointerup', handleUp, { once: true });
  window.addEventListener('pointercancel', handleUp, { once: true });
}

function clamp(value: number, min: number, max: number) {
  return Math.min(max, Math.max(min, value));
}
