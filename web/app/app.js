const byId = (id) => document.getElementById(id);
const projectControls = ["new", "open", "save", "example", "import", "production", "settings"];
const controls = ["playback", "step", "reset", "scene-toggle", "library", "inspector", "remove", ...projectControls];
const playback = byId("playback");
const formatTime = new Intl.NumberFormat("ru", { minimumFractionDigits: 1, maximumFractionDigits: 3 });
const notice = byId("notice");
const errorText = (error) => error instanceof Error ? error.message : String(error);

function showNotice(text) {
  notice.textContent = text;
  notice.hidden = !text;
}

async function startGraphics(startApp) {
  if (navigator.gpu) {
    try {
      return { demo: await startApp(byId("scene"), false), backend: "WebGPU" };
    } catch (error) {
      console.info("Factory Sim: WebGPU initialization failed; trying WebGL2", error);
      const canvas = byId("scene");
      // A canvas that already had a WebGPU context cannot open a WebGL2 context.
      canvas.replaceWith(canvas.cloneNode(false));
    }
  }

  byId("boot").textContent = "Запускается режим совместимости WebGL2…";
  try {
    return { demo: await startApp(byId("scene"), true), backend: "WebGL2" };
  } catch (error) {
    console.error("Factory Sim: WebGL2 initialization failed", error);
    throw new Error("Браузер не смог запустить WebGPU или WebGL2. Проверьте, что аппаратное ускорение включено.");
  }
}

async function boot() {
  byId("boot").textContent = "Загружается редактор Factory Sim…";
  const revision = new URL(import.meta.url).search;
  const { default: init, start_app } = await import("./pkg/factory_sim.js" + revision);
  await init({ module_or_path: new URL("./pkg/factory_sim_bg.wasm" + revision, import.meta.url) });
  const { demo, backend } = await startGraphics(start_app);
  byId("graphics").textContent = backend;
  byId("graphics").hidden = false;
  byId("splash").hidden = true;

  let last;
  let running = false;
  let unsaved = false;
  const modelDialog = byId("model-dialog");
  const projectMenu = byId("project-menu");
  const allowReplace = () => !unsaved || confirm("Заменить проект с несохранёнными изменениями?");
  const openModelDialog = () => {
    if (!running && !modelDialog.open) {
      byId("model-form").reset();
      byId("model-error").hidden = true;
      modelDialog.showModal();
    }
  };
  function guardUnsaved(event) {
    event.preventDefault();
    event.returnValue = true;
  }
  function update() {
    const current = demo.status();
    if (demo.take_import_request()) openModelDialog();
    if (current === last) return;
    last = current;
    const { seconds, name, edited, good, scrap, library_open, can_remove, inspector_open, scene_open, running: isRunning, unsaved: isUnsaved } = JSON.parse(current);
    running = isRunning;
    if (unsaved !== isUnsaved) {
      unsaved = isUnsaved;
      if (unsaved) window.addEventListener("beforeunload", guardUnsaved);
      else window.removeEventListener("beforeunload", guardUnsaved);
    }
    byId("unsaved").hidden = !unsaved;
    const nextAction = running ? "Пауза" : "Пуск";
    playback.disabled = false;
    playback.title = nextAction;
    playback.setAttribute("aria-label", nextAction);
    byId("play-icon").toggleAttribute("hidden", running);
    byId("pause-icon").toggleAttribute("hidden", !running);
    byId("step").disabled = running;
    byId("reset").disabled = false;
    byId("library").disabled = running;
    byId("library").setAttribute("aria-pressed", library_open);
    byId("scene-toggle").disabled = false;
    byId("scene-toggle").setAttribute("aria-pressed", scene_open);
    byId("inspector").disabled = false;
    byId("inspector").setAttribute("aria-pressed", inspector_open);
    byId("remove").disabled = !can_remove;
    for (const id of projectControls) byId(id).disabled = running && id !== "settings";
    byId("state").textContent = running ? "В работе" : edited ? "Пауза. Пуск или шаг применит изменения" : "На паузе";
    byId("project-name").textContent = name === "Production cell" ? "Производственная ячейка" : name;
    byId("time").textContent = `${formatTime.format(seconds)} с`;
    byId("good").textContent = good ?? "—";
    byId("scrap").textContent = scrap ?? "—";
  }

  function action(fn) {
    return async () => {
      try {
        await fn();
        showNotice("");
        update();
      } catch (error) {
        showNotice(errorText(error));
      }
    };
  }

  playback.addEventListener("click", action(() => demo.set_running(!running)));
  byId("step").addEventListener("click", action(() => demo.step()));
  byId("library").addEventListener("click", action(() => demo.toggle_library()));
  byId("scene-toggle").addEventListener("click", action(() => demo.toggle_scene()));
  byId("inspector").addEventListener("click", action(() => demo.toggle_inspector()));
  byId("remove").addEventListener("click", action(() => demo.remove_selected()));
  byId("reset").addEventListener("click", action(() => demo.reset()));
  for (const id of projectControls) {
    byId(id).addEventListener("click", () => { projectMenu.open = false; });
  }
  byId("new").addEventListener("click", action(() => { if (allowReplace()) demo.new_project(); }));
  byId("example").addEventListener("click", action(() => { if (allowReplace()) demo.open_example(); }));
  byId("production").addEventListener("click", action(() => demo.show_production()));
  byId("settings").addEventListener("click", action(() => demo.show_settings()));
  byId("open").addEventListener("click", () => byId("file").click());
  byId("import").addEventListener("click", openModelDialog);
  byId("model-cancel").addEventListener("click", () => modelDialog.close());
  byId("model-form").addEventListener("submit", async (event) => {
    event.preventDefault();
    const file = byId("model-file").files[0];
    if (!file) return;
    const error = byId("model-error");
    const submit = byId("model-submit");
    error.hidden = true;
    submit.disabled = true;
    try {
      if (file.size > 32 * 1024 * 1024) throw new Error("Модель превышает 32 МиБ.");
      demo.import_model(
        file.name,
        new Uint8Array(await file.arrayBuffer()),
        Number(byId("model-scale").value),
        byId("model-up").value === "z",
        Number(byId("model-mass").value),
      );
      modelDialog.close();
      update();
    } catch (cause) {
      error.textContent = errorText(cause);
      error.hidden = false;
    } finally {
      submit.disabled = false;
    }
  });
  byId("file").addEventListener("change", action(async () => {
    const file = byId("file").files[0];
    byId("file").value = "";
    if (file && allowReplace()) demo.replace_project(await file.text());
  }));
  byId("save").addEventListener("click", action(() => {
    const href = URL.createObjectURL(new Blob([demo.project_json()], { type: "application/json" }));
    const link = document.createElement("a");
    link.href = href;
    link.download = "factory-sim.json";
    link.click();
    demo.mark_saved();
    setTimeout(() => URL.revokeObjectURL(href), 1000);
  }));
  const timer = setInterval(update, 200);
  window.addEventListener("pagehide", (event) => {
    if (!event.persisted) {
      clearInterval(timer);
      demo.destroy();
    }
  });
  update();
}

boot().catch((error) => {
  controls.forEach((id) => { byId(id).disabled = true; });
  byId("boot").textContent = `Не удалось запустить Factory Sim. ${errorText(error)}`;
  byId("state").textContent = "Сцена недоступна";
});
