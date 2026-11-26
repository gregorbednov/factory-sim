use std::{
    collections::BTreeMap,
    future::{Ready, ready},
    sync::{Arc, Mutex},
    time::Duration,
};

use anyhow::{Context as _, Result};
use tokio::{net::TcpListener, time};
use tokio_modbus::{
    prelude::*,
    server::{Service, tcp::Server},
};

use super::{io_timeout, retry, status};
use crate::{
    project::{Area, Direction, Driver, ModbusBinding, Protocol, Tag, Value, ValueType},
    runtime::{Action, Handle},
    sim::Snapshot,
};

#[derive(Clone)]
struct Mapped {
    tag: Tag,
    scale: f32,
}

fn from_register(entry: &Mapped, word: u16) -> Value {
    let signed = word as i16;
    match entry.tag.value_type {
        ValueType::Int => Value::Int(signed.into()),
        ValueType::Float => Value::Float(f32::from(signed) / entry.scale),
        ValueType::Bool => unreachable!("validated register binding"),
    }
}

fn to_register(entry: &Mapped, value: Value) -> u16 {
    match value {
        Value::Float(value) => ((value * entry.scale) as i16) as u16,
        Value::Int(value) => (value.clamp(i32::from(i16::MIN), i32::from(i16::MAX)) as i16) as u16,
        Value::Bool(_) => unreachable!("validated register binding"),
    }
}

fn bit(value: Value) -> bool {
    matches!(value, Value::Bool(true))
}

type Image = BTreeMap<(Area, u16), Mapped>;

fn make_image(snapshot: &Snapshot, bindings: &[ModbusBinding]) -> Image {
    bindings
        .iter()
        .map(|binding| {
            (
                (binding.area, binding.address),
                Mapped {
                    tag: snapshot.tags[&binding.tag].tag.clone(),
                    scale: binding.scale,
                },
            )
        })
        .collect()
}

struct Shared {
    image: Image,
    unit_id: u8,
    handle: Handle,
    connections: Mutex<usize>,
    config: Driver,
}

struct Connection(Arc<Shared>);

impl Connection {
    fn new(shared: Arc<Shared>) -> Self {
        {
            let mut count = shared.connections.lock().unwrap();
            *count += 1;
            if *count == 1 {
                status(&shared.handle, &shared.config, "connected".into(), false);
            }
        }
        Self(shared)
    }
}

impl Drop for Connection {
    fn drop(&mut self) {
        let mut count = self.0.connections.lock().unwrap();
        *count -= 1;
        if *count == 0 {
            status(
                &self.0.handle,
                &self.0.config,
                "listening · no controller".into(),
                true,
            );
        }
    }
}

impl Service for Connection {
    type Request = SlaveRequest<'static>;
    type Response = Response;
    type Exception = ExceptionCode;
    type Future = Ready<Result<Response, ExceptionCode>>;

    fn call(&self, req: Self::Request) -> Self::Future {
        ready(self.response(req))
    }
}

impl Connection {
    fn response(&self, request: SlaveRequest<'static>) -> Result<Response, ExceptionCode> {
        use ExceptionCode::{
            GatewayTargetDevice, IllegalDataAddress, IllegalDataValue, IllegalFunction,
            ServerDeviceBusy,
        };
        if request.slave != self.0.unit_id {
            return Err(GatewayTargetDevice);
        }
        let snapshot = self.0.handle.snapshot();
        let read = |area, address: u16, count: usize, limit: usize| {
            if count == 0 || count > limit {
                return Err(IllegalDataValue);
            }
            (0..count)
                .map(|offset| {
                    let address = address
                        .checked_add(offset as u16)
                        .ok_or(IllegalDataAddress)?;
                    self.0.image.get(&(area, address)).ok_or(IllegalDataAddress)
                })
                .collect::<Result<Vec<_>, _>>()
        };
        let bits = |area, address, count| {
            Ok(read(area, address, count, 2000)?
                .into_iter()
                .map(|mapped| bit(snapshot.tags[&mapped.tag.id].value))
                .collect())
        };
        let words = |area, address, count| {
            Ok(read(area, address, count, 125)?
                .into_iter()
                .map(|mapped| to_register(mapped, snapshot.tags[&mapped.tag.id].value))
                .collect())
        };
        let update = |area, address, count, limit, decode: &dyn Fn(usize, &Mapped) -> Value| {
            let points = read(area, address, count, limit)?;
            if points
                .iter()
                .any(|mapped| mapped.tag.direction != Direction::Actuator)
            {
                return Err(IllegalDataAddress);
            }
            let batch = points
                .iter()
                .enumerate()
                .map(|(i, mapped)| (mapped.tag.id.clone(), decode(i, mapped)))
                .collect();
            self.0
                .handle
                .execute(Action::Write(batch))
                .map_err(|_| ServerDeviceBusy)
        };
        Ok(match request.request {
            Request::ReadCoils(a, n) => Response::ReadCoils(bits(Area::Coil, a, n.into())?),
            Request::ReadDiscreteInputs(a, n) => {
                Response::ReadDiscreteInputs(bits(Area::DiscreteInput, a, n.into())?)
            }
            Request::ReadHoldingRegisters(a, n) => {
                Response::ReadHoldingRegisters(words(Area::HoldingRegister, a, n.into())?)
            }
            Request::ReadInputRegisters(a, n) => {
                Response::ReadInputRegisters(words(Area::InputRegister, a, n.into())?)
            }
            Request::WriteSingleCoil(a, v) => {
                update(Area::Coil, a, 1, 1, &|_, _| Value::Bool(v))?;
                Response::WriteSingleCoil(a, v)
            }
            Request::WriteSingleRegister(a, v) => {
                update(Area::HoldingRegister, a, 1, 1, &|_, m| from_register(m, v))?;
                Response::WriteSingleRegister(a, v)
            }
            Request::WriteMultipleCoils(a, v) => {
                update(Area::Coil, a, v.len(), 1968, &|i, _| Value::Bool(v[i]))?;
                Response::WriteMultipleCoils(a, v.len() as u16)
            }
            Request::WriteMultipleRegisters(a, v) => {
                update(Area::HoldingRegister, a, v.len(), 123, &|i, m| {
                    from_register(m, v[i])
                })?;
                Response::WriteMultipleRegisters(a, v.len() as u16)
            }
            _ => return Err(IllegalFunction),
        })
    }
}

pub async fn serve(driver: &Driver, handle: Handle) {
    let Protocol::ModbusServer {
        address,
        unit_id,
        bindings,
    } = &driver.protocol
    else {
        unreachable!()
    };
    let shared = Arc::new(Shared {
        image: make_image(&handle.snapshot(), bindings),
        unit_id: *unit_id,
        handle: handle.clone(),
        connections: Mutex::new(0),
        config: driver.clone(),
    });
    loop {
        let result = async {
            let listener = TcpListener::bind(address)
                .await
                .with_context(|| format!("bind {address}"))?;
            status(&handle, driver, "listening · no controller".into(), true);
            Server::new(listener)
                .serve(
                    &|stream, _addr| {
                        let service = Connection::new(shared.clone());
                        async move { Ok(Some((service, stream))) }
                    },
                    |error| log::debug!("Modbus client socket: {error}"),
                )
                .await?;
            Ok(())
        }
        .await;
        retry(&handle, driver, result).await;
    }
}

struct Group {
    area: Area,
    start: u16,
    direction: Direction,
    points: Vec<Mapped>,
}

fn groups(image: Image) -> Vec<Group> {
    let mut groups: Vec<Group> = Vec::new();
    for ((area, address), point) in image {
        let limit = if area.bit() { 1968 } else { 123 };
        if let Some(last) = groups.last_mut()
            && last.area == area
            && last.direction == point.tag.direction
            && usize::from(last.start) + last.points.len() == usize::from(address)
            && last.points.len() < limit
        {
            last.points.push(point);
        } else {
            groups.push(Group {
                area,
                start: address,
                direction: point.tag.direction,
                points: vec![point],
            });
        }
    }
    groups
}

pub async fn connect(driver: &Driver, handle: Handle) {
    let Protocol::ModbusClient {
        address,
        unit_id,
        bindings,
    } = &driver.protocol
    else {
        unreachable!()
    };
    let groups = groups(make_image(&handle.snapshot(), bindings));
    loop {
        let result = async {
            let mut client =
                io_timeout(async { Ok(tcp::connect_slave(*address, Slave(*unit_id)).await?) })
                    .await?;
            let mut timer = time::interval(Duration::from_millis(driver.poll_ms));
            timer.set_missed_tick_behavior(time::MissedTickBehavior::Skip);
            let mut published = false;
            loop {
                timer.tick().await;
                let snapshot = handle.snapshot();
                let batch = io_timeout(exchange(&mut client, &groups, &snapshot)).await?;
                if !batch.is_empty() {
                    handle.execute(Action::Write(batch))?;
                }
                if !published {
                    status(&handle, driver, "connected".into(), false);
                    published = true;
                }
            }
        }
        .await;
        retry(&handle, driver, result).await;
    }
}

async fn exchange(
    client: &mut tokio_modbus::client::Context,
    groups: &[Group],
    snapshot: &Snapshot,
) -> Result<Vec<(String, Value)>> {
    let mut batch = Vec::new();
    for group in groups {
        let n = group.points.len() as u16;
        let start = group.start;
        if group.direction == Direction::Actuator {
            if group.area.bit() {
                let values = match group.area {
                    Area::Coil => client.read_coils(start, n).await??,
                    _ => client.read_discrete_inputs(start, n).await??,
                };
                batch.extend(
                    group
                        .points
                        .iter()
                        .zip(values)
                        .map(|(m, v)| (m.tag.id.clone(), Value::Bool(v))),
                );
            } else {
                let values = match group.area {
                    Area::HoldingRegister => client.read_holding_registers(start, n).await??,
                    _ => client.read_input_registers(start, n).await??,
                };
                batch.extend(
                    group
                        .points
                        .iter()
                        .zip(values)
                        .map(|(m, v)| (m.tag.id.clone(), from_register(m, v))),
                );
            }
        } else if group.area.bit() {
            let values: Vec<_> = group
                .points
                .iter()
                .map(|m| bit(snapshot.tags[&m.tag.id].value))
                .collect();
            client.write_multiple_coils(start, &values).await??;
        } else {
            let values: Vec<_> = group
                .points
                .iter()
                .map(|m| to_register(m, snapshot.tags[&m.tag.id].value))
                .collect();
            client.write_multiple_registers(start, &values).await??;
        }
    }
    Ok(batch)
}
