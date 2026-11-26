use std::collections::BTreeMap;

use anyhow::{Context, Result, ensure};
use chrono::{Datelike, Days, NaiveDate, NaiveDateTime, Timelike};
use fasteval::{Evaler, ExpressionI, Parser, Slab};
use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Equipment {
    Roller,
    StopBlade,
    Scale,
    Capacitive,
    LightArray,
    StackLight,
    Fence,
    SafetyDoor,
    Turntable,
    ChainTransfer,
}

impl Equipment {
    #[cfg(feature = "studio")]
    pub const ALL: [Self; 10] = [
        Self::Roller,
        Self::StopBlade,
        Self::Scale,
        Self::Capacitive,
        Self::LightArray,
        Self::StackLight,
        Self::Fence,
        Self::SafetyDoor,
        Self::Turntable,
        Self::ChainTransfer,
    ];

    #[cfg(feature = "studio")]
    pub fn label(self) -> &'static str {
        match self {
            Self::Roller => "Roller conveyor",
            Self::StopBlade => "Stop blade",
            Self::Scale => "Conveyor scale",
            Self::Capacitive => "Capacitive sensor",
            Self::LightArray => "Light array",
            Self::StackLight => "Stack light",
            Self::Fence => "Fence panel",
            Self::SafetyDoor => "Safety door",
            Self::Turntable => "Turntable",
            Self::ChainTransfer => "Chain transfer",
        }
    }

    #[cfg(feature = "studio")]
    pub fn size(self) -> [f32; 3] {
        match self {
            Self::Roller => [2.0, 0.22, 1.1],
            Self::Scale | Self::ChainTransfer | Self::Turntable => [1.1, 0.22, 1.1],
            Self::StopBlade => [0.08, 0.12, 0.8],
            Self::Capacitive => [0.10, 0.12, 0.10],
            Self::LightArray => [1.5, 0.75, 0.08],
            Self::StackLight => [0.16, 0.6, 0.16],
            Self::Fence | Self::SafetyDoor => [1.2, 1.8, 0.08],
        }
    }
}

pub struct GuideRail {
    pub position: [f32; 3],
    pub yaw: f32,
    pub size: [f32; 3],
}

pub fn guide_rails(length: f32, height: f32, inlet: f32, outlet: f32) -> [GuideRail; 4] {
    let thickness = 0.035;
    let start = -0.5 * length;
    let end = 0.5 * length;
    let taper_end = 0.15 * length;
    let slope = (outlet - inlet) * 0.5 / (taper_end - start);
    let taper_offset = thickness * 0.5 * slope.hypot(1.0);
    std::array::from_fn(|index| {
        let side = if index < 2 { -1.0 } else { 1.0 };
        let (from_x, to_x, from_z, to_z) = if index % 2 == 0 {
            (
                start,
                taper_end,
                inlet * 0.5 + taper_offset,
                outlet * 0.5 + taper_offset,
            )
        } else {
            (
                taper_end,
                end,
                (outlet + thickness) * 0.5,
                (outlet + thickness) * 0.5,
            )
        };
        let dx = to_x - from_x;
        let dz = (to_z - from_z) * side;
        GuideRail {
            position: [(from_x + to_x) * 0.5, 0.0, (from_z + to_z) * side * 0.5],
            yaw: -dz.atan2(dx).to_degrees(),
            size: [dx.hypot(dz) + 0.012, height, thickness],
        }
    })
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Production {
    pub seed: u64,
    pub start: Option<NaiveDateTime>,
    pub calendars: BTreeMap<String, Calendar>,
}

impl Production {
    pub fn clock(&self, tick: u64) -> Option<NaiveDateTime> {
        self.start
            .unwrap_or_else(|| {
                NaiveDate::from_ymd_opt(2026, 1, 5)
                    .unwrap()
                    .and_hms_opt(8, 0, 0)
                    .unwrap()
            })
            .checked_add_signed(chrono::TimeDelta::try_seconds(
                (tick / u64::from(crate::sim::HZ)) as i64,
            )?)
    }

    pub fn validate(&self) -> Result<()> {
        for (id, calendar) in &self.calendars {
            ensure!(!id.trim().is_empty(), "empty calendar id");
            for shift in calendar
                .weekly
                .iter()
                .chain(calendar.overrides.values().flatten())
            {
                shift.validate().with_context(|| format!("calendar {id}"))?;
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Calendar {
    pub weekly: Vec<Shift>,
    pub overrides: BTreeMap<NaiveDate, Vec<Shift>>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Shift {
    #[serde(default)]
    pub days: Vec<u32>,
    pub start_minute: u32,
    pub end_minute: u32,
}

impl Shift {
    fn validate(&self) -> Result<()> {
        ensure!(
            self.days.iter().all(|day| (1..=7).contains(day)),
            "weekdays must be 1–7"
        );
        ensure!(
            self.start_minute < 1440
                && self.end_minute <= 1440
                && self.start_minute != self.end_minute,
            "invalid shift time"
        );
        Ok(())
    }
}

impl Calendar {
    pub fn on_duty(&self, clock: NaiveDateTime) -> bool {
        let date = clock.date();
        let minute = clock.hour() * 60 + clock.minute();
        let windows = |date| self.overrides.get(&date).unwrap_or(&self.weekly);
        let day = date.weekday().number_from_monday();
        let applies = |s: &Shift, date: NaiveDate, day| {
            self.overrides.contains_key(&date) || s.days.is_empty() || s.days.contains(&day)
        };
        let today = windows(date).iter().any(|s| {
            applies(s, date, day)
                && minute >= s.start_minute
                && (s.end_minute < s.start_minute || minute < s.end_minute)
        });
        if today || self.overrides.contains_key(&date) {
            return today;
        }
        date.checked_sub_days(Days::new(1)).is_some_and(|previous| {
            let weekday = previous.weekday().number_from_monday();
            windows(previous).iter().any(|s| {
                applies(s, previous, weekday)
                    && s.end_minute < s.start_minute
                    && minute < s.end_minute
            })
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StaffRole {
    Operator,
    Technician,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Staff {
    pub role: StaffRole,
    pub area: String,
    pub calendar: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Machine {
    pub area: String,
    pub input: String,
    pub output: String,
    pub scrap_product: Option<String>,
    pub seconds: f64,
    pub scrap_percent: f64,
    pub failure_percent: f64,
    pub repair_seconds: f64,
    pub intake: [f32; 3],
    pub outlet: [f32; 3],
    pub reject: [f32; 3],
    pub door: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unload_robot: Option<String>,
}

impl Default for Machine {
    fn default() -> Self {
        Self {
            area: "shop".into(),
            input: "raw-blank".into(),
            output: "machined-base".into(),
            scrap_product: None,
            seconds: 3.0,
            scrap_percent: 0.0,
            failure_percent: 0.0,
            repair_seconds: 60.0,
            intake: [-0.85, 0.0, 0.0],
            outlet: [0.85, 0.0, 0.0],
            reject: [0.0, 0.0, 0.85],
            door: None,
            unload_robot: None,
        }
    }
}

pub fn ticks(seconds: f64) -> Result<u64> {
    let ticks = (seconds * f64::from(crate::sim::HZ)).ceil();
    ensure!(
        ticks.is_finite() && ticks >= 1.0 && ticks < u64::MAX as f64,
        "invalid duration"
    );
    Ok(ticks as u64)
}

pub fn position(at: [f32; 3]) -> bool {
    at.iter()
        .all(|value| value.is_finite() && value.abs() <= 1000.0)
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum RobotStep {
    Move { at: [f32; 3] },
    Rotate { degrees: f32 },
    Wait { seconds: f64 },
    WaitSignal,
    Grab,
    Release,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Robot {
    pub two_axis: bool,
    pub repeat: bool,
    pub speed: f32,
    pub program: Vec<RobotStep>,
}

impl Default for Robot {
    fn default() -> Self {
        Self {
            two_axis: false,
            speed: 1.5,
            repeat: true,
            program: vec![
                RobotStep::Move {
                    at: [0.15, 0.35, 0.25],
                },
                RobotStep::Grab,
                RobotStep::Move {
                    at: [0.15, 0.85, 0.25],
                },
                RobotStep::Move {
                    at: [1.75, 0.85, 0.25],
                },
                RobotStep::Move {
                    at: [1.75, 0.35, 0.25],
                },
                RobotStep::Release,
                RobotStep::Move {
                    at: [1.75, 0.85, 0.25],
                },
                RobotStep::Move {
                    at: [0.15, 0.85, 0.25],
                },
            ],
        }
    }
}

impl Robot {
    pub fn reach(&self) -> [f32; 3] {
        if self.two_axis {
            [1.125, 0.625, 0.0]
        } else {
            [2.125, 1.25, 0.5]
        }
    }

    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.speed.is_finite() && (0.01..=10.0).contains(&self.speed),
            "invalid robot speed"
        );
        ensure!(
            !self.program.is_empty() && self.program.len() <= 256,
            "program must have 1–256 steps"
        );
        for step in &self.program {
            match step {
                RobotStep::Move { at } => ensure!(
                    at.iter()
                        .zip(self.reach())
                        .all(|(value, limit)| (0.0..=limit).contains(value)),
                    "point outside robot reach"
                ),
                RobotStep::Rotate { degrees } => ensure!(
                    degrees.is_finite()
                        && degrees.abs() <= 360.0
                        && (!self.two_axis || *degrees % 90.0 == 0.0),
                    "invalid gripper angle"
                ),
                RobotStep::Wait { seconds } => {
                    ticks(*seconds)?;
                }
                RobotStep::WaitSignal | RobotStep::Grab | RobotStep::Release => {}
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
pub enum Emission {
    Counts {
        rows: Vec<CountRow>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        skip: Option<Probability>,
    },
    Probability {
        rows: Vec<ProbabilityRow>,
    },
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CountRow {
    pub product: Option<String>,
    pub count: u32,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProbabilityRow {
    pub product: Option<String>,
    pub probability: Probability,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(untagged)]
pub enum Probability {
    Fixed(f64),
    Formula(String),
}

struct Formula {
    root: ExpressionI,
    slab: Slab,
}

impl Formula {
    fn parse(expression: &str) -> Result<Self> {
        ensure!(
            expression.len() <= 512
                && expression
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || b" _+*/%^().,<>=!&|-\t".contains(&c)),
            "invalid formula characters or length"
        );
        ensure!(
            !expression
                .split(|c: char| !c.is_ascii_alphanumeric() && c != '_')
                .any(|word| word == "print"),
            "print is unavailable in probability formulas"
        );
        let mut slab = Slab::new();
        let root = Parser::new()
            .parse(expression, &mut slab.ps)
            .map_err(|error| anyhow::anyhow!("{error:?}"))?;
        let formula = Self { root, slab };
        let names = formula.root.from(&formula.slab.ps).var_names(&formula.slab);
        ensure!(
            names
                .iter()
                .all(|name| ["t", "hour", "weekday"].contains(&name.as_str())),
            "unknown formula variable"
        );
        Ok(formula)
    }

    fn evaluate(&self, t: f64, clock: Option<NaiveDateTime>) -> Result<f64> {
        let hour = clock.map_or(0.0, |time| {
            f64::from(time.num_seconds_from_midnight()) / 3600.0
        });
        let weekday = clock.map_or(1.0, |time| f64::from(time.weekday().number_from_monday()));
        let mut variables = |name: &str, _: Vec<f64>| match name {
            "t" => Some(t),
            "hour" => Some(hour),
            "weekday" => Some(weekday),
            _ => None,
        };
        self.root
            .from(&self.slab.ps)
            .eval(&self.slab, &mut variables)
            .map_err(|error| anyhow::anyhow!("{error:?}"))
    }
}

enum Weight {
    Fixed(f64),
    Formula(Formula),
}
impl Weight {
    fn compile(probability: &Probability) -> Result<Self> {
        match probability {
            Probability::Fixed(value) => {
                ensure!((0.0..=1.0).contains(value), "probability must be in [0, 1]");
                Ok(Self::Fixed(*value))
            }
            Probability::Formula(expression) => Ok(Self::Formula(Formula::parse(expression)?)),
        }
    }

    fn evaluate(&self, t: f64, clock: Option<NaiveDateTime>) -> Result<f64> {
        let value = match self {
            Self::Fixed(value) => *value,
            Self::Formula(formula) => formula.evaluate(t, clock)?,
        };
        ensure!((0.0..=1.0).contains(&value), "probability outside [0, 1]");
        Ok(value)
    }
}

enum Distribution {
    Counts {
        counts: Vec<u32>,
        remaining: Vec<u32>,
        total: u64,
        skip: Option<Weight>,
    },
    Probability(Vec<Weight>),
}

pub struct Emitter {
    products: Vec<Option<String>>,
    distribution: Distribution,
}

impl Emitter {
    pub fn new(config: &Emission) -> Result<Self> {
        let (products, distribution) = match config {
            Emission::Counts { rows, skip } => {
                let counts: Vec<_> = rows.iter().map(|row| row.count).collect();
                let total = counts.iter().map(|value| u64::from(*value)).sum();
                ensure!(total > 0, "series must contain an outcome");
                (
                    rows.iter().map(|r| r.product.clone()).collect::<Vec<_>>(),
                    Distribution::Counts {
                        remaining: counts.clone(),
                        counts,
                        total,
                        skip: skip.as_ref().map(Weight::compile).transpose()?,
                    },
                )
            }
            Emission::Probability { rows } => {
                let mut fixed = 0.0;
                let mut weights = Vec::new();
                for row in rows {
                    let weight = Weight::compile(&row.probability)?;
                    if let Weight::Fixed(value) = weight {
                        fixed += value;
                    }
                    weights.push(weight);
                }
                ensure!(
                    fixed <= 1.0 + 1e-12,
                    "sum of static probabilities exceeds 1"
                );
                (
                    rows.iter().map(|r| r.product.clone()).collect::<Vec<_>>(),
                    Distribution::Probability(weights),
                )
            }
        };
        ensure!(products.len() <= 128, "too many emitter outcomes");
        Ok(Self {
            products,
            distribution,
        })
    }

    pub fn products(&self) -> impl Iterator<Item = &str> {
        self.products
            .iter()
            .filter_map(|product| product.as_deref())
    }

    pub fn next(
        &mut self,
        rng: &mut ChaCha8Rng,
        t: f64,
        clock: Option<NaiveDateTime>,
    ) -> Result<Option<String>> {
        let index = match &mut self.distribution {
            Distribution::Counts {
                counts,
                remaining,
                total,
                skip,
            } => {
                if let Some(skip) = skip
                    && rng.gen_range(0.0..1.0) < skip.evaluate(t, clock)?
                {
                    return Ok(None);
                }
                if *total == 0 {
                    remaining.clone_from(counts);
                    *total = counts.iter().map(|count| u64::from(*count)).sum();
                }
                let mut pick = rng.gen_range(0..*total);
                let index = remaining
                    .iter()
                    .position(|count| {
                        let count = u64::from(*count);
                        if pick < count {
                            true
                        } else {
                            pick -= count;
                            false
                        }
                    })
                    .unwrap();
                remaining[index] -= 1;
                *total -= 1;
                Some(index)
            }
            Distribution::Probability(weights) => {
                let pick = rng.gen_range(0.0..1.0);
                let mut total = 0.0;
                let mut outcome = None;
                for (index, weight) in weights.iter().enumerate() {
                    let probability = weight
                        .evaluate(t, clock)
                        .with_context(|| format!("outcome {}", index + 1))?;
                    total += probability;
                    if outcome.is_none() && pick < total {
                        outcome = Some(index);
                    }
                }
                ensure!(total <= 1.0 + 1e-12, "sum of probabilities exceeds 1");
                outcome
            }
        };
        Ok(index.and_then(|index| self.products[index].clone()))
    }
}

pub fn stream(seed: u64, id: &str, domain: u8) -> ChaCha8Rng {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in id.bytes().chain([domain]) {
        hash = (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3);
    }
    ChaCha8Rng::seed_from_u64(seed ^ hash)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_gate_does_not_consume_a_series_slot() -> Result<()> {
        let config = Emission::Counts {
            rows: vec![
                CountRow {
                    product: Some("a".into()),
                    count: 3,
                },
                CountRow {
                    product: Some("b".into()),
                    count: 1,
                },
            ],
            skip: Some(Probability::Formula("t < 4".into())),
        };
        let mut emitter = Emitter::new(&config)?;
        let mut rng = stream(19, "source", 0);
        for t in 0..4 {
            assert_eq!(emitter.next(&mut rng, f64::from(t), None)?, None);
        }
        for set in 0..4 {
            let mut outcomes = (0..4)
                .map(|i| emitter.next(&mut rng, f64::from(4 + set * 4 + i), None))
                .collect::<Result<Vec<_>>>()?;
            outcomes.sort();
            assert_eq!(outcomes, ["a", "a", "a", "b"].map(|p| Some(p.to_string())));
        }
        Ok(())
    }

    #[test]
    fn probabilities_obey_time_and_reject_invalid_sums() -> Result<()> {
        let config = Emission::Probability {
            rows: vec![
                ProbabilityRow {
                    product: Some("a".into()),
                    probability: Probability::Formula("(t < 2) * (weekday == 1)".into()),
                },
                ProbabilityRow {
                    product: Some("b".into()),
                    probability: Probability::Formula("t >= 3".into()),
                },
            ],
        };
        let mut emitter = Emitter::new(&config)?;
        let mut rng = stream(19, "source", 0);
        assert_eq!(
            emitter
                .next(&mut rng, 1.0, Production::default().clock(0))?
                .as_deref(),
            Some("a")
        );
        assert_eq!(emitter.next(&mut rng, 2.0, None)?, None);
        assert_eq!(emitter.next(&mut rng, 3.0, None)?.as_deref(), Some("b"));

        let mut emitter = Emitter::new(&Emission::Probability {
            rows: vec![
                ProbabilityRow {
                    product: None,
                    probability: Probability::Fixed(0.5),
                },
                ProbabilityRow {
                    product: None,
                    probability: Probability::Formula("t".into()),
                },
            ],
        })?;
        assert!(emitter.next(&mut rng, 0.6, None).is_err());
        assert!(emitter.next(&mut rng, f64::NAN, None).is_err());
        assert!(
            Emitter::new(&Emission::Probability {
                rows: vec![ProbabilityRow {
                    product: None,
                    probability: Probability::Formula("bad_symbol".into())
                },]
            })
            .is_err()
        );
        Ok(())
    }

    #[test]
    fn date_overrides_can_cancel_overnight_work() -> Result<()> {
        let mut calendar = Calendar {
            weekly: vec![Shift {
                days: vec![5],
                start_minute: 22 * 60,
                end_minute: 6 * 60,
            }],
            overrides: Default::default(),
        };
        let time = |date: &str| date.parse::<NaiveDateTime>().unwrap();
        assert!(!calendar.on_duty(time("2026-01-09T21:59:59")));
        assert!(calendar.on_duty(time("2026-01-09T22:00:00")));
        assert!(calendar.on_duty(time("2026-01-10T05:59:59")));
        assert!(!calendar.on_duty(time("2026-01-10T06:00:00")));
        calendar.overrides.insert("2026-01-10".parse()?, Vec::new());
        assert!(!calendar.on_duty(time("2026-01-10T00:00:00")));
        Ok(())
    }
}
