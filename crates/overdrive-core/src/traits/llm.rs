//! [`Llm`] — sole source of LLM inference for the SRE-investigation agent.
//!
//! Production uses `RigLlm` (rig-rs over a real provider, with
//! `builtin:credential-proxy` holding the provider key). Simulation uses
//! `SimLlm` which replays a deterministic transcript — deviation in tool
//! choice or parameter shape fails the test.
//!
//! See `docs/whitepaper.md` §12 (*Native SRE Investigation Agent*) for how
//! the agent uses this boundary.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum LlmError {
    #[error("transcript mismatch at step {step}: expected {expected:?}, got {actual:?}")]
    TranscriptMismatch { step: usize, expected: String, actual: String },
    #[error("llm provider: {0}")]
    Provider(String),
}

/// A tool the LLM may invoke. Schemas are JSON-shaped; the call surface
/// is provider-neutral.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDef {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
    pub output_schema: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Prompt {
    pub system: String,
    pub messages: Vec<Message>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub content: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Role {
    User,
    Assistant,
    Tool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Completion {
    pub content: String,
    pub tool_calls: Vec<ToolCall>,
    pub usage: Usage,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub tool: String,
    pub input: serde_json::Value,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Usage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
}

#[async_trait]
pub trait Llm: Send + Sync + 'static {
    async fn complete(&self, prompt: &Prompt, tools: &[ToolDef]) -> Result<Completion, LlmError>;
}
