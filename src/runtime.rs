use std::{
    sync::{
        Arc, RwLock,
        mpsc::{self, RecvTimeoutError, SyncSender},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

#[cfg(feature = "studio")]
use anyhow::ensure;
use anyhow::{Context, Result, anyhow};

use crate::{
    project::{Direction, Disconnect, Project, Value},
    sim::{HZ, Simulation, Snapshot},
};

pub enum Action {
    Run,
    Pause,
    #[cfg(any(feature = "mcp", feature = "studio"))]
    Reset,
    #[cfg(feature = "studio")]
    Step(u32),
    #[cfg(feature = "studio")]
    SetPipes(Vec<crate::project::Pipe>),
    Write(Vec<(String, Value)>),
    DriverStatus {
        driver: String,
        status: String,
        disconnected: bool,
    },
    Shutdown,
}

struct Message {
    action: Action,
    reply: Option<mpsc::Sender<Result<()>>>,
}

#[derive(Clone)]
pub struct Handle {
    sender: SyncSender<Message>,
    snapshot: Arc<RwLock<Arc<Snapshot>>>,
}

impl Handle {
    pub fn snapshot(&self) -> Arc<Snapshot> {
        self.snapshot.read().unwrap().clone()
    }

    pub fn execute(&self, action: Action) -> Result<()> {
        let (send, receive) = mpsc::channel();
        self.sender
            .try_send(Message {
                action,
                reply: Some(send),
            })
            .map_err(|e| anyhow!("simulation command queue: {e}"))?;
        receive.recv().context("simulation stopped")?
    }
}

pub struct Runtime {
    pub handle: Handle,
    thread: Option<JoinHandle<()>>,
}

impl Runtime {
    pub fn new(mut simulation: Simulation) -> Result<Self> {
        for driver in simulation.project.drivers.clone() {
            simulation.drivers.insert(
                driver.id.clone(),
                if driver.enabled {
                    "starting"
                } else {
                    "disabled"
                }
                .into(),
            );
            if driver.enabled {
                for id in driver.mapped_tags() {
                    if simulation.project.tags()[id].direction == Direction::Actuator {
                        simulation.stale(id, false);
                    }
                }
            }
        }
        let snapshot = Arc::new(RwLock::new(Arc::new(simulation.snapshot())));
        let published = snapshot.clone();
        let (sender, receiver) = mpsc::sync_channel::<Message>(256);
        let thread = thread::Builder::new()
            .name("factory-simulation".into())
            .spawn(move || {
                let mut clock = TickClock::new(Instant::now());
                let mut published_tick = simulation.tick;
                loop {
                    if simulation.running {
                        for _ in 0..clock.take_due(Instant::now()) {
                            simulation.step();
                        }
                        if simulation.tick.saturating_sub(published_tick) >= 2 {
                            publish(&simulation, &published);
                            published_tick = simulation.tick;
                        }
                    }
                    let message = if simulation.running {
                        match receiver
                            .recv_timeout(clock.next.saturating_duration_since(Instant::now()))
                        {
                            Ok(message) => message,
                            Err(RecvTimeoutError::Timeout) => continue,
                            Err(RecvTimeoutError::Disconnected) => break,
                        }
                    } else {
                        match receiver.recv() {
                            Ok(message) => message,
                            Err(_) => break,
                        }
                    };
                    if matches!(message.action, Action::Shutdown) {
                        break;
                    }
                    let was_running = simulation.running;
                    let result = apply(&mut simulation, message.action);
                    if simulation.running && !was_running {
                        clock = TickClock::new(Instant::now());
                    }
                    publish(&simulation, &published);
                    published_tick = simulation.tick;
                    if let Some(reply) = message.reply {
                        let _ = reply.send(result);
                    }
                }
            })?;
        Ok(Self {
            handle: Handle { sender, snapshot },
            thread: Some(thread),
        })
    }
}

const TICK_PERIOD: Duration = Duration::from_nanos(1_000_000_000 / HZ as u64);
const MAX_TICKS_PER_BATCH: u32 = 8;

struct TickClock {
    next: Instant,
}

impl TickClock {
    fn new(now: Instant) -> Self {
        Self {
            next: now + TICK_PERIOD,
        }
    }

    fn take_due(&mut self, now: Instant) -> u32 {
        let Some(late) = now.checked_duration_since(self.next) else {
            return 0;
        };
        let due = (late.as_nanos() / TICK_PERIOD.as_nanos() + 1)
            .min(u128::from(MAX_TICKS_PER_BATCH)) as u32;
        // Yield to pending commands between batches, keeping every outstanding tick.
        self.next += TICK_PERIOD * due;
        due
    }
}

fn publish(simulation: &Simulation, destination: &RwLock<Arc<Snapshot>>) {
    let snapshot = Arc::new(simulation.snapshot());
    *destination.write().unwrap() = snapshot;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scheduler_retains_ticks_after_a_long_stall() {
        let now = Instant::now();
        let mut clock = TickClock::new(now);
        assert_eq!(clock.take_due(now), 0);
        let resumed = now + TICK_PERIOD * 35;
        for count in [8, 8, 8, 8, 3, 0] {
            assert_eq!(clock.take_due(resumed), count);
        }
        assert_eq!(clock.take_due(resumed + TICK_PERIOD), 1);
    }
}

impl Drop for Runtime {
    fn drop(&mut self) {
        let _ = self.handle.sender.send(Message {
            action: Action::Shutdown,
            reply: None,
        });
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn apply(sim: &mut Simulation, action: Action) -> Result<()> {
    match action {
        Action::Run => sim.running = true,
        Action::Pause => sim.running = false,
        #[cfg(any(feature = "mcp", feature = "studio"))]
        Action::Reset => sim.reset()?,
        #[cfg(feature = "studio")]
        Action::Step(ticks) => {
            ensure!(!sim.running, "pause the simulation before stepping");
            ensure!(
                (1..=1200).contains(&ticks),
                "step count must be in 1..=1200"
            );
            for _ in 0..ticks {
                sim.step();
            }
        }
        #[cfg(feature = "studio")]
        Action::SetPipes(pipes) => sim.set_pipes(pipes)?,
        Action::Write(values) => sim.command_batch(values)?,
        Action::DriverStatus {
            driver,
            status,
            disconnected,
        } => {
            if disconnected {
                let config = sim
                    .project
                    .drivers
                    .iter()
                    .find(|d| d.id == driver)
                    .with_context(|| format!("unknown driver {driver}"))?;
                let reset = matches!(config.on_disconnect, Disconnect::Reset);
                let tags = sim.project.tags();
                let mapped: Vec<_> = config
                    .mapped_tags()
                    .into_iter()
                    .filter(|id| tags[*id].direction == Direction::Actuator)
                    .map(str::to_string)
                    .collect();
                for id in mapped {
                    sim.stale(&id, reset);
                }
            }
            sim.drivers.insert(driver, status);
        }
        Action::Shutdown => unreachable!("handled by scheduler"),
    }
    Ok(())
}

pub struct Session {
    // Network tasks stop before the simulation thread is joined.
    _network: Option<crate::network::Network>,
    pub runtime: Runtime,
}

impl Session {
    pub fn new(project: Project, io: bool) -> Result<Self> {
        let runtime = Runtime::new(Simulation::new(project.clone())?)?;
        let network = if io {
            crate::network::Network::start(&project.drivers, &runtime.handle)?
        } else {
            for driver in project.drivers.iter().filter(|d| d.enabled) {
                runtime.handle.execute(Action::DriverStatus {
                    driver: driver.id.clone(),
                    status: "disabled for this run".into(),
                    disconnected: true,
                })?;
            }
            None
        };
        Ok(Self {
            _network: network,
            runtime,
        })
    }
}
