import React from "react";
import ReactDOM from "react-dom/client";
import ClipboardOverlay from "./ClipboardOverlay";
import { syncLanguageFromSettings } from "@/i18n";

void syncLanguageFromSettings();

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <ClipboardOverlay />
  </React.StrictMode>,
);
