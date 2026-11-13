import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { App } from "./App.tsx";
import { startMotionSwitch } from "./shared/motion.ts";
import "@fontsource-variable/space-grotesk/wght.css";
import "@fontsource-variable/jetbrains-mono/wght.css";
import "@xyflow/react/dist/style.css";
import "./styles.css";

startMotionSwitch();
createRoot(document.getElementById("root") as HTMLElement).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
