//! MimiEyes (VENDORED.md #3): the `panes` tool — an agent reads and lays
//! out its project's view tabs and panes. The engine relays the request to
//! the open MimiEyes window (`HzRequest`), which owns that state and answers.

use serde_json::{Value, json};

use crate::ToolDef;

pub const PANES: &str = "panes";

/// The engine's relay method (`zeron_engine::hz_relay::HZ_REQUEST`).
const HZ_REQUEST: &str = "HzRequest";

pub const PANES_DESCRIPTION: &str = "\
MimiEyes view tabs and panes of YOUR project (the project of the chat you are in). The user sees \
each session as a pane inside a named view tab; use this to put your team on screen.\n\
\n\
op \"list\": the project's tabs (id, name, active) with their panes (chat id, short id, title, \
focused) and each tab's layout tree.\n\
\n\
op \"arrange\": lay out one tab.\n\
- tab: \"current\" (the tab your own pane is in; default), a tab id or name, or \"new\" to create one.\n\
- name: the tab's name (required with \"new\"; renames otherwise).\n\
- layout: a tree. A pane is {\"chat\": \"self\" | chat id | unique id prefix | exact title}. A split is \
{\"split\": \"right\" (a | b side by side) or \"down\" (a over b), \"ratio\": share of a, 0.1–0.9 \
(default: even), \"a\": node, \"b\": node}. Omit layout to only rename.\n\
- focus: true switches the user's screen to this tab (default: only if it is already on screen).\n\
A session moves out of any other tab it was in; panes of the tab left out of the layout leave it \
(their sessions stay in the sidebar). Only sessions of this project can be placed. Create teammates \
first (create_chat / create_chats, with a title so the user sees a name), then arrange them.\n\
\n\
Example — you and the researcher stacked on the left, the worker on the whole right:\n\
{\"op\": \"arrange\", \"layout\": {\"split\": \"right\", \"a\": {\"split\": \"down\", \"a\": {\"chat\": \"self\"}, \
\"b\": {\"chat\": \"Researcher\"}}, \"b\": {\"chat\": \"Worker\"}}}";

pub fn tool() -> ToolDef {
    ToolDef {
        name: PANES,
        description: PANES_DESCRIPTION,
        input_schema: json!({
            "type": "object",
            "properties": {
                "op": { "type": "string", "enum": ["list", "arrange"] },
                "tab": { "type": "string", "description": "\"current\" (default), a tab id or name, or \"new\"." },
                "name": { "type": "string", "description": "Tab name (required with tab \"new\")." },
                "layout": { "type": "object", "description": "Pane tree: {\"chat\": ...} or {\"split\": \"right\"|\"down\", \"ratio\"?: number, \"a\": node, \"b\": node}." },
                "focus": { "type": "boolean", "description": "Switch the user's screen to this tab." }
            },
            "required": ["op"]
        }),
    }
}

/// Ask the window, on behalf of chat `from`.
pub async fn call(zeron: &crate::zeron::Zeron, from: Option<&str>, args: Value) -> anyhow::Result<Value> {
    let from = from.ok_or_else(|| {
        anyhow::anyhow!("panes only works from an agent MimiEyes launched in a chat")
    })?;
    zeron
        .call(HZ_REQUEST, json!({ "from": from, "request": args }))
        .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_schema_names_both_operations() {
        let tool = tool();
        assert_eq!(tool.name, "panes");
        let ops = &tool.input_schema["properties"]["op"]["enum"];
        assert_eq!(ops, &json!(["list", "arrange"]));
        assert!(PANES_DESCRIPTION.contains("\"split\": \"right\""));
    }
}
