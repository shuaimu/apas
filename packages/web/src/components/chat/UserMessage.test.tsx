import { render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { Message } from "@/lib/store";
import { UserMessage } from "./UserMessage";

function userMessage(overrides: Partial<Message>): Message {
  return {
    id: "msg-user",
    role: "user",
    content: "",
    timestamp: new Date(2026, 5, 17, 9, 5),
    ...overrides,
  };
}

function timeOnly(date: Date) {
  return date.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
}

function monthDayTime(date: Date) {
  return `${date.toLocaleDateString([], {
    month: "short",
    day: "numeric",
  })} ${timeOnly(date)}`;
}

describe("UserMessage", () => {
  afterEach(() => {
    vi.useRealTimers();
  });

  it("renders multiline content exactly", () => {
    const content = "first line\nsecond line";

    const { container } = render(
      <UserMessage message={userMessage({ content })} />,
    );

    expect(container.querySelector("p")?.textContent).toBe(content);
  });

  it("renders same-day timestamps as time only", () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date(2026, 5, 17, 12, 0));
    const timestamp = new Date(2026, 5, 17, 9, 5);

    render(
      <UserMessage
        message={userMessage({
          content: "same-day message",
          timestamp,
        })}
      />,
    );

    expect(screen.getByText(timeOnly(timestamp))).toBeTruthy();
    expect(screen.queryByText(monthDayTime(timestamp))).toBeNull();
  });

  it("renders prior-day timestamps with month, day, and time", () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date(2026, 5, 17, 12, 0));
    const timestamp = new Date(2026, 5, 15, 18, 30);

    render(
      <UserMessage
        message={userMessage({
          content: "prior-day message",
          timestamp,
        })}
      />,
    );

    expect(screen.getByText(monthDayTime(timestamp))).toBeTruthy();
  });
});
