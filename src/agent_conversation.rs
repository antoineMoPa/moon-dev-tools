use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AgentState {
    Working,
    WaitingForInput,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub(crate) enum AgentMessageRole {
    System,
    User,
    Assistant,
    Tool,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub(crate) struct AgentMessage {
    pub(crate) role: AgentMessageRole,
    pub(crate) content: Option<String>,
    #[serde(default)]
    pub(crate) tool_calls: Vec<AgentToolCall>,
    #[serde(default)]
    pub(crate) tool_call_id: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub(crate) struct AgentToolCall {
    pub(crate) id: String,
    pub(crate) function: AgentToolFunction,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub(crate) struct AgentToolFunction {
    pub(crate) name: String,
    /// The arguments as the model wrote them: JSON, as a string.
    pub(crate) arguments: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub(crate) struct AgentPrompt {
    pub(crate) text: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct AgentConversation {
    /// Minerve's own session id, which changes when `/resume` switches session.
    pub(crate) session_id: Option<String>,
    pub(crate) state: Option<AgentState>,
    pub(crate) messages: Vec<AgentMessage>,
    pub(crate) broken: Option<String>,
}
