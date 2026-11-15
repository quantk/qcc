import { StrictMode } from "react";
import { createRoot } from "react-dom/client";

import { tauriApi } from "./api/api";
import { builtinModules } from "./modules";
import { App } from "./shell/App";
import { AppStore, provideStore } from "./state/store";
import { ui } from "./ui/registry";
import "./styles.css";

const store = new AppStore(tauriApi);
provideStore(store);
store.start();
for (const module of builtinModules) ui.activate(module);

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
