import { useEffect, useMemo, useState } from "react";
import type {
  CSSProperties,
  KeyboardEvent,
  PointerEvent as ReactPointerEvent,
} from "react";

type Pane = "left" | "right";

type PaneRules = {
  defaultWidth: number;
  min: number;
  max: number;
  storageKey: string;
  openStorageKey: string;
  collapseAt: number;
};

const rules: Record<Pane, PaneRules> = {
  left: {
    defaultWidth: 264,
    min: 216,
    max: 380,
    storageKey: "alpine.ui.leftRailWidth",
    openStorageKey: "alpine.ui.leftRailOpen",
    collapseAt: 700,
  },
  right: {
    defaultWidth: 360,
    min: 300,
    max: 620,
    storageKey: "alpine.ui.inspectorWidth",
    openStorageKey: "alpine.ui.inspectorOpen",
    collapseAt: 900,
  },
};

const clamp = (value: number, { min, max }: PaneRules) =>
  Math.min(max, Math.max(min, Math.round(value)));

function readWidth(pane: Pane) {
  try {
    const value = Number(window.localStorage.getItem(rules[pane].storageKey));
    return Number.isFinite(value) && value > 0
      ? clamp(value, rules[pane])
      : rules[pane].defaultWidth;
  } catch {
    return rules[pane].defaultWidth;
  }
}

function readOpen(pane: Pane) {
  if (window.innerWidth <= rules[pane].collapseAt) return false;
  try {
    const value = window.localStorage.getItem(rules[pane].openStorageKey);
    return value == null ? true : value === "true";
  } catch {
    return true;
  }
}

export function useWorkspaceLayout() {
  const [leftWidth, setLeftWidth] = useState(() => readWidth("left"));
  const [rightWidth, setRightWidth] = useState(() => readWidth("right"));
  const [leftOpen, setLeftOpen] = useState(() => readOpen("left"));
  const [rightOpen, setRightOpen] = useState(() => readOpen("right"));

  useEffect(
    () => window.localStorage.setItem(rules.left.storageKey, String(leftWidth)),
    [leftWidth],
  );
  useEffect(
    () =>
      window.localStorage.setItem(rules.right.storageKey, String(rightWidth)),
    [rightWidth],
  );
  useEffect(
    () =>
      window.localStorage.setItem(rules.left.openStorageKey, String(leftOpen)),
    [leftOpen],
  );
  useEffect(
    () =>
      window.localStorage.setItem(
        rules.right.openStorageKey,
        String(rightOpen),
      ),
    [rightOpen],
  );
  useEffect(() => {
    const collapseForViewport = () => {
      if (window.innerWidth <= rules.left.collapseAt) setLeftOpen(false);
      if (window.innerWidth <= rules.right.collapseAt) setRightOpen(false);
    };
    collapseForViewport();
    window.addEventListener("resize", collapseForViewport);
    return () => window.removeEventListener("resize", collapseForViewport);
  }, []);

  const setPaneWidth = (pane: Pane, value: number) => {
    const next = clamp(value, rules[pane]);
    if (pane === "left") setLeftWidth(next);
    else setRightWidth(next);
  };

  const resizeWithKeyboard = (
    pane: Pane,
    event: KeyboardEvent<HTMLDivElement>,
  ) => {
    const current = pane === "left" ? leftWidth : rightWidth;
    const direction = pane === "left" ? 1 : -1;
    if (event.key === "Home") setPaneWidth(pane, rules[pane].min);
    else if (event.key === "End") setPaneWidth(pane, rules[pane].max);
    else if (event.key === "ArrowLeft")
      setPaneWidth(pane, current - 16 * direction);
    else if (event.key === "ArrowRight")
      setPaneWidth(pane, current + 16 * direction);
    else return;
    event.preventDefault();
  };

  const beginPointerResize = (
    pane: Pane,
    event: ReactPointerEvent<HTMLDivElement>,
  ) => {
    if (event.button !== 0) return;
    event.preventDefault();
    const startX = event.clientX;
    const startWidth = pane === "left" ? leftWidth : rightWidth;
    const direction = pane === "left" ? 1 : -1;
    const previousCursor = document.body.style.cursor;
    const previousSelect = document.body.style.userSelect;
    document.body.style.cursor = "col-resize";
    document.body.style.userSelect = "none";
    const move = (moveEvent: PointerEvent) =>
      setPaneWidth(pane, startWidth + (moveEvent.clientX - startX) * direction);
    const finish = () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", finish);
      window.removeEventListener("pointercancel", finish);
      document.body.style.cursor = previousCursor;
      document.body.style.userSelect = previousSelect;
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", finish, { once: true });
    window.addEventListener("pointercancel", finish, { once: true });
  };

  const style = useMemo(
    () =>
      ({
        "--left-rail-width": `${leftWidth}px`,
        "--inspector-width": `${rightWidth}px`,
      }) as CSSProperties,
    [leftWidth, rightWidth],
  );

  return {
    left: {
      open: leftOpen,
      setOpen: setLeftOpen,
      value: leftWidth,
      ...rules.left,
      onKeyDown: (event: KeyboardEvent<HTMLDivElement>) =>
        resizeWithKeyboard("left", event),
      onPointerDown: (event: ReactPointerEvent<HTMLDivElement>) =>
        beginPointerResize("left", event),
      reset: () => setPaneWidth("left", rules.left.defaultWidth),
    },
    right: {
      open: rightOpen,
      setOpen: setRightOpen,
      value: rightWidth,
      ...rules.right,
      ensureMinimum: (minimum: number) =>
        setPaneWidth("right", Math.max(rightWidth, minimum)),
      onKeyDown: (event: KeyboardEvent<HTMLDivElement>) =>
        resizeWithKeyboard("right", event),
      onPointerDown: (event: ReactPointerEvent<HTMLDivElement>) =>
        beginPointerResize("right", event),
      reset: () => setPaneWidth("right", rules.right.defaultWidth),
    },
    style,
  };
}

type SplitDividerProps = {
  label: string;
  controls: string;
  pane: ReturnType<typeof useWorkspaceLayout>["left"];
};

export function SplitDivider({ label, controls, pane }: SplitDividerProps) {
  return (
    <div
      className={`split-divider ${controls === "project-rail" ? "left-divider" : "right-divider"}`}
      role="separator"
      aria-label={label}
      aria-controls={controls}
      aria-orientation="vertical"
      aria-valuemin={pane.min}
      aria-valuemax={pane.max}
      aria-valuenow={pane.value}
      tabIndex={0}
      onKeyDown={pane.onKeyDown}
      onPointerDown={pane.onPointerDown}
      onDoubleClick={pane.reset}
    />
  );
}
