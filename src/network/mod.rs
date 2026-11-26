mod modbus;
#[cfg(feature = "opcua")]
mod opc;

use std::time::Duration;

use anyhow::{Context, Result};
use tokio::{runtime, task::JoinHandle, time};

use crate::{
    project::{Driver, Protocol},
    runtime::{Action, Handle},
};

pub struct Network {
    tasks: Vec<JoinHandle<()>>,
    runtime: Option<runtime::Runtime>,
}

impl Network {
    pub fn start(drivers: &[Driver], handle: &Handle) -> Result<Option<Self>> {
        if !drivers.iter().any(|driver| driver.enabled) {
            return Ok(None);
        }
        let runtime = runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()?;
        let tasks = drivers
            .iter()
            .filter(|driver| driver.enabled)
            .map(|driver| {
                let driver = driver.clone();
                let handle = handle.clone();
                runtime.spawn(async move {
                    match &driver.protocol {
                        Protocol::ModbusServer { .. } => modbus::serve(&driver, handle).await,
                        Protocol::ModbusClient { .. } => modbus::connect(&driver, handle).await,
                        #[cfg(feature = "opcua")]
                        Protocol::OpcUaClient { .. } => opc::connect(&driver, handle).await,
                        #[cfg(not(feature = "opcua"))]
                        Protocol::OpcUaClient { .. } => {
                            unreachable!("validated during project load")
                        }
                    }
                })
            })
            .collect();
        Ok(Some(Self {
            tasks,
            runtime: Some(runtime),
        }))
    }
}

impl Drop for Network {
    fn drop(&mut self) {
        for task in &self.tasks {
            task.abort();
        }
        if let Some(runtime) = self.runtime.take() {
            runtime.shutdown_timeout(Duration::from_millis(200));
        }
    }
}

fn status(handle: &Handle, driver: &Driver, status: String, disconnected: bool) {
    if let Err(err) = handle.execute(Action::DriverStatus {
        driver: driver.id.clone(),
        status,
        disconnected,
    }) {
        log::debug!("{}: cannot publish driver state: {err}", driver.id);
    }
}

async fn retry(handle: &Handle, driver: &Driver, result: Result<()>) {
    let error = match result {
        Ok(()) => "disconnected".to_owned(),
        Err(error) => format!("{error:#}"),
    };
    status(handle, driver, error, true);
    time::sleep(Duration::from_millis(750)).await;
}

async fn io_timeout<T>(future: impl std::future::Future<Output = Result<T>>) -> Result<T> {
    time::timeout(Duration::from_secs(2), future)
        .await
        .context("request timed out")?
}
