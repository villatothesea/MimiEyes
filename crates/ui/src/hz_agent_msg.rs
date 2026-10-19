//! MimiEyes (VENDORED.md #3): messages one agent sends another.
//!
//! Zeron MCP's `send_message` prefixes the text with a line naming the
//! sender for the receiving agent:
//!
//! `[Message from Zeron chat Planner (a1b2c3d4). Reply to it with the Zeron
//! \`send_message\` tool, chat <full id>.]` + blank line + the message.
//!
//! The agent needs that line; a person reading the pane does not. The
//! transcript shows the sender as a label instead (the raw text is unchanged).

/// The sender of an agent-sent message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentSender {
    /// The sending chat's title, when it has one.
    pub title: Option<String>,
    /// Its short id (the first characters of the chat id).
    pub short_id: String,
}

impl AgentSender {
    /// "From Planner · a1b2c3d4" (or "From a1b2c3d4").
    pub fn label(&self) -> String {
        match &self.title {
            Some(title) => format!("From {title} · {}", self.short_id),
            None => format!("From {}", self.short_id),
        }
    }
}

const PREFIX: &str = "[Message from Zeron chat ";
const REPLY: &str = ". Reply to it with the Zeron `send_message` tool, chat ";

/// Split an agent-sent message into its sender and the message itself;
/// `None` for anything else.
pub fn split(text: &str) -> Option<(AgentSender, &str)> {
    let rest = text.strip_prefix(PREFIX)?;
    let reply_at = rest.find(REPLY)?;
    let label = &rest[..reply_at];
    let after = &rest[reply_at + REPLY.len()..];
    let end = after.find(".]")?;
    let body = after[end + 2..].trim_start_matches(['\r', '\n']);
    let (title, short_id) = match label.rfind(" (") {
        Some(open) if label.ends_with(')') => (
            Some(label[..open].trim().to_owned()).filter(|t| !t.is_empty()),
            label[open + 2..label.len() - 1].trim().to_owned(),
        ),
        _ => (None, label.trim().to_owned()),
    };
    if short_id.is_empty() {
        return None;
    }
    Some((AgentSender { title, short_id }, body))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_titled_sender_splits_off() {
        let text = "[Message from Zeron chat Planner (a1b2c3d4). Reply to it with the Zeron `send_message` tool, chat a1b2c3d4-0000-0000.]\n\nWrite the parser.";
        let (from, body) = split(text).unwrap();
        assert_eq!(from.title.as_deref(), Some("Planner"));
        assert_eq!(from.short_id, "a1b2c3d4");
        assert_eq!(from.label(), "From Planner · a1b2c3d4");
        assert_eq!(body, "Write the parser.");
    }

    #[test]
    fn an_untitled_sender_shows_its_id() {
        let text = "[Message from Zeron chat 9f8e7d6c. Reply to it with the Zeron `send_message` tool, chat 9f8e7d6c-1.]\n\nhi";
        let (from, body) = split(text).unwrap();
        assert_eq!(from.title, None);
        assert_eq!(from.label(), "From 9f8e7d6c");
        assert_eq!(body, "hi");
    }

    #[test]
    fn titles_with_parentheses_keep_them() {
        let text = "[Message from Zeron chat Fix (part 2) (abcd1234). Reply to it with the Zeron `send_message` tool, chat abcd1234-9.]\n\nok";
        let (from, _) = split(text).unwrap();
        assert_eq!(from.title.as_deref(), Some("Fix (part 2)"));
        assert_eq!(from.short_id, "abcd1234");
    }

    #[test]
    fn ordinary_messages_are_left_alone() {
        assert!(split("Write the parser.").is_none());
        assert!(split("[Message from Zeron chat broken").is_none());
    }
}
