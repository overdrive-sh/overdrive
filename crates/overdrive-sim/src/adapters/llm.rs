//! `SimLlm` — transcript-replay implementation of the [`Llm`] port.
//!
//! DST needs LLM completions to be bit-reproducible. `SimLlm` holds a
//! `Vec<Completion>` captured from a prior run; each call to
//! `complete` pops the next entry and returns it. When the transcript
//! is exhausted or a tool-choice deviation is detected, `complete`
//! returns [`LlmError::TranscriptMismatch`] — the investigation agent
//! treats this as a test failure rather than silently hallucinating
//! novel output.
//!
//! Phase-1 transcripts are empty (the investigation agent does not
//! exist yet); the type exists so that every non-determinism port has
//! a sim implementation ready for Phase-2 wiring.

use async_trait::async_trait;
use parking_lot::Mutex;

use overdrive_core::traits::llm::{Completion, Llm, LlmError, Prompt, ToolDef};

/// A prepared transcript — a sequence of completions the agent is
/// expected to produce in order.
pub struct SimLlm {
    transcript: Mutex<Vec<Completion>>,
    cursor: Mutex<usize>,
}

impl SimLlm {
    /// Construct a sim LLM primed with the given transcript. Calls to
    /// `complete` consume entries in order.
    #[must_use]
    pub const fn new(transcript: Vec<Completion>) -> Self {
        Self { transcript: Mutex::new(transcript), cursor: Mutex::new(0) }
    }

    /// True when every queued completion has been consumed.
    #[must_use]
    pub fn is_exhausted(&self) -> bool {
        let cursor = *self.cursor.lock();
        let len = self.transcript.lock().len();
        cursor >= len
    }
}

#[async_trait]
impl Llm for SimLlm {
    async fn complete(&self, _prompt: &Prompt, _tools: &[ToolDef]) -> Result<Completion, LlmError> {
        let transcript = self.transcript.lock().clone();
        let index;
        let next;
        {
            let mut cursor = self.cursor.lock();
            index = *cursor;
            next = transcript.get(index).cloned();
            if next.is_some() {
                *cursor = index + 1;
            }
        }
        next.ok_or_else(|| LlmError::TranscriptMismatch {
            step: index,
            expected: "<transcript exhausted>".to_owned(),
            actual: "<caller made another request>".to_owned(),
        })
    }
}
