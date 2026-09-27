"use client";

import { useEffect, useRef, type ReactNode } from "react";

/** Keep the controls above the software keyboard without resizing the PTY. */
export function MobileTerminalViewport({ children }: { children: ReactNode }) {
  const element = useRef<HTMLElement>(null);
  useEffect(() => {
    const viewport = window.visualViewport;
    if (!viewport) return;
    let frame = 0;
    const resize = () => {
      cancelAnimationFrame(frame);
      frame = requestAnimationFrame(() => {
        const node = element.current;
        if (!node || viewport.scale !== 1) return;
        const top = Math.max(
          0,
          node.getBoundingClientRect().top - viewport.offsetTop,
        );
        node.style.maxHeight = `${Math.max(120, viewport.height - top)}px`;
      });
    };
    resize();
    viewport.addEventListener("resize", resize);
    viewport.addEventListener("scroll", resize);
    return () => {
      cancelAnimationFrame(frame);
      viewport.removeEventListener("resize", resize);
      viewport.removeEventListener("scroll", resize);
    };
  }, []);
  return (
    <section
      ref={element}
      aria-label="Mobile terminal"
      className="flex h-full min-h-0 flex-col bg-[#0a0a0a] text-white"
    >
      {children}
    </section>
  );
}
