//! Regression tests for multi-tenant system prompts.
//!
//! The agent must build the conversational system prompt from a workspace
//! scoped to the incoming message's user, not from the shared owner-scope
//! workspace created at startup. Otherwise per-user prompt documents
//! (SOUL.md, AGENTS.md, TOOLS.md) become invisible and different users can
//! see the same owner-scoped prompt.
//!
//! These tests:
//! 1. Seed identity files for two users (alice, bob) in the database
//! 2. Send messages as each user
//! 3. Verify the system prompt in captured LLM requests contains the
//!    correct user's identity
//! 4. Verify user A's identity doesn't leak into user B's prompt
//!
//! These tests ensure each user's prompt context is isolated correctly.

#[cfg(feature = "libsql")]
mod support;

#[cfg(feature = "libsql")]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use steward_core::channels::IncomingMessage;
    use steward_core::llm::Role;
    use steward_core::workspace::Workspace;

    use crate::support::test_rig::TestRigBuilder;
    use crate::support::trace_llm::{LlmTrace, TraceResponse, TraceStep};

    const TIMEOUT: Duration = Duration::from_secs(15);

    const ALICE_USER_ID: &str = "alice";
    const BOB_USER_ID: &str = "bob";

    const ALICE_SOUL: &str = "Alice values careful engineering and lives in Seattle.";
    const BOB_SOUL: &str = "Bob values ocean science and lives in Miami.";

    /// Create a simple trace that returns a canned text response.
    /// We need one step per message we plan to send.
    fn simple_trace(num_steps: usize) -> LlmTrace {
        let steps: Vec<TraceStep> = (0..num_steps)
            .map(|i| TraceStep {
                request_hint: None,
                response: TraceResponse::Text {
                    content: format!("Response {}", i),
                    input_tokens: 100,
                    output_tokens: 10,
                },
                expected_tool_results: Vec::new(),
            })
            .collect();

        // Create separate turns for each step so the trace replays correctly.
        let turns: Vec<crate::support::trace_llm::TraceTurn> = steps
            .into_iter()
            .enumerate()
            .map(|(i, step)| crate::support::trace_llm::TraceTurn {
                user_input: format!("message {}", i),
                steps: vec![step],
                expects: Default::default(),
            })
            .collect();

        LlmTrace::new("test-model", turns)
    }

    /// Seed prompt-bearing files for a user by creating a workspace scoped to that
    /// user and writing SOUL.md.
    async fn seed_prompt_context(
        db: &Arc<dyn steward_core::db::Database>,
        user_id: &str,
        content: &str,
    ) {
        let ws = Workspace::new_with_db(user_id, db.clone());
        ws.write("SOUL.md", content)
            .await
            .unwrap_or_else(|e| panic!("Failed to seed SOUL.md for {user_id}: {e}"));
    }

    /// Extract the system prompt from captured LLM requests.
    ///
    /// The system prompt is the first message with role=System in the first
    /// LLM request for a given turn.
    fn extract_system_prompt(requests: &[Vec<steward_core::llm::ChatMessage>]) -> Option<String> {
        requests.last().and_then(|msgs| {
            msgs.iter()
                .find(|m| matches!(m.role, Role::System))
                .map(|m| m.content.clone())
        })
    }

    // -----------------------------------------------------------------------
    // Test 1: Alice's identity should appear in system prompt when messaging
    // as Alice.
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn alice_system_prompt_contains_alice_prompt_context() {
        let trace = simple_trace(1);
        let rig = TestRigBuilder::new().with_trace(trace).build().await;

        // Seed alice's prompt context into the database
        let db = rig.database();
        seed_prompt_context(db, ALICE_USER_ID, ALICE_SOUL).await;

        // Send a message AS alice (using her user_id)
        let msg = IncomingMessage::new("test", ALICE_USER_ID, "Hello, who am I?");
        rig.send_incoming(msg).await;
        let _responses = rig.wait_for_responses(1, TIMEOUT).await;

        // The system prompt sent to the LLM should contain Alice's prompt context
        let requests = rig.captured_llm_requests();
        let system_prompt =
            extract_system_prompt(&requests).expect("Expected a system prompt in the LLM request");

        assert!(
            system_prompt.contains("careful engineering"),
            "System prompt should contain Alice's prompt context when messaging as Alice.\n\
             Actual system prompt:\n{system_prompt}"
        );

        rig.shutdown();
    }

    // -----------------------------------------------------------------------
    // Test 2: Bob's identity should appear in system prompt when messaging
    // as Bob.
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn bob_system_prompt_contains_bob_prompt_context() {
        let trace = simple_trace(1);
        let rig = TestRigBuilder::new().with_trace(trace).build().await;

        // Seed bob's prompt context into the database
        let db = rig.database();
        seed_prompt_context(db, BOB_USER_ID, BOB_SOUL).await;

        // Send a message AS bob
        let msg = IncomingMessage::new("test", BOB_USER_ID, "Hello, who am I?");
        rig.send_incoming(msg).await;
        let _responses = rig.wait_for_responses(1, TIMEOUT).await;

        // The system prompt should contain Bob's prompt context
        let requests = rig.captured_llm_requests();
        let system_prompt =
            extract_system_prompt(&requests).expect("Expected a system prompt in the LLM request");

        assert!(
            system_prompt.contains("ocean science"),
            "System prompt should contain Bob's prompt context when messaging as Bob.\n\
             Actual system prompt:\n{system_prompt}"
        );

        rig.shutdown();
    }

    // -----------------------------------------------------------------------
    // Test 3: Alice's identity must NOT appear in Bob's system prompt.
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn alice_prompt_context_does_not_leak_into_bob_prompt() {
        let trace = simple_trace(1);
        let rig = TestRigBuilder::new().with_trace(trace).build().await;

        // Seed BOTH users' prompt context
        let db = rig.database();
        seed_prompt_context(db, ALICE_USER_ID, ALICE_SOUL).await;
        seed_prompt_context(db, BOB_USER_ID, BOB_SOUL).await;

        // Send a message AS bob
        let msg = IncomingMessage::new("test", BOB_USER_ID, "Tell me about myself");
        rig.send_incoming(msg).await;
        let _responses = rig.wait_for_responses(1, TIMEOUT).await;

        // Bob's prompt must NOT contain Alice's identity
        let requests = rig.captured_llm_requests();
        let system_prompt = extract_system_prompt(&requests);

        if let Some(ref prompt) = system_prompt {
            assert!(
                !prompt.contains("careful engineering"),
                "Alice's prompt context LEAKED into Bob's system prompt!\n\
                 System prompt:\n{prompt}"
            );
        }
        // Also verify Bob's prompt context IS present (compound check)
        let prompt = system_prompt.expect("Expected a system prompt in the LLM request");
        assert!(
            prompt.contains("ocean science"),
            "Bob's own prompt context should be in his system prompt.\n\
             Actual system prompt:\n{prompt}"
        );

        rig.shutdown();
    }

    // -----------------------------------------------------------------------
    // Test 4: Bob's identity must NOT appear in Alice's system prompt.
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn bob_prompt_context_does_not_leak_into_alice_prompt() {
        let trace = simple_trace(1);
        let rig = TestRigBuilder::new().with_trace(trace).build().await;

        // Seed BOTH users' prompt context
        let db = rig.database();
        seed_prompt_context(db, ALICE_USER_ID, ALICE_SOUL).await;
        seed_prompt_context(db, BOB_USER_ID, BOB_SOUL).await;

        // Send a message AS alice
        let msg = IncomingMessage::new("test", ALICE_USER_ID, "Tell me about myself");
        rig.send_incoming(msg).await;
        let _responses = rig.wait_for_responses(1, TIMEOUT).await;

        // Alice's prompt must NOT contain Bob's identity
        let requests = rig.captured_llm_requests();
        let system_prompt = extract_system_prompt(&requests);

        if let Some(ref prompt) = system_prompt {
            assert!(
                !prompt.contains("ocean science"),
                "Bob's prompt context LEAKED into Alice's system prompt!\n\
                 System prompt:\n{prompt}"
            );
        }
        // Also verify Alice's prompt context IS present
        let prompt = system_prompt.expect("Expected a system prompt in the LLM request");
        assert!(
            prompt.contains("careful engineering"),
            "Alice's own prompt context should be in her system prompt.\n\
             Actual system prompt:\n{prompt}"
        );

        rig.shutdown();
    }

    #[tokio::test]
    async fn desktop_system_prompt_uses_normal_reply_guidance() {
        let trace = simple_trace(1);
        let rig = TestRigBuilder::new().with_trace(trace).build().await;

        let msg = IncomingMessage::new("desktop", "desktop-user", "Hello there");
        rig.send_incoming(msg).await;
        let _responses = rig.wait_for_responses(1, TIMEOUT).await;

        let requests = rig.captured_llm_requests();
        let system_prompt =
            extract_system_prompt(&requests).expect("Expected a system prompt in the LLM request");

        let has_reply_guidance =
            system_prompt.contains("Use normal assistant output to reply here");
        let has_desktop_formatting = system_prompt.contains("## Channel Formatting (desktop)")
            && system_prompt.contains("No markdown tables");
        assert!(
            has_reply_guidance || has_desktop_formatting,
            "System prompt should preserve desktop reply/channel guidance.\n\
             Actual system prompt:\n{system_prompt}"
        );
        assert!(
            !system_prompt.contains("`message` tool"),
            "System prompt should not mention removed outbound-channel tooling.\n\
             Actual system prompt:\n{system_prompt}"
        );

        rig.shutdown();
    }
}
