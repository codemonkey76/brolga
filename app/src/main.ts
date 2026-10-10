import { mount } from "svelte";
import App from "./App.svelte";
import "./app.css";

const target = document.getElementById("app");
if (!target) throw new Error("index.html has no #app to mount in.");

export default mount(App, { target });
