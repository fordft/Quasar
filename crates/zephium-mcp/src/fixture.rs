//! A fixture server for tests: tools with and without annotations, an error
//! result, an oversized result and one text resource.
use std::sync::Arc;

use rmcp::model::*;
use rmcp::service::RequestContext;
use rmcp::{ErrorData, RoleServer, ServerHandler};
use serde_json::{json, Map, Value};

#[derive(Clone, Default)]
pub struct Fixture;

fn schema(value: Value) -> Arc<Map<String, Value>> {
    Arc::new(value.as_object().cloned().unwrap_or_default())
}

fn tools() -> Vec<Tool> {
    let mut echo = Tool::new(
        "echo",
        "Returns the text it was given.",
        schema(
            json!({"type": "object", "properties": {"text": {"type": "string"}}, "required": ["text"]}),
        ),
    );
    echo.annotations = Some(ToolAnnotations::from_raw(
        Some("Echo".into()),
        Some(true),
        None,
        None,
        Some(false),
    ));
    let add = Tool::new(
        "add",
        "Adds two numbers.",
        schema(
            json!({"type": "object", "properties": {"a": {"type": "number"}, "b": {"type": "number"}}, "required": ["a", "b"]}),
        ),
    );
    let mut send = Tool::new(
        "send_note",
        "Sends a note to the team.",
        schema(
            json!({"type": "object", "properties": {"to": {"type": "string"}, "body": {"type": "string"}}, "required": ["to", "body"]}),
        ),
    );
    send.annotations = Some(ToolAnnotations::from_raw(
        None,
        Some(false),
        Some(false),
        None,
        Some(true),
    ));
    let flood = Tool::new(
        "flood",
        "Returns a very large text.",
        schema(json!({"type": "object", "properties": {}})),
    );
    vec![echo, add, send, flood]
}

impl ServerHandler for Fixture {
    fn get_info(&self) -> ServerConfig {
        let mut config = ServerConfig::default();
        config.capabilities = ServerCapabilities::builder()
            .enable_tools()
            .enable_resources()
            .build();
        config.server_info = Implementation::new("fixture", "1.2.3");
        config.instructions = Some("Fixture server for Quasar tests.".into());
        config
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        Ok(ListToolsResult::with_all_items(tools()))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        let args = request.arguments.unwrap_or_default();
        let result = match &*request.name {
            "echo" => CallToolResult::success(vec![ContentBlock::text(
                args.get("text")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_owned(),
            )]),
            "add" => {
                let (Some(a), Some(b)) = (
                    args.get("a").and_then(Value::as_f64),
                    args.get("b").and_then(Value::as_f64),
                ) else {
                    return Ok(CallToolResult::error(vec![ContentBlock::text(
                        "a and b are numbers",
                    )])
                    .into());
                };
                CallToolResult::success(vec![ContentBlock::text(format!("{}", a + b))])
            }
            "send_note" => CallToolResult::success(vec![ContentBlock::text("sent")]),
            "flood" => CallToolResult::success(vec![ContentBlock::text("x".repeat(200_000))]),
            _ => return Err(ErrorData::invalid_params("unknown tool", None)),
        };
        Ok(result.into())
    }

    async fn list_resources(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListResourcesResult, ErrorData> {
        Ok(ListResourcesResult::with_all_items(vec![Resource::new(
            "memo://readme",
            "Read me",
        )]))
    }

    async fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResponse, ErrorData> {
        if request.uri != "memo://readme" {
            return Err(ErrorData::resource_not_found("no such resource", None));
        }
        Ok(ReadResourceResult::new(vec![ResourceContents::text(
            "Hello from the fixture.",
            "memo://readme",
        )])
        .into())
    }
}

/// Serves the fixture on stdin and stdout until the client leaves.
#[cfg(feature = "fixture")]
pub async fn serve_stdio() {
    use rmcp::ServiceExt;
    if let Ok(service) = Fixture.serve(rmcp::transport::stdio()).await {
        let _ = service.waiting().await;
    }
}
