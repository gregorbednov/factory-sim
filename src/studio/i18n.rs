use chrono::Weekday;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Language {
    English,
    #[default]
    Russian,
}

impl Language {
    pub fn text(self, label: &str) -> &str {
        if self == Self::English {
            return label;
        }

        match label {
            "Language" => "Язык",
            "Unsaved project" => "Проект не сохранён",
            "Run" => "Пуск",
            "Pause" => "Пауза",
            "Step" => "Шаг",
            "Reset" => "Сброс",
            "Settings" => "Настройки",
            "Production" => "Производство",
            "I/O" => "ПЛК",
            "Save" => "Сохранить",
            "Open" => "Открыть",
            "New" => "Новый",
            "Menu" => "Меню",
            "Save…" => "Сохранить…",
            "Save current project" => "Сохранение текущего проекта",
            "Save and open" => "Сохранить и открыть",
            "Open without saving" => "Открыть без сохранения",
            "Cancel" => "Отмена",
            "Save changes before opening the other project?" => {
                "Сохранить изменения перед открытием другого проекта?"
            }
            "Scene" => "Сцена",
            "+ Add" => "+ Добавить",
            "Equipment" => "Оборудование",
            "Stations and staff" => "Станции и персонал",
            "Products" => "Изделия",
            "Import 3D model…" => "Импорт 3D-модели…",
            "Remove object" => "Удалить объект",
            "Inspector" => "Свойства",
            "All tags" => "Все теги",
            "Select a scene object to edit its properties and watch its tags." => {
                "Выберите объект на сцене, чтобы изменить его свойства и посмотреть теги."
            }
            "Position (m)" => "Положение (м)",
            "Yaw (°)" => "Поворот (°)",
            "Product" => "Изделие",
            "Manual box" => "Ящик своих размеров",
            "Length / height / width" => "Длина / высота / ширина",
            "Max speed (m/s)" => "Макс. скорость (м/с)",
            "Stroke (m)" => "Ход (м)",
            "Speed (m/s)" => "Скорость (м/с)",
            "Range (m)" => "Дальность (м)",
            "Interval (s)" => "Интервал (с)",
            "Mass (kg)" => "Масса (кг)",
            "20 kg" => "20 кг",
            "100 kg" => "100 кг",
            "Stale" => "Устарел",
            "Running" => "Работает",
            "Paused" => "На паузе",
            "Apply changes" => "Применить изменения",
            "New project" => "Новый проект",
            "There are unsaved changes in the current project." => {
                "В текущем проекте есть несохранённые изменения."
            }
            "Create empty project" => "Создать пустой проект",
            "Untitled line" => "Линия без названия",
            "Save project" => "Сохранение проекта",
            "Open project" => "Открытие проекта",
            "Project file path" => "Путь к файлу проекта",
            "Import 3D product" => "Импорт 3D-изделия",
            "Initial simulated date, seed and work calendars" => {
                "Начальная дата симуляции, зерно генератора и рабочие календари"
            }
            "Add weekday calendar" => "Добавить календарь по будням",
            "Apply production settings" => "Применить настройки производства",
            "Graphics quality" => "Качество графики",
            "Performance" => "Производительность",
            "Balanced" => "Баланс",
            "High quality" => "Высокое качество",
            "Language and graphics" => "Язык и графика",
            "Active" => "Текущий режим",
            "Backend saved. Restart Factory Sim to apply." => {
                "Графический режим сохранён. Перезапустите Factory Sim, чтобы применить его."
            }
            "Applied on next launch." => "Будет применено при следующем запуске.",
            "Reset view" => "Сбросить вид",
            "PLC connections" => "Подключения ПЛК",
            "Driver profiles and symbol mappings" => "Профили драйверов и привязки тегов",
            "Apply profiles" => "Применить профили",
            "Millimeters" => "Миллиметры",
            "Centimeters" => "Сантиметры",
            "Meters" => "Метры",
            "Inches" => "Дюймы",
            "The model importer stopped unexpectedly" => "Импорт модели неожиданно прервался",
            "STEP (.step, .stp), OBJ or STL" => "STEP (.step, .stp), OBJ или STL",
            "Model file path" => "Путь к файлу модели",
            "File units" => "Единицы файла",
            "Up axis" => "Ось вверх",
            "Import and place" => "Импортировать и разместить",
            "Pause the simulation to add the product." => {
                "Поставьте симуляцию на паузу, чтобы добавить изделие."
            }
            "Select product" => "Выберите изделие",
            "Emission" => "Режим подачи",
            "Single product" => "Одно изделие",
            "Exact series" => "Точная серия",
            "Probabilities" => "Вероятности",
            "Skip before drawing" => "Пропуск до выбора изделия",
            "Outcome" => "Исход",
            "Skip" => "Пропуск",
            "Count " => "Количество ",
            "Remove" => "Удалить",
            "Add outcome" => "Добавить исход",
            "Unassigned probability skips an emission." => {
                "Остаток вероятности до 100 % означает пропуск."
            }
            "Remove outcome" => "Удалить исход",
            "Formula" => "Формула",
            "t: seconds, hour: 0–24, weekday: 1–7. Result: 0–1." => {
                "t — секунды, hour — час суток (0–24), weekday — день недели (1–7). Результат от 0 до 1."
            }
            "s" => "с",
            "min" => "мин",
            "h" => "ч",
            "Hz" => "Гц",
            "sim Hz" => "Гц сим.",
            "Parcels" => "Изделий",
            "Area" => "Участок",
            "Input" => "Входное изделие",
            "Good output" => "Годное изделие",
            "Scrap model" => "Модель брака",
            "Same as good output" => "Как у годного изделия",
            "Cycle" => "Цикл",
            "Repair" => "Ремонт",
            "Scrap" => "Брак",
            "Failure per cycle" => "Отказ за цикл",
            "None" => "Нет",
            "Ports in local coordinates (m)" => "Порты в локальных координатах (м)",
            "Intake" => "Приём",
            "Unload with" => "Выгрузка",
            "Built-in gripper" => "Встроенный захват",
            "Set both output points on the conveyor surfaces within the robot's reach." => {
                "Укажите точки обоих выходов на поверхности конвейеров в пределах хода робота."
            }
            "This robot unloads machines automatically." => {
                "Робот автоматически разгружает станки."
            }
            "Unload route unavailable" => "Нет доступной траектории выгрузки",
            "waiting_unload" => "Ожидание выгрузки роботом",
            "Good outlet" => "Выход годных деталей",
            "Reject output" => "Выход брака",
            "Calendar" => "Календарь",
            "Off duty" => "Вне смены",
            "Create a calendar in Production." => "Создайте календарь в разделе «Производство».",
            "Repeat program" => "Повторять программу",
            "Program" => "Программа",
            "Move" => "Переместить",
            "Rotate" => "Повернуть",
            "Wait" => "Выдержка",
            "Wait signal" => "Ждать сигнал",
            "Grab" => "Захватить",
            "Release" => "Отпустить",
            "Angle (°)" => "Угол (°)",
            "Add step" => "Добавить шаг",
            "Left drag: move    Ctrl: bypass snap    Right drag: orbit    Shift + right drag: pan    Scroll: zoom" => {
                "ЛКМ: переместить · Ctrl: без привязки · ПКМ: вращение · Shift+ПКМ: сдвиг · Колесо: масштаб"
            }
            "Right drag: orbit    Shift + right drag: pan    Scroll: zoom" => {
                "ПКМ: вращение    Shift + ПКМ: сдвиг    Колесо: масштаб"
            }

            "Conveyor" => "Ленточный конвейер",
            "Pusher" => "Толкатель",
            "Photoelectric sensor" => "Фотоэлектрический датчик",
            "Emitter" => "Источник изделий",
            "Remover" => "Удалитель изделий",
            "Crate" => "Ящик",
            "Guard" => "Ограничитель",
            "Machining center" => "Обрабатывающий центр",
            "Two-axis Pick & Place" => "Двухосевой манипулятор",
            "XYZ Pick & Place" => "Манипулятор XYZ",
            "Technician" => "Наладчик",
            "Operator" => "Оператор",
            "Passive flow guide" => "Пассивный выравниватель потока",
            "Inlet width (m)" => "Ширина входа (м)",
            "Outlet width (m)" => "Ширина выхода (м)",
            "Random spawn on conveyor" => "Случайная подача на конвейер",
            "Fixed position" => "Фиксированное положение",
            "Spawn blocked" => "Подача заблокирована",
            "Length (m)" => "Длина (м)",
            "Height (m)" => "Высота (м)",
            "Roller conveyor" => "Роликовый конвейер",
            "Stop blade" => "Выдвижной стопор",
            "Conveyor scale" => "Конвейерные весы",
            "Capacitive sensor" => "Ёмкостный датчик",
            "Light array" => "Световая завеса",
            "Stack light" => "Сигнальная колонна",
            "Fence panel" => "Секция ограждения",
            "Safety door" => "Защитная дверь",
            "Turntable" => "Поворотный стол",
            "Chain transfer" => "Цепной перегружатель",
            "Standard carton" => "Стандартная картонная коробка",
            "Tall carton" => "Высокая картонная коробка",
            "Low plastic box" => "Низкий пластиковый ящик",
            "Raw material" => "Заготовка",
            "Product base" => "Основание изделия",
            "Product lid" => "Крышка изделия",

            "Simulation running" => "Симуляция запущена",
            "Elapsed simulation time" => "Модельное время",
            "Enable" => "Разрешить работу",
            "Target speed" => "Заданная скорость",
            "Belt speed" => "Скорость ленты",
            "Extend" => "Выдвинуть",
            "Front limit" | "front" => "Передний датчик",
            "Back limit" | "back" => "Задний датчик",
            "Stroke" => "Ход",
            "Detected" | "detected" => "Обнаружено",
            "Distance" => "Расстояние",
            "Emitted" => "Подано",
            "Skipped" => "Пропущено",
            "Invalid probability" => "Ошибка вероятности",
            "Removed" => "Удалено",
            "forward" => "Вперёд",
            "reverse" => "Назад",
            "raise" => "Поднять",
            "raised" => "Поднят",
            "aligned" => "Согласована",
            "red" => "Красный",
            "yellow" => "Жёлтый",
            "green" => "Зелёный",
            "open" => "Открыть",
            "closed" => "Закрыта",
            "obstructed" => "Препятствие",
            "turn" => "Повернуть",
            "limit_0" => "Предел 0°",
            "limit_90" => "Предел 90°",
            "left" => "Влево",
            "right" => "Вправо",
            "Analog speed (V)" => "Аналоговая скорость (В)",
            "Weight (kg)" => "Масса (кг)",
            "Weight (V)" => "Сигнал весов (В)",
            "Distance (m)" => "Расстояние (м)",
            "Distance (V)" => "Сигнал расстояния (В)",
            "beam_1" => "Луч 1",
            "beam_2" => "Луч 2",
            "beam_3" => "Луч 3",
            "beam_4" => "Луч 4",
            "beam_5" => "Луч 5",
            "beam_6" => "Луч 6",
            "beam_7" => "Луч 7",
            "beam_8" => "Луч 8",
            "Blocked beams" => "Маска перекрытых лучей",
            "Blocked beams (V)" => "Сигнал завесы (В)",
            "Table angle (°)" => "Угол стола (°)",
            "Lift (m)" => "Подъём (м)",
            "Door angle (°)" => "Угол двери (°)",
            "start" => "Пуск",
            "stop" => "Стоп",
            "reset" => "Сброс",
            "emergency" => "Аварийный останов",
            "busy" | "Busy" => "Занят",
            "fault" => "Отказ",
            "input_error" => "Неподходящее изделие",
            "repairing" => "Ремонтируется",
            "waiting_operator" => "Ожидание оператора",
            "blocked" => "Выход занят",
            "interlock" => "Блокировка дверью",
            "good_count" => "Годных изделий",
            "scrap_count" => "Бракованных изделий",
            "failure_count" => "Отказов",
            "Machining progress (%)" => "Ход обработки (%)",
            "Repair progress (%)" => "Ход ремонта (%)",
            "signal" => "Сигнал",
            "moving" => "Движется",
            "holding" => "Удерживает изделие",
            "waiting" => "Ожидает",
            "done" => "Программа завершена",
            "Program step" => "Шаг программы",
            "x" => "Ось X",
            "y" => "Ось Y",
            "z" => "Ось Z",
            "angle" => "Угол захвата",
            "On duty" => "На смене",
            "starting" => "Подключение",
            "connected" => "Подключено",
            "disabled" => "Отключено",
            "disabled for this run" => "Отключено для этого запуска",
            "disconnected" => "Связь потеряна",
            "listening · no controller" => "Ожидание подключения контроллера",

            "Error" => "Ошибка",
            "invalid scale" => "Некорректный масштаб",
            "model file exceeds 32 MiB" => "Файл модели превышает 32 МиБ",
            "STEP must be a text file" => "Файл STEP должен быть текстовым",
            "CAD kernel could not tessellate this STEP file" => {
                "Ядро CAD не смогло построить сетку из файла STEP"
            }
            "read STL mesh" => "Ошибка чтения сетки STL",
            "read OBJ mesh" => "Ошибка чтения сетки OBJ",
            "model exceeds the triangle limit" => "В модели слишком много треугольников",
            "choose a STEP, OBJ or STL model" => "Выберите модель в формате STEP, OBJ или STL",
            "model must enclose a three-dimensional volume for physics" => {
                "Для симуляции модель должна иметь объём"
            }
            "invalid triangle index" => "Некорректный индекс треугольника",
            "non-finite model coordinate" => "Некорректная координата вершины",
            "expected a standalone STEP part with one data section" => {
                "Требуется отдельная деталь STEP с одной секцией данных"
            }
            "expected a STEP part with a single shell" => "Требуется деталь STEP с одной оболочкой",
            "one or more STEP faces could not be converted" => {
                "Не удалось преобразовать некоторые грани STEP"
            }
            "one or more STEP faces could not be tessellated" => {
                "Не удалось построить сетку для некоторых граней STEP"
            }
            "invalid duration" => "Некорректная длительность",
            "weekdays must be 1–7" => "Дни недели задаются числами от 1 до 7",
            "invalid shift time" => "Некорректное время смены",
            "invalid formula characters or length" => "Недопустимые символы или длина формулы",
            "unknown formula variable" => "Неизвестная переменная в формуле",
            "series must contain an outcome" => "В серии должен быть хотя бы один исход",
            "probability must be in [0, 1]" | "probability outside [0, 1]" => {
                "Вероятность должна быть от 0 до 1"
            }
            "sum of static probabilities exceeds 1" => {
                "Сумма постоянных вероятностей превышает 100 %"
            }
            "project name cannot be empty" => "У проекта должно быть название",
            "empty calendar id" => "У календаря должен быть идентификатор",
            "invalid robot speed" => "Некорректная скорость робота",
            "program must have 1–256 steps" => "В программе должно быть от 1 до 256 шагов",
            "point outside robot reach" => "Точка вне рабочей зоны робота",
            "invalid gripper angle" => "Некорректный угол захвата",
            "too many emitter outcomes" => "Слишком много исходов подачи",
            "sum of probabilities exceeds 1" => "Сумма вероятностей превышает 100 %",
            "no settings directory" => "Не удалось определить папку настроек",
            "invalid settings path" => "Некорректный путь к настройкам",
            _ => label,
        }
    }

    pub fn weekday(self, day: Weekday) -> &'static str {
        let names = match self {
            Self::English => ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"],
            Self::Russian => ["Пн", "Вт", "Ср", "Чт", "Пт", "Сб", "Вс"],
        };
        names[day.num_days_from_monday() as usize]
    }

    pub fn error(self, details: &str) -> String {
        if self == Self::English {
            return details.to_owned();
        }
        let translated = details
            .split(": ")
            .map(|item| self.text(item))
            .collect::<Vec<_>>()
            .join(": ");
        if translated == details {
            format!("{} — {details}", self.text("Error"))
        } else {
            translated
        }
    }
}
