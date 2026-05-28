# chat-1-message

## When to use
Use when a workflow consistently revolves around a single chat message or prompt response, especially for quick one-turn interactions, message formatting, or minimal-context chat handling.

## How to apply
- Treat the request as a one-message exchange unless the user asks for follow-up.
- Keep context compact and avoid introducing unnecessary multi-turn structure.
- Prefer direct, concise responses that solve the user’s immediate message.
- If the message is ambiguous, ask one clarifying question before expanding the workflow.

## Notes
- This skill is only useful when the pattern is repeated and stable across many chats.
- Keep the interaction lightweight; do not over-engineer multi-step state.
