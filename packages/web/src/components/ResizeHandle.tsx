"use client";

import { useCallback, useEffect, useRef, useState } from "react";

interface ResizeHandleProps {
  direction: "horizontal" | "vertical";
  onResize: (delta: number) => void;
  onResizeEnd?: () => void;
  className?: string;
}

export function ResizeHandle({ direction, onResize, onResizeEnd, className = "" }: ResizeHandleProps) {
  const [isDragging, setIsDragging] = useState(false);
  const lastPosition = useRef<number>(0);

  const handleMouseDown = useCallback((e: React.MouseEvent) => {
    e.preventDefault();
    setIsDragging(true);
    lastPosition.current = direction === "horizontal" ? e.clientX : e.clientY;
  }, [direction]);

  const handleTouchStart = useCallback((e: React.TouchEvent) => {
    setIsDragging(true);
    const touch = e.touches[0];
    lastPosition.current = direction === "horizontal" ? touch.clientX : touch.clientY;
  }, [direction]);

  useEffect(() => {
    if (!isDragging) return;

    const handleMouseMove = (e: MouseEvent) => {
      const currentPosition = direction === "horizontal" ? e.clientX : e.clientY;
      const delta = currentPosition - lastPosition.current;
      lastPosition.current = currentPosition;
      onResize(delta);
    };

    const handleTouchMove = (e: TouchEvent) => {
      const touch = e.touches[0];
      const currentPosition = direction === "horizontal" ? touch.clientX : touch.clientY;
      const delta = currentPosition - lastPosition.current;
      lastPosition.current = currentPosition;
      onResize(delta);
    };

    const handleEnd = () => {
      setIsDragging(false);
      onResizeEnd?.();
    };

    document.addEventListener("mousemove", handleMouseMove);
    document.addEventListener("mouseup", handleEnd);
    document.addEventListener("touchmove", handleTouchMove);
    document.addEventListener("touchend", handleEnd);

    // Prevent text selection while dragging
    document.body.style.userSelect = "none";
    document.body.style.cursor = direction === "horizontal" ? "col-resize" : "row-resize";

    return () => {
      document.removeEventListener("mousemove", handleMouseMove);
      document.removeEventListener("mouseup", handleEnd);
      document.removeEventListener("touchmove", handleTouchMove);
      document.removeEventListener("touchend", handleEnd);
      document.body.style.userSelect = "";
      document.body.style.cursor = "";
    };
  }, [isDragging, direction, onResize, onResizeEnd]);

  const isHorizontal = direction === "horizontal";

  return (
    <div
      onMouseDown={handleMouseDown}
      onTouchStart={handleTouchStart}
      className={`
        ${isHorizontal ? "w-1 cursor-col-resize hover:w-1.5" : "h-1 cursor-row-resize hover:h-1.5"}
        ${isDragging ? "bg-blue-500" : "bg-gray-200 dark:bg-gray-700 hover:bg-blue-400 dark:hover:bg-blue-500"}
        transition-all duration-150 flex-shrink-0
        ${className}
      `}
      title="Drag to resize"
    />
  );
}
