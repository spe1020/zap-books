import { createElement, isValidElement, type ReactElement } from "react";
import { expect, test } from "vitest";
import { ErrorBoundary } from "./ErrorBoundary";

type Props = ConstructorParameters<typeof ErrorBoundary>[0];
const text = (node: unknown): string =>
  typeof node === "string"
    ? node
    : Array.isArray(node)
      ? node.map(text).join("")
      : isValidElement(node)
        ? text((node.props as { children?: unknown }).children)
        : "";

// The stored-data path that used to crash a screen is now set aside on
// load (WEB-3), so the boundary is pinned here instead: what it shows
// once a child throws, and when it tries again. React's own catching is
// what getDerivedStateFromError hooks into.
test("a screen that throws is replaced by a message, and retried on a new key", () => {
  const child = createElement("p", null, "screen");
  const props: Props = {
    children: child,
    label: "This screen",
    resetKey: "inbox",
  };
  const boundary = new ErrorBoundary(props);
  expect(boundary.render()).toBe(child);

  boundary.state = {
    ...boundary.state,
    ...ErrorBoundary.getDerivedStateFromError(),
  };
  const shown = boundary.render() as ReactElement;
  expect((shown.props as { role: string }).role).toBe("alert");
  expect(text(shown)).toContain(
    "This screen couldn’t be shown. A saved record may be damaged.",
  );
  expect(text(shown)).toContain("The rest of the app still works.");

  // Same screen: stays failed. Another screen or business: tries again.
  expect(ErrorBoundary.getDerivedStateFromProps(props, boundary.state)).toBe(
    null,
  );
  expect(
    ErrorBoundary.getDerivedStateFromProps(
      { ...props, resetKey: "settings" },
      boundary.state,
    ),
  ).toEqual({ failed: false, resetKey: "settings" });
});
