use std::time::Duration;

use anyhow::{Result, bail, ensure};
use opcua::{
    client::{ClientBuilder, IdentityToken, Session},
    types::{
        AttributeId, DataValue, EndpointDescription, MessageSecurityMode, NodeId, ReadValueId,
        TimestampsToReturn, UserTokenPolicy, Variant, WriteValue,
    },
};
use tokio::time;

use super::{io_timeout, retry, status};
use crate::{
    project::{Direction, Driver, OpcSecurity, Protocol, Value},
    runtime::{Action, Handle},
    sim::Snapshot,
};

struct Mapping {
    reads: Vec<ReadValueId>,
    commands: Vec<String>,
    outputs: Vec<(String, NodeId)>,
}

pub async fn connect(driver: &Driver, handle: Handle) {
    let Protocol::OpcUaClient {
        endpoint,
        security,
        bindings,
    } = &driver.protocol
    else {
        unreachable!()
    };
    let snapshot = handle.snapshot();
    let mut mapping = Mapping {
        reads: Vec::new(),
        commands: Vec::new(),
        outputs: Vec::new(),
    };
    for binding in bindings {
        let node_id = binding.node.parse::<NodeId>().expect("validated NodeId");
        if snapshot.tags[&binding.tag].tag.direction == Direction::Actuator {
            mapping.reads.push(ReadValueId {
                node_id,
                attribute_id: AttributeId::Value as u32,
                ..Default::default()
            });
            mapping.commands.push(binding.tag.clone());
        } else {
            mapping.outputs.push((binding.tag.clone(), node_id));
        }
    }
    loop {
        let result = connected(driver, &handle, endpoint, security, &mapping).await;
        retry(&handle, driver, result).await;
    }
}

async fn connected(
    driver: &Driver,
    handle: &Handle,
    url: &str,
    security: &OpcSecurity,
    mapping: &Mapping,
) -> Result<()> {
    let builder = ClientBuilder::new()
        .application_name("Factory Sim")
        .application_uri("urn:factory-sim:opcua-client")
        .trust_server_certs(false)
        .session_retry_limit(0);
    let (builder, policy, mode) = match security {
        OpcSecurity::None => (builder, "None", MessageSecurityMode::None),
        OpcSecurity::SignAndEncrypt { pki_dir } => (
            builder.pki_dir(pki_dir).create_sample_keypair(true),
            "Basic256Sha256",
            MessageSecurityMode::SignAndEncrypt,
        ),
    };
    let mut client = builder
        .client()
        .map_err(|errors| anyhow::anyhow!("invalid OPC UA configuration: {}", errors.join(", ")))?;
    let endpoint: EndpointDescription = (url, policy, mode, UserTokenPolicy::anonymous()).into();
    let (session, event_loop) = io_timeout(async {
        client
            .connect_to_matching_endpoint(endpoint, IdentityToken::Anonymous)
            .await
            .map_err(|e| anyhow::anyhow!("OPC UA endpoint: {e}"))
    })
    .await?;
    let task = event_loop.spawn();
    let result = async {
        io_timeout(async {
            ensure!(session.wait_for_connection().await, "OPC UA session failed");
            Ok(())
        })
        .await?;
        let mut period = time::interval(Duration::from_millis(driver.poll_ms));
        period.set_missed_tick_behavior(time::MissedTickBehavior::Skip);
        let mut published = false;
        loop {
            period.tick().await;
            let snapshot = handle.snapshot();
            let batch = io_timeout(exchange(&session, mapping, &snapshot)).await?;
            if !batch.is_empty() {
                handle.execute(Action::Write(batch))?;
            }
            if !published {
                status(handle, driver, "connected".into(), false);
                published = true;
            }
        }
    }
    .await;
    task.abort();
    result
}

async fn exchange(
    session: &Session,
    mapping: &Mapping,
    snapshot: &Snapshot,
) -> Result<Vec<(String, Value)>> {
    let mut commands = Vec::new();
    if !mapping.reads.is_empty() {
        let values = session
            .read(&mapping.reads, TimestampsToReturn::Neither, 0.0)
            .await
            .map_err(|status| anyhow::anyhow!("OPC UA Read: {status}"))?;
        ensure!(
            values.len() == mapping.reads.len(),
            "OPC UA Read returned the wrong number of values"
        );
        for (id, value) in mapping.commands.iter().zip(values) {
            ensure!(
                value.status.is_none_or(|status| status.is_good()),
                "OPC UA node for {id}: {:?}",
                value.status
            );
            let value = match value.value {
                Some(Variant::Boolean(value)) => Value::Bool(value),
                Some(Variant::Float(value)) => Value::Float(value),
                Some(Variant::Int32(value)) => Value::Int(value),
                _ => bail!("OPC UA node for {id} must contain Boolean, Float, or Int32"),
            };
            commands.push((id.clone(), snapshot.tags[id].tag.check(value)?));
        }
    }
    if !mapping.outputs.is_empty() {
        let writes: Vec<_> = mapping
            .outputs
            .iter()
            .map(|(id, node_id)| {
                let value = match snapshot.tags[id].value {
                    Value::Bool(value) => Variant::Boolean(value),
                    Value::Float(value) => Variant::Float(value),
                    Value::Int(value) => Variant::Int32(value),
                };
                WriteValue {
                    node_id: node_id.clone(),
                    attribute_id: AttributeId::Value as u32,
                    index_range: Default::default(),
                    value: DataValue {
                        value: Some(value),
                        ..Default::default()
                    },
                }
            })
            .collect();
        let statuses = session
            .write(&writes)
            .await
            .map_err(|status| anyhow::anyhow!("OPC UA Write: {status}"))?;
        ensure!(
            statuses.len() == writes.len(),
            "OPC UA Write returned the wrong number of statuses"
        );
        for ((id, _), status) in mapping.outputs.iter().zip(statuses) {
            ensure!(status.is_good(), "OPC UA write for {id}: {status}");
        }
    }
    Ok(commands)
}
