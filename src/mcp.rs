use anyhow::Result;
use rmcp::{
    ErrorData, ServerHandler, ServiceExt,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{CallToolResult, ServerCapabilities, ServerConfig},
    tool, tool_handler, tool_router,
};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value as JsonValue, json};

use crate::{
    project::{Project, Value},
    runtime::{Action, Handle, Session},
};

struct Server {
    handle: Handle,
    tool_router: ToolRouter<Self>,
}

#[derive(Deserialize, JsonSchema)]
struct TagId {
    id: String,
}

#[derive(Deserialize, JsonSchema)]
struct TagWrite {
    id: String,
    /// A Boolean or an integer or floating-point number matching the actuator type.
    value: JsonValue,
}

type Reply = Result<CallToolResult, ErrorData>;

#[tool_router]
impl Server {
    fn new(handle: Handle) -> Self {
        Self {
            handle,
            tool_router: Self::tool_router(),
        }
    }

    #[tool(description = "Read simulation state, positions, driver state, and the current tags")]
    fn state(&self) -> Reply {
        encode(&*self.handle.snapshot())
    }

    #[tool(description = "List project tags, their roles, types, quality, and current values")]
    fn list_tags(&self) -> Reply {
        encode(&self.handle.snapshot().tags)
    }

    #[tool(description = "Read the tag with the given stable ID")]
    fn read_tag(&self, Parameters(args): Parameters<TagId>) -> Reply {
        let snapshot = self.handle.snapshot();
        let tag = snapshot
            .tags
            .get(&args.id)
            .ok_or_else(|| ErrorData::invalid_params(format!("unknown tag {}", args.id), None))?;
        encode(tag)
    }

    #[tool(description = "Set an actuator tag with a Boolean or a typed number")]
    fn set_actuator(&self, Parameters(args): Parameters<TagWrite>) -> Reply {
        let value = serde_json::from_value::<Value>(args.value)
            .map_err(|err| ErrorData::invalid_params(err.to_string(), None))?;
        self.apply(Action::Write(vec![(args.id, value)]))
    }

    #[tool(description = "Start real-time simulation")]
    fn start(&self) -> Reply {
        self.apply(Action::Run)
    }

    #[tool(description = "Pause physical time")]
    fn pause(&self) -> Reply {
        self.apply(Action::Pause)
    }

    #[tool(description = "Reset project bodies and commands, and pause physical time")]
    fn reset(&self) -> Reply {
        self.apply(Action::Reset)
    }

    fn apply(&self, action: Action) -> Reply {
        self.handle
            .execute(action)
            .map_err(|err| ErrorData::invalid_params(err.to_string(), None))?;
        let snapshot = self.handle.snapshot();
        Ok(CallToolResult::structured(
            json!({ "running": snapshot.running, "tick": snapshot.tick }),
        ))
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for Server {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build()).with_instructions(
            "Controls a local virtual factory. Use stable IDs returned by list_tags.",
        )
    }
}

fn encode<T: serde::Serialize>(value: &T) -> Reply {
    serde_json::to_value(value)
        .map(CallToolResult::structured)
        .map_err(|err| ErrorData::internal_error(err.to_string(), None))
}

pub fn run(project: Project, io: bool) -> Result<()> {
    let session = Session::new(project, io)?;
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    rt.block_on(async {
        let server = Server::new(session.runtime.handle.clone())
            .serve(rmcp::transport::stdio())
            .await?;
        server.waiting().await?;
        Ok(())
    })
}
