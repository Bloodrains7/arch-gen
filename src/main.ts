import { mount } from "svelte";
import App from "./App.svelte";
import "./app.css";

// Prevent Tauri from intercepting drag & drop events globally
// This allows our custom drag & drop to work within the webview
document.addEventListener("dragover", (e) => e.preventDefault(), true);
document.addEventListener("drop", (e) => e.preventDefault(), true);

const app = mount(App, {
  target: document.getElementById("app")!,
});

export default app;
