import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import App from "./App";
import { ErrorBoundary, installGlobalErrorLogging } from "./components/ErrorBoundary";
import { TooltipProvider } from "./components/ui/tooltip";
import "./index.css";

// Tema ilk karede doğru olsun (yanıp sönme olmasın).
document.documentElement.classList.toggle("dark", window.matchMedia("(prefers-color-scheme: dark)").matches);

installGlobalErrorLogging();

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <TooltipProvider>
      <ErrorBoundary>
        <App />
      </ErrorBoundary>
    </TooltipProvider>
  </StrictMode>,
);
