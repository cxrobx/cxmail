import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { applyTheme, startGlass, useUIStore } from "@/stores/uiStore";
import "./styles/globals.css";

// Apply the saved theme and window transparency BEFORE the first render, not
// from an effect. An effect runs after the initial paint, so every launch would
// show one frame at the wrong theme and at `globals.css`'s fully-opaque alpha
// fallback — the window would snap from solid to glass in front of you.
//
// Reading through `getState()` is safe here: zustand's `persist` rehydrates
// from localStorage synchronously while the module is being imported, so the
// store already holds the saved values by the time this line runs.
//
// This also makes the one `set_native_appearance` call that corrects the dark
// boot default `.setup()` pinned before it could read localStorage.
applyTheme(useUIStore.getState().theme);

// The window boots OPAQUE (`lib.rs`) and only becomes glass once there is
// something painted inside it — a transparent window around an empty webview
// is bare wallpaper with three traffic lights floating on it (gotcha #59). Two
// frames: the first gets the render scheduled, the second lands after it has
// been composited. One frame fires early often enough to show the flash this
// exists to prevent.
requestAnimationFrame(() => requestAnimationFrame(startGlass));

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>
);
