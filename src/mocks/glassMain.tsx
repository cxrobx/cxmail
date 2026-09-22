import React from "react";
import ReactDOM from "react-dom/client";
import "../styles/globals.css";
import GlassHarness from "./GlassHarness";

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <GlassHarness />
  </React.StrictMode>,
);
