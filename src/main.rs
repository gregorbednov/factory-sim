#[cfg(feature = "mcp")]
mod mcp;
mod network;
mod product;
mod production;
mod project;
mod runtime;
mod sim;
#[cfg(feature = "studio")]
mod studio;

use std::{
    fs,
    path::PathBuf,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, ensure};
use clap::{Parser, Subcommand};
use serde::Deserialize;

use project::{Project, Value};
use runtime::{Action, Session};
use sim::Simulation;

#[derive(Parser)]
#[command(version, about)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Open the 3D editor.
    #[cfg(feature = "studio")]
    Studio {
        project: Option<PathBuf>,
        #[arg(long)]
        backend: Option<studio::Backend>,
        /// Limit continuous viewport redraws to this frame rate.
        #[arg(long, value_parser = clap::value_parser!(u32).range(15..=240))]
        fps_limit: Option<u32>,
        /// Start simulating as soon as the editor opens.
        #[arg(long)]
        run: bool,
    },
    /// Check a project and its symbol mappings.
    Validate { project: PathBuf },
    /// Run a project without a window.
    Run {
        project: PathBuf,
        /// Run a deterministic number of ticks without a network connection.
        #[arg(long, conflicts_with = "seconds")]
        ticks: Option<u64>,
        /// Run with the wall clock and enabled I/O drivers. Omit to wait for Ctrl-C.
        #[arg(long, value_parser = clap::value_parser!(u64).range(1..))]
        seconds: Option<u64>,
        /// Read an ordered JSON array of actuator changes at specified ticks.
        #[arg(long, requires = "ticks")]
        script: Option<PathBuf>,
        /// Output periodic snapshots as JSON Lines.
        #[arg(long, value_parser = clap::value_parser!(u64).range(1..))]
        trace_every: Option<u64>,
        /// Disable the configured I/O drivers in a real-time run.
        #[arg(long)]
        no_io: bool,
    },
    /// Start a local Model Context Protocol server over stdin and stdout.
    #[cfg(feature = "mcp")]
    Mcp {
        project: PathBuf,
        #[arg(long)]
        no_io: bool,
    },
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Event {
    tick: u64,
    tag: String,
    value: Value,
}

fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();
    match Cli::parse().command {
        #[cfg(feature = "studio")]
        Commands::Studio {
            project,
            backend,
            fps_limit,
            run,
        } => studio::open(project, backend, fps_limit, run),
        Commands::Validate { project } => {
            let project = Project::load(&project)?;
            println!(
                "{}: {} parts, {} tags, {} enabled drivers",
                project.name,
                project.parts.len(),
                project.tags().len(),
                project.drivers.iter().filter(|d| d.enabled).count()
            );
            Ok(())
        }
        Commands::Run {
            project,
            ticks,
            seconds,
            script,
            trace_every,
            no_io,
        } => {
            let project = Project::load(&project)?;
            if let Some(ticks) = ticks {
                run_offline(project, ticks, script, trace_every)
            } else {
                run_realtime(project, seconds, trace_every, !no_io)
            }
        }
        #[cfg(feature = "mcp")]
        Commands::Mcp { project, no_io } => mcp::run(Project::load(&project)?, !no_io),
    }
}

fn run_offline(
    project: Project,
    ticks: u64,
    script: Option<PathBuf>,
    trace_every: Option<u64>,
) -> Result<()> {
    let events: Vec<Event> = if let Some(path) = script {
        serde_json::from_slice(
            &fs::read(&path).with_context(|| format!("read {}", path.display()))?,
        )
        .with_context(|| format!("parse {}", path.display()))?
    } else {
        Vec::new()
    };
    ensure!(
        events.windows(2).all(|win| win[0].tick <= win[1].tick),
        "script events must be sorted by tick"
    );
    ensure!(
        events.last().is_none_or(|event| event.tick < ticks),
        "script event is outside the run"
    );
    let mut sim = Simulation::new(project)?;
    let mut events = events.into_iter().peekable();
    for tick in 0..ticks {
        while events.peek().is_some_and(|event| event.tick == tick) {
            let event = events.next().unwrap();
            sim.command(&event.tag, event.value)
                .with_context(|| format!("script at tick {tick}"))?;
        }
        sim.step();
        if trace_every.is_some_and(|period| sim.tick.is_multiple_of(period)) && sim.tick != ticks {
            println!("{}", serde_json::to_string(&sim.snapshot())?);
        }
    }
    println!("{}", serde_json::to_string(&sim.snapshot())?);
    Ok(())
}

fn run_realtime(
    project: Project,
    seconds: Option<u64>,
    trace_every: Option<u64>,
    io: bool,
) -> Result<()> {
    let session = Session::new(project, io)?;
    let handle = &session.runtime.handle;
    handle.execute(Action::Run)?;
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    rt.block_on(async {
        let started = Instant::now();
        let mut timer = tokio::time::interval(Duration::from_millis(10));
        timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut last_published = 0;
        let signal = tokio::signal::ctrl_c();
        tokio::pin!(signal);
        loop {
            tokio::select! {
                result = &mut signal => { result?; break; }
                _ = timer.tick() => {
                    if seconds.is_some_and(|seconds| started.elapsed().as_secs() >= seconds) { break; }
                    let snapshot = handle.snapshot();
                    if trace_every.is_some_and(|period| snapshot.tick - last_published >= period) {
                        println!("{}", serde_json::to_string(&*snapshot)?);
                        last_published = snapshot.tick;
                    }
                }
            }
        }
        handle.execute(Action::Pause)?;
        println!("{}", serde_json::to_string(&*handle.snapshot())?);
        Ok(())
    })
}
