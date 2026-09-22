import React from "react";
import ReactDOM from "react-dom/client";
import "../styles/globals.css";
import AssistHarness from "./AssistHarness";

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <AssistHarness />
  </React.StrictMode>,
);
