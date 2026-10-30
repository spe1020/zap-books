import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import Root from "./Root";
import { ErrorBoundary } from "./components/ErrorBoundary";
import "./styles.css";
import { PRODUCT_NAME } from "./config";
document.title = PRODUCT_NAME;
createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <ErrorBoundary label="The workspace" note="Reload the page to try again.">
      <Root />
    </ErrorBoundary>
  </StrictMode>,
);
