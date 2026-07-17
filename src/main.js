import "./app.css";
import AppShell from "./AppShell.svelte";
import { mount } from "svelte";

const app = mount(AppShell, {
  target: document.getElementById("app"),
});

export default app;
