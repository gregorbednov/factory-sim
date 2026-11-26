const byId = (id) => document.getElementById(id);
const controls = ["playback", "reset", "open", "save"];
const playback = byId("playback");
const formatTime = new Intl.NumberFormat("ru", { minimumFractionDigits: 1, maximumFractionDigits: 1 });
const notice = byId("notice");
const errorText = (error) => error instanceof Error ? error.message : String(error);

function showNotice(text) {
  notice.textContent = text;
  notice.hidden = !text;
}

async function startGraphics(startDemo) {
  if (navigator.gpu) {
    try {
      return { demo: await startDemo(byId("scene"), false), backend: "WebGPU" };
    } catch (error) {
      console.info("Factory Sim: WebGPU initialization failed; trying WebGL2", error);
      const canvas = byId("scene");
      // A canvas that already had a WebGPU context cannot open a WebGL2 context.
      canvas.replaceWith(canvas.cloneNode(false));
    }
  }

  byId("boot").textContent = "Запускается режим совместимости WebGL2…";
  try {
    return { demo: await startDemo(byId("scene"), true), backend: "WebGL2" };
  } catch (error) {
    console.error("Factory Sim: WebGL2 initialization failed", error);
    throw new Error("Браузер не смог запустить WebGPU или WebGL2. Проверьте, что аппаратное ускорение включено.");
  }
}

async function boot() {
  byId("boot").textContent = "Загружается модель производственной ячейки…";
  const revision = new URL(import.meta.url).search;
  const { default: init, start_demo } = await import("./pkg/factory_sim.js" + revision);
  await init({ module_or_path: new URL("./pkg/factory_sim_bg.wasm" + revision, import.meta.url) });
  const { demo, backend } = await startGraphics(start_demo);
  byId("graphics").textContent = backend;
  byId("graphics").hidden = false;
  byId("splash").hidden = true;

  let last;
  let running = false;
  function update() {
    const current = demo.status();
    if (current === last) return;
    last = current;
    const { seconds, name, edited, good, scrap, running: isRunning } = JSON.parse(current);
    running = isRunning;
    const nextAction = running ? "Пауза" : "Пуск";
    playback.disabled = false;
    playback.title = nextAction;
    playback.setAttribute("aria-label", nextAction);
    byId("play-icon").toggleAttribute("hidden", running);
    byId("pause-icon").toggleAttribute("hidden", !running);
    byId("reset").disabled = false;
    byId("open").disabled = running;
    byId("save").disabled = running;
    byId("state").textContent = running ? "В работе" : edited ? "Пауза. Пуск применит размещение" : "На паузе";
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
  byId("reset").addEventListener("click", action(() => demo.reset()));
  byId("open").addEventListener("click", () => byId("file").click());
  byId("file").addEventListener("change", action(async () => {
    const file = byId("file").files[0];
    byId("file").value = "";
    if (file) demo.replace_project(await file.text());
  }));
  byId("save").addEventListener("click", action(() => {
    const href = URL.createObjectURL(new Blob([demo.project_json()], { type: "application/json" }));
    const link = document.createElement("a");
    link.href = href;
    link.download = "factory-sim.json";
    link.click();
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
  byId("boot").textContent = `Демку не удалось запустить. ${errorText(error)}`;
  byId("state").textContent = "Сцена недоступна";
});
