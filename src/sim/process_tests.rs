use super::*;
use serde_json::{Value as Json, json};

fn part(id: &str, kind: &str, position: [f32; 3], fields: Json) -> Json {
    let mut part = json!({
        "id": id, "name": id, "kind": kind, "position": position, "size": [0.3, 0.4, 0.3],
    });
    part.as_object_mut()
        .unwrap()
        .extend(fields.as_object().unwrap().clone());
    part
}

fn source(id: &str, position: [f32; 3], temperature_c: f32, dirt_mg_l: f32) -> Json {
    part(
        id,
        "water_source",
        position,
        json!({
            "temperature_c": temperature_c, "max_flow_l_s": 0.35, "dirt_mg_l": dirt_mg_l,
        }),
    )
}

fn sink(id: &str, position: [f32; 3]) -> Json {
    part(id, "water_drain", position, json!({"max_flow_l_s":0.3}))
}

fn process(parts: Vec<Json>, pipes: Json, commands: Json) -> Result<Simulation> {
    Simulation::new(serde_json::from_value(json!({
        "version":1, "name":"Process regression", "parts":parts, "pipes":pipes,
        "initial_commands":commands,
    }))?)
}

#[test]
fn junctions_conserve_water_and_mix_temperature_and_dirt() -> Result<()> {
    let mut simulation = process(
        vec![
            source("hot", [-2.0, 0.8, -0.4], 60.0, 100.0),
            source("cold", [-2.0, 0.8, 0.4], 10.0, 0.0),
            part(
                "mix",
                "water_junction",
                [-1.0, 0.3, 0.0],
                json!({"mixing": true}),
            ),
            part(
                "split",
                "water_junction",
                [0.0, 0.3, 0.0],
                json!({"mixing": false}),
            ),
            part(
                "valve",
                "water_valve",
                [1.0, 0.25, -0.4],
                json!({"max_flow_l_s": 0.3}),
            ),
            part(
                "meter",
                "water_instrument",
                [1.0, 0.25, 0.4],
                json!({"measurement":"flow"}),
            ),
            sink("low", [2.0, 0.1, -0.4]),
            part(
                "high",
                "water_drain",
                [2.0, 0.1, 0.4],
                json!({"max_flow_l_s": 0.6}),
            ),
        ],
        json!([
            {"from":"hot","to":"mix"},
            {"from":"cold","to":"mix","to_port":1},
            {"from":"mix","to":"split"},
            {"from":"split","to":"valve"},
            {"from":"valve","to":"low"},
            {"from":"split","from_port":1,"to":"meter"},
            {"from":"meter","to":"high"}
        ]),
        json!({"valve.opening_pct":100.0}),
    )?;
    simulation.step();
    let value = |sim: &Simulation, id, role| number(&sim.tags, id, role);
    let hot = value(&simulation, "hot", "flow_l_s");
    let cold = value(&simulation, "cold", "flow_l_s");
    let first = value(&simulation, "split", "outlet_1_l_s");
    let second = value(&simulation, "split", "outlet_2_l_s");
    assert!(hot > 0.0 && cold > 0.0 && first > 0.0 && second > 0.0);
    assert!((hot + cold - first - second).abs() < 1e-6);
    assert!(
        (value(&simulation, "meter", "liquid_temp_c") - (hot * 60.0 + cold * 10.0) / (hot + cold))
            .abs()
            < 1e-4
    );
    assert!((value(&simulation, "meter", "dirt_mg_l") - hot * 100.0 / (hot + cold)).abs() < 1e-4);
    simulation.command("valve.opening_pct", Value::Float(0.0))?;
    simulation.step();
    assert_eq!(value(&simulation, "split", "outlet_1_l_s"), 0.0);
    assert!(value(&simulation, "split", "outlet_2_l_s") > second);
    Ok(())
}

fn dirty_pump(filtered: bool) -> Result<Simulation> {
    let mut parts = vec![
        source("dirty", [-2.0, 0.8, 0.0], 20.0, 1000.0),
        part(
            "pump",
            "water_pump",
            [0.0, 0.15, 0.0],
            json!({"max_flow_l_s":0.3,"dirt_limit_mg":60.0}),
        ),
        sink("drain", [1.0, 0.1, 0.0]),
    ];
    let mut pipes = vec![json!({"from":"pump","to":"drain"})];
    if filtered {
        parts.push(part(
            "filter",
            "water_filter",
            [-1.0, 0.2, 0.0],
            json!({
                "clean_resistance":3,"dirty_resistance":25,"capacity_mg":3000,"efficiency_pct":100,
            }),
        ));
        pipes.extend([
            json!({"from":"dirty","to":"filter"}),
            json!({"from":"filter","to":"pump"}),
        ]);
    } else {
        pipes.push(json!({"from":"dirty","to":"pump"}));
    }
    process(parts, json!(pipes), json!({"pump.target_l_s":0.2}))
}

#[test]
fn dirt_trips_pump_repeatedly_and_a_filter_protects_it() -> Result<()> {
    let mut dirty = dirty_pump(false)?;
    let mut filtered = dirty_pump(true)?;
    for _ in 0..HZ {
        dirty.step();
        filtered.step();
    }
    assert!(flag(&dirty.tags, "pump", "failed"));
    assert_eq!(dirty.tags["pump.failures"].value, Value::Int(1));
    assert_eq!(number(&dirty.tags, "pump", "flow_l_s"), 0.0);
    assert!(!flag(&filtered.tags, "pump", "failed"));
    assert!(number(&filtered.tags, "filter", "loading_pct") > 0.0);
    assert!(number(&filtered.tags, "filter", "pressure_drop_kpa") > 0.0);
    assert_eq!(number(&filtered.tags, "pump", "dirt_mg_l"), 0.0);
    dirty.command("pump.reset_fault", Value::Bool(true))?;
    dirty.step();
    assert!(number(&dirty.tags, "pump", "flow_l_s") > 0.0);
    for _ in 0..HZ {
        dirty.step();
    }
    assert_eq!(dirty.tags["pump.failures"].value, Value::Int(2));
    assert!(flag(&dirty.tags, "pump", "failed"));

    filtered.command("filter.replace", Value::Bool(true))?;
    filtered.step();
    assert!(number(&filtered.tags, "filter", "loading_pct") < 0.1);
    Ok(())
}

#[test]
fn exchanger_preserves_heat_balance_and_filler_stops_at_dose() -> Result<()> {
    let mut simulation = process(
        vec![
            source("warm", [-2.0, 0.8, -0.3], 25.0, 20.0),
            source("cold", [-2.0, 0.8, 0.3], 10.0, 0.0),
            part(
                "heater",
                "water_heater",
                [-1.0, 0.3, -0.3],
                json!({"max_power_w": 800}),
            ),
            part(
                "hx",
                "water_heat_exchanger",
                [0.0, 0.3, 0.0],
                json!({"conductance_w_k": 100,"resistance": 5}),
            ),
            part(
                "filler",
                "water_filler",
                [1.0, 0.45, -0.3],
                json!({"max_flow_l_s":0.13,"dose_l":0.1}),
            ),
            sink("drain", [1.0, 0.1, 0.5]),
            part(
                "bottle",
                "crate",
                [1.0, 0.112, -0.3],
                json!({"product":"bottle-500ml","mass":0.045}),
            ),
        ],
        json!([
            {"from":"warm","to":"heater"},
            {"from":"heater","to":"hx"},
            {"from":"cold","to":"hx","to_port":1},
            {"from":"hx","to":"filler"},
            {"from":"hx","from_port":1,"to":"drain"}
        ]),
        json!({"heater.power_pct":100.0,"filler.fill":true}),
    )?;
    simulation.step();
    let read = |id, role| number(&simulation.tags, id, role);
    let q_hot = read("hx", "outlet_1_l_s");
    let q_cold = read("hx", "outlet_2_l_s");
    let heat = read("hx", "heat_w");
    assert!(q_hot > 0.0 && q_cold > 0.0 && heat > 0.0);
    let cooled = read("filler", "liquid_temp_c");
    let warmed = read("hx", "outlet_2_temperature_c");
    assert!(((read("heater", "liquid_temp_c") - cooled) * q_hot * 4180.0 - heat).abs() < 0.01);
    assert!(((warmed - 10.0) * q_cold * 4180.0 - heat).abs() < 0.01);

    for _ in 0..HZ * 2 {
        simulation.step();
    }
    let snapshot = simulation.snapshot();
    let bottle = snapshot.parcels.iter().find(|p| p.id == "bottle").unwrap();
    let contents = bottle.liquid.unwrap();
    assert!((contents.volume_l - 0.1).abs() < 1e-8);
    assert!((contents.dirt_mg_l - 20.0).abs() < 1e-6);
    assert_eq!(number(&simulation.tags, "filler", "flow_l_s"), 0.0);
    assert!(flag(&simulation.tags, "filler", "dose_done"));
    assert!(
        (simulation.world.colliders[simulation.parcels["bottle"].collider].mass() - 0.145).abs()
            < 1e-6
    );
    Ok(())
}

#[test]
fn parallel_legs_feed_a_capped_common_outlet() -> Result<()> {
    let mut sim = process(
        vec![
            source("supply", [-3.0, 0.45, 0.0], 20.0, 0.0),
            part(
                "pump",
                "water_pump",
                [-2.0, 0.16, 0.0],
                json!({"max_flow_l_s":0.35}),
            ),
            part(
                "tee",
                "water_junction",
                [-1.0, 0.25, 0.0],
                json!({"mixing":false}),
            ),
            part(
                "meter",
                "water_instrument",
                [0.0, 0.24, 0.8],
                json!({"measurement":"flow"}),
            ),
            part(
                "mix",
                "water_junction",
                [1.0, 0.25, 0.0],
                json!({"mixing":true}),
            ),
            part(
                "drain",
                "water_drain",
                [1.8, 0.20, 0.0],
                json!({"max_flow_l_s":0.13}),
            ),
        ],
        json!([
            {"from":"supply","to":"pump"},
            {"from":"pump","to":"tee"},
            {"from":"tee","to":"mix"},
            {"from":"tee","from_port":1,"to":"meter"},
            {"from":"meter","to":"mix","to_port":1},
            {"from":"mix","to":"drain"}
        ]),
        json!({"pump.target_l_s":0.25}),
    )?;
    sim.step();
    let first = number(&sim.tags, "tee", "outlet_1_l_s");
    let second = number(&sim.tags, "tee", "outlet_2_l_s");
    assert!(first > second && second > 0.0);
    assert!((first + second - 0.13).abs() < 1e-6);
    assert_eq!(number(&sim.tags, "drain", "flow_l_s"), 0.13);
    Ok(())
}
