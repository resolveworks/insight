# Chat

Insight uses a pure agentic approach—you ask questions in natural language, and the AI agent finds and synthesizes answers from your documents.

## How It Works

When you ask a question, the agent:

1. Analyzes your question to understand what you're looking for
2. Uses its tools to search and read relevant documents
3. Gathers evidence from multiple sources
4. Synthesizes an answer with citations

The agent handles all document retrieval internally. You don't need to know which documents contain the answer—just ask your question.

## Example Questions

**Factual queries:**

- "What was the total budget allocation for 2023?"
- "Who approved the permit application?"

**Summarization:**

- "Summarize the key findings from these reports"
- "What are the main arguments in this legal brief?"

**Analysis:**

- "Compare the environmental assessments from 2020 and 2023"
- "Find contradictions between these witness statements"

**Discovery:**

- "What topics are covered in this collection?"
- "Find all mentions of Company X"

## Citations

Every answer includes citations pointing to specific documents and passages. Click a citation to jump to the source document.

## Models

Insight uses local language models for all AI features. No data is sent to external servers.

### Available Models

| Model           | Size   | Notes                  |
| --------------- | ------ | ---------------------- |
| Qwen3 8B Q4_K_M | 5 GB   | Default, good balance  |
| Qwen3 4B Q4_K_M | 2.5 GB | Lightweight, faster    |
| Qwen3 8B Q8_0   | 8.5 GB | Higher quality, slower |

Models can be changed in settings. Larger models provide better answers but require more RAM and are slower.

## Behind the Scenes

The agent uses hybrid retrieval (full-text + semantic search via milli) to find relevant documents. Semantic search means the agent can find conceptually related content even without exact keyword matches—searching for "climate change impacts" will also surface documents discussing "global warming effects."
