const byId = (id) => document.getElementById(id);
const controls = ["run", "pause", "reset", "open", "save"];
const formatTime = new Intl.NumberFormat("ru", { minimumFractionDigits: 1, maximumFractionDigits: 1 });
const notice = byId("notice");
const errorText = (error) => error instanceof Error ? error.message : String(error);

function showNotice(text) {
  notice.textContent = text;
  notice.hidden = !text;
}

async function boot() {
  if (!navigator.gpu) {
    throw new Error("WebGPU недоступен. Откройте эту страницу по HTTPS в браузере с поддержкой WebGPU.");
  }

  byId("boot").textContent = "Загружается модель производственной ячейки…";
  const { default: init, start_demo } = await import("./pkg/factory_sim.js");
  await init();
  const demo = await start_demo(byId("scene"));
  byId("splash").hidden = true;

  let last;
  function update() {
    const current = demo.status();
    if (current === last) return;
    last = current;
    const { running, seconds, name, edited, good, scrap } = JSON.parse(current);
    byId("run").disabled = running;
    byId("pause").disabled = !running;
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

  byId("run").addEventListener("click", action(() => demo.set_running(true)));
  byId("pause").addEventListener("click", action(() => demo.set_running(false)));
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
