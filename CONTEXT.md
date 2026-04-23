# Agentic File System

An experimental file system where explicitly managed directories host agents that observe local file activity and collaborate under a daemon supervisor.

## Language

**Managed Directory**:
A directory where a **Directory Agent** has been explicitly installed.
_Avoid_: Agent directory, agent folder, watched folder

**Directory Agent**:
An agent installed in a **Managed Directory** that can answer questions about and mutate its **Managed Subtree**.
_Avoid_: Folder agent, local agent

**Managed Subtree**:
The filesystem entries a **Directory Agent** is responsible for under its **Managed Directory**, excluding nested **Managed Directories** and already-supervised symlink targets.
_Avoid_: Watched subtree, indexed subtree

**File Reference**:
A clickable response reference from the **Supervisor Agent** to a file in a **Managed Subtree**.
_Avoid_: File link, file pointer, source link

**Directory History**:
The reversible record of changes made within a **Managed Directory**.
_Avoid_: Folder history, undo log

**History Backend**:
The storage mechanism inside an **Agent Home** that records **Directory History**.
_Avoid_: Project git, repository

**Directory Index**:
The local searchable record of paths, metadata, content digests, and lightweight summaries for a **Managed Subtree**.
_Avoid_: Search database, knowledge base

**History Baseline**:
The initial **History Entry** that records the starting state of a **Managed Directory** during installation.
_Avoid_: Initial commit, snapshot

**Index Status**:
The build state of a **Directory Index**.
_Avoid_: Scan status, indexing progress

**Filesystem Event**:
An operating-system notification about a file or directory change inside a **Managed Subtree**.
_Avoid_: Poll result, scan event

**Settled Change**:
A normalized file change produced after related **Filesystem Events** for a path stop arriving.
_Avoid_: Event batch, debounce

**Startup Reconciliation**:
The restart-time comparison of a **Managed Subtree** against **Directory History** and **Directory Index** to catch changes missed while the **Directory Agent** was stopped.
_Avoid_: Polling fallback, periodic scan

**Linux-Only Experiment**:
The initial AFS platform scope, limited to Linux because monitoring depends on inotify-style filesystem events.
_Avoid_: Cross-platform release, portable mode

**AFS Ignore Policy**:
The per-**Managed Directory** rules that exclude filesystem entries from monitoring, indexing, and answers.
_Avoid_: Ignore file, gitignore

**History Entry**:
A reversible record in **Directory History**.
_Avoid_: Revision, commit

**Agent Change**:
An atomic reversible change set made by a **Directory Agent** while handling one request or task.
_Avoid_: Edit, patch, revision

**External Change**:
A file change observed in a **Managed Subtree** that was not made by a **Directory Agent** while handling a **Task Request**.
_Avoid_: Manual edit, outside edit

**Change Conflict**:
A failed attempt to record a **History Entry** because a relevant file changed after the work began.
_Avoid_: Merge conflict, race

**Task Request**:
A deliberate request from the **Supervisor Agent** or a **Directory Agent** that asks a **Directory Agent** to perform work.
_Avoid_: Job, command

**Task Queue**:
The ordered backlog of waiting **Task Requests** for a **Directory Agent**.
_Avoid_: Inbox, pending jobs

**Agent Command**:
A shell command run by a **Directory Agent** while handling a **Task Request**.
_Avoid_: Tool call, script execution

**Reply Target**:
The recipient that a **Task Request** says should receive the response.
_Avoid_: Callback, response sink

**Broadcast Request**:
A request sent to all registered **Directory Agents**.
_Avoid_: Fan-out, global message

**Agent Conversation**:
A related exchange of **Agent Messages** that may include direct messages and **Broadcast Requests** while handling a user prompt or delegated task.
_Avoid_: Discovery round, workflow, protocol

**Change Report**:
A reply section that states whether files were modified and lists the resulting **History Entries** and affected files.
_Avoid_: Mutation report, edit summary

**Relevance Reply**:
A **Broadcast Request** reply that states whether the replying **Directory Agent** has `none`, `possible`, or `strong` relevance, with a short reason.
_Avoid_: Score, rank

**Conversation Participant**:
A **Directory Agent** that sent or received an **Agent Message** in an **Agent Conversation**.
_Avoid_: Involved agent, collaborator

**Registered Agent**:
A **Directory Agent** known to the **Supervisor Agent** and eligible to receive routed requests.
_Avoid_: Active agent, live agent

**Agent Registry**:
The **Supervisor Agent**'s record of **Registered Agents**, their **Managed Directories**, endpoints, and ownership metadata.
_Avoid_: Agent list, routing table

**Agent Home**:
The `.afs/` control directory inside a **Managed Directory** that stores the **Directory Agent**'s durable configuration, identity metadata, and local history backend.
_Avoid_: Control folder, metadata directory

**Agent Identity**:
The stable identity stored in an **Agent Home** and used to identify a **Directory Agent** across restarts and messages.
_Avoid_: Agent name, directory name

**Agent Instructions**:
The local behavior guidance stored in an **Agent Home** for a **Directory Agent**.
_Avoid_: Personality, policy

**Pi Agent Runtime**:
The external Pi coding-agent harness used as the base runtime for AFS agents.
_Avoid_: Pi agent, base agent

**Agent Process**:
A long-running process for a **Directory Agent** or the **Supervisor Agent**.
_Avoid_: Agent invocation, worker job

**Agent Message**:
A request or response exchanged between **Agent Processes**.
_Avoid_: IPC packet, chat message

**Agent Transport**:
The local communication mechanism used to exchange **Agent Messages** between **Agent Processes**.
_Avoid_: Bus, protocol

**Ownership Split**:
The transition where a new nested **Managed Directory** is installed and its subtree is removed from the parent **Directory Agent**'s **Managed Subtree**.
_Avoid_: Detach, carve-out

**Ownership Merge**:
The transition where a nested **Managed Directory** is removed and its former subtree returns to the parent **Directory Agent**'s **Managed Subtree**.
_Avoid_: Reattach, takeover

**Agent Removal**:
The **Supervisor Agent**-mediated removal of a **Directory Agent** from a **Managed Directory**.
_Avoid_: Delete agent, disable agent

**Agent Home Archive**:
A timestamped copy of a removed **Agent Home** preserved after **Agent Removal**.
_Avoid_: Tombstone, backup

**Cross-Directory Task**:
A user request coordinated by the **Supervisor Agent** that involves multiple **Directory Agents**.
_Avoid_: Coordinated transaction, global change

**Supervisor Agent**:
The always-running daemon agent that receives user prompts and coordinates work across **Directory Agents**.
_Avoid_: Root agent, main agent

**Supervisor Home**:
The user-level storage location for **Supervisor Agent** daemon configuration, the **Agent Registry**, and conversation receipts.
_Avoid_: Global history, central repository

**Supervisor Socket**:
The local Unix socket in the **Supervisor Home** that the **AFS CLI** uses to connect to the running **Supervisor Agent**.
_Avoid_: HTTP endpoint, daemon port

**Conversation Receipt**:
The **Supervisor Agent**'s durable summary of a completed user prompt.
_Avoid_: Chat log, transcript

**Progress Update**:
A live status message from the **Supervisor Agent** while an **Agent Conversation** is still in progress.
_Avoid_: Spinner, log line

**AFS CLI**:
The initial user interface for starting the **Supervisor Agent** daemon and sending prompts to it.
_Avoid_: Terminal UI, command shell

**AFS Prompt UI**:
A later minimal graphical interface for sending prompts to the **Supervisor Agent** and viewing answers.
_Avoid_: Dashboard, control panel

## Relationships

- A **Managed Directory** has exactly one **Directory Agent**
- A **Directory Agent** belongs to exactly one **Managed Directory**
- A **Directory Agent** manages its **Managed Subtree**, not necessarily every descendant path
- A **Directory Agent** can read, write, and update files in its **Managed Subtree**
- A **Directory Agent** can answer questions about files in its **Managed Subtree** and return **File References** to the **Supervisor Agent**
- The **Supervisor Agent** can return **File References** that the user can click to open files
- Every **File Reference** points to a file in a **Managed Subtree**
- If a **Directory Agent** finds a file, that file is by definition in its **Managed Subtree**
- A **Directory Agent** maintains a **Directory Index** for its **Managed Subtree**
- The initial **Directory Index** stores file paths, metadata, content digests, and lightweight summaries where practical
- The initial **Directory Index** uses cheap local type-specific extraction where practical, especially PDF text extraction
- Vision, OCR, and audio transcription are optional future agent capabilities, not required core indexing behavior
- A **Directory Agent** reads full file contents on demand when answering
- A **Directory Agent** can answer while its **Directory Index** is still warming
- When **Index Status** is incomplete and affects confidence, the reply includes a partial-index caveat
- A **Directory Agent** updates its **Directory Index** and **Directory History** from **Filesystem Events**
- A **Directory Agent** normalizes related **Filesystem Events** into **Settled Changes**
- Editor atomic-save patterns should usually become one **External Change** for the final path
- Temporary files are recorded only if they remain in the **Managed Subtree** after the settle window
- The initial system requires filesystem-event support such as Linux inotify
- The initial system does not include a polling fallback for monitoring
- On restart, a **Directory Agent** performs **Startup Reconciliation** before marking **Index Status** healthy
- **Startup Reconciliation** is not a polling fallback; it is crash and downtime recovery
- **Startup Reconciliation** records missed changes as one **External Change** batch with the changed files listed
- A **Directory Agent** can answer during **Startup Reconciliation** with a reconciling caveat
- Mutation requests wait until **Startup Reconciliation** completes
- The initial system is a **Linux-Only Experiment**
- A **Managed Directory** has an **AFS Ignore Policy**
- The **AFS Ignore Policy** can be seeded from `.gitignore` when present
- The **AFS Ignore Policy** is stored and configured in the **Agent Home**
- The default **AFS Ignore Policy** excludes **Agent Home** contents, nested **Managed Directories**, and already-supervised symlink targets
- Ignored entries can still be inside a **Managed Subtree**
- Ignored entries are excluded from normal monitoring, indexing, and answers
- A **Directory Agent** modifies ignored entries only when explicitly asked with a path
- A change to an ignored entry still becomes a **History Entry** when made by a **Directory Agent**
- The **Supervisor Agent** does not maintain a global content index in the initial system
- Content knowledge remains local to **Directory Agents**
- An **Agent Home** is control data and is excluded from normal **Directory Index** and question-answering results
- **Agent Home** contents are inspected only for AFS maintenance or explicit control-state requests
- Changes inside a **Managed Directory** are recorded in **Directory History** as **Agent Changes** or **External Changes**
- **Directory History** is stored in a dedicated **History Backend** inside the **Agent Home**
- The initial **History Backend** can be implemented with git but is separate from any user or project git repository
- **Directory History** tracks all managed file types, including binary files
- `afs install <path>` creates a **History Baseline** synchronously before reporting success
- An **Agent Change** is a kind of **History Entry**
- An **External Change** is a kind of **History Entry**
- A **History Entry** is the user-facing undo unit
- An **Agent Change** is the undo unit for work performed by a **Directory Agent**
- An **External Change** is tracked for audit and baseline but is not attributed to a **Task Request**
- A **Directory Agent** creates an **Agent Change** from a known base state
- If a relevant file changes before an **Agent Change** is recorded, the agent reports a **Change Conflict** instead of overwriting
- A conflicted **Agent Change** can be retried against the latest **Directory History**
- A **Directory Agent** can observe continuously but creates an **Agent Change** only while handling a **Task Request**
- Background monitoring does not authorize autonomous file mutation
- While handling a **Task Request**, a **Directory Agent** may decide to persist generated information to files in its **Managed Subtree**
- A **Directory Agent** handles one active **Task Request** at a time in the initial system
- **Task Requests** are serialized per **Directory Agent**
- A busy **Directory Agent** places new **Task Requests** in its **Task Queue**
- The initial **Task Queue** is first-in, first-out
- Interrupt and cancel behavior is a future feature
- A **Directory Agent** can run **Agent Commands** inside its **Managed Subtree**
- An **Agent Command** runs with the **Managed Directory** as its working directory by default
- Outside-subtree file access by an **Agent Command** is out of scope unless it goes through an included symlink target in the **Managed Subtree**
- The initial system relies on agent discipline and history checks rather than OS-level sandboxing
- **Agent Command** outputs and file modifications are reported in the agent's reply
- A **Directory Agent** can send a **Task Request** directly to another **Directory Agent**
- Any **Directory Agent** can request mutation from any other **Directory Agent**
- The initial system has no inter-agent permissions system; a **Directory Agent** has full permission inside its own **Managed Subtree**
- The initial system has no enforced privacy boundary between **Directory Agents**
- A **Directory Agent** can summarize or selectively disclose information based on its instructions, but this is not an access-control system
- A **Task Request** names a **Reply Target**, either the delegating **Directory Agent** or the **Supervisor Agent**
- A **Directory Agent** records the requester when it creates an **Agent Change**
- A **Task Request** can result in file mutation by default
- Every agent reply includes a **Change Report**
- A **Broadcast Request** is sent to all registered **Directory Agents**
- A **Broadcast Request** has a configurable reply timeout
- A **Broadcast Request** reply includes a **Relevance Reply**
- In normal operation, a **Directory Agent** stays silent when its **Relevance Reply** would be `none`
- In diagnostic mode, a **Directory Agent** can reply with `none` and a reason
- The **Supervisor Agent** continues an **Agent Conversation** with the **Directory Agents** that replied before the timeout
- Late replies can be ignored or attached to the **Conversation Receipt** as late
- The **Supervisor Agent** routes and aggregates **Broadcast Requests** but is not a **Broadcast Request** recipient
- A user prompt can lead to an **Agent Conversation**
- An **Agent Conversation** can mix **Broadcast Requests** and direct **Agent Messages**
- An **Agent Conversation** records its **Conversation Participants**
- Visibility in an **Agent Conversation** depends on whether each **Agent Message** is broadcast or direct
- The initial system does not require a formal discovery protocol; relevant **Directory Agents** reply when they have useful information
- When a user prompt names an explicit path in a **Managed Subtree**, the **Supervisor Agent** routes directly to the owning **Directory Agent**
- When a user prompt does not identify a target **Managed Subtree**, the **Supervisor Agent** can start with a **Broadcast Request**
- When a user prompt names an unmanaged path, the **Supervisor Agent** reports that it is unmanaged and can suggest `afs install <path>` or a suitable parent path
- The **Supervisor Agent** tells the user which files changed and which **Conversation Participants** were involved
- The **Supervisor Agent** groups **Conversation Participants** by agents that answered, agents that modified files, and agents that were consulted without changes
- While an **Agent Conversation** is running, the **Supervisor Agent** emits **Progress Updates** about replies, waiting agents, queueing, and file modifications
- A **Directory Agent** becomes a **Registered Agent** when the **Supervisor Agent** installs it or re-discovers it during startup
- The **Agent Registry** is the **Supervisor Agent**'s source of truth for routing direct requests and **Broadcast Requests**
- The **Agent Registry** maps **Managed Directories** to agent endpoints and ownership metadata
- A **Managed Directory** contains one **Agent Home**
- The **Agent Home** stores durable agent state; runtime endpoint data is refreshed in the **Agent Registry**
- The **Agent Home** stores the **Agent Identity**
- The **Agent Home** stores **Agent Instructions**
- **Agent Instructions** customize how a **Directory Agent** summarizes, answers, edits, and collaborates
- **Agent Instructions** do not change ownership, history, or permission rules
- **Agent Messages** use **Agent Identity** internally
- User-facing summaries show a display name derived from the **Managed Directory** path by default
- The **Supervisor Agent** re-discovers **Directory Agents** by finding **Agent Homes**
- If a **Managed Directory** is renamed or moved with its **Agent Home**, the same **Agent Identity** follows the directory
- On startup, the **Supervisor Agent** can re-discover an **Agent Identity** at a new **Managed Directory** path and update the **Agent Registry**
- A directory without an **Agent Home** is unmanaged, even if it has the same path as a formerly managed directory
- A **Directory Agent** is built on the **Pi Agent Runtime**
- The **Supervisor Agent** is built on the **Pi Agent Runtime** with registry, routing, and installation responsibilities
- A **Directory Agent** runs as a long-running **Agent Process**
- While the **Supervisor Agent** daemon is running, installed **Directory Agents** are expected to have running **Agent Processes**
- The **Supervisor Agent** owns the lifecycle of **Directory Agent** processes
- The **Supervisor Agent** starts, stops, restarts, and reconnects **Directory Agent** processes
- On startup, the **Supervisor Agent** discovers **Agent Homes**, starts or reconnects **Directory Agents**, and registers them
- The initial **Agent Transport** uses the **Pi Agent Runtime** RPC mode over local stdio
- Direct **Agent Messages** between **Directory Agents** are logically direct but physically routed by the **Supervisor Agent** in the initial system
- Installing a nested **Managed Directory** performs an **Ownership Split**
- An **Agent Removal** for a nested **Managed Directory** performs an **Ownership Merge**
- During an **Ownership Split**, the parent **Directory Agent** excludes the child subtree from its **Managed Subtree**
- During an **Ownership Merge**, the parent **Directory Agent** takes over the former child subtree and absorbs the child **Directory History**
- During an **Ownership Merge**, the parent preserves the child **Agent Changes** when possible
- During an **Ownership Merge**, the parent records a new **Agent Change** for the act of absorbing the child **Directory History**
- During **Agent Removal** with a parent **Directory Agent**, the removed **Agent Home** is moved into an **Agent Home Archive** under the parent **Agent Home**
- During **Agent Removal** with no parent **Directory Agent**, the removed agent's **Directory History** is lost unless preserved explicitly
- `afs remove <path> --discard-history` explicitly discards removable history and archives
- A **Cross-Directory Task** is non-atomic in the initial system
- A **Cross-Directory Task** can produce one independent **Agent Change** per participating **Directory Agent**
- The **Supervisor Agent** may record which **Agent Changes** came from a **Cross-Directory Task**, but it does not own the reversible history
- The **Supervisor Agent** stores durable coordination state in the **Supervisor Home**
- The **Supervisor Home** is `~/.afs/` by default
- The **Supervisor Home** stores daemon configuration, the **Agent Registry**, and **Conversation Receipts**
- The **Supervisor Home** does not store content history or a global content index
- The **Supervisor Home** contains the **Supervisor Socket**
- The initial system has exactly one running **Supervisor Agent** per **Supervisor Home**
- A **Conversation Receipt** stores the prompt, timestamp, **Conversation Participants**, final answer, **File References**, **Change Reports**, and **History Entry** IDs
- A **Conversation Receipt** does not store every intermediate **Agent Message** by default
- **Conversation Receipts** are internal supervisor state in the initial CLI
- A nested **Managed Directory** is excluded from its ancestor's **Managed Subtree**
- A filesystem entry belongs to at most one **Managed Subtree**
- A symlink target is included in a **Managed Subtree** only when no **Directory Agent** already supervises that target
- The **Supervisor Agent** can install and initialize **Directory Agents** in selected directories
- Only the **Supervisor Agent** installs or removes **Directory Agents**
- A **Directory Agent** can recommend installing or removing another **Directory Agent**, but the **Supervisor Agent** performs the operation
- The **Supervisor Agent** receives user prompts and coordinates answers from the file system
- The initial user interface is the **AFS CLI**
- The **AFS CLI** can start the **Supervisor Agent** daemon and send user prompts
- The **AFS CLI** connects to the running **Supervisor Agent** through the **Supervisor Socket**
- The **AFS CLI** streams **Progress Updates** while waiting for a final answer
- The initial **AFS CLI** commands are `afs daemon`, `afs install <path>`, `afs remove <path>`, `afs ask "<prompt>"`, `afs agents`, `afs history <path>`, and `afs undo <path> <history-entry>`
- `afs daemon` runs in the foreground by default
- `afs daemon` fails if a live **Supervisor Agent** already owns the **Supervisor Socket**
- `afs ask` fails with `daemon is not running` when the **Supervisor Agent** daemon is unavailable
- `afs undo <path> <history-entry>` can undo only the latest applicable **History Entry** in the initial system
- `afs undo` requires confirmation before undoing an **External Change** in interactive use
- `afs undo --yes` is required for scripted undo of an **External Change**
- Selective undo of older **History Entries** is a future feature
- `afs install <path>` creates default **Agent Instructions**
- `afs install <path>` can accept instructions from a file or purpose text
- `afs install <path>` is scriptable and does not require an interactive prompt in the initial system
- `afs install <path>` starts the **Directory Agent** and lets **Directory Index** building continue asynchronously after the **History Baseline** exists
- `afs agents` shows **Index Status**
- `afs agents` shows each **Registered Agent**'s managed path, process health, **Index Status**, reconciliation state, and **Task Queue** length
- `afs install <path>` is idempotent when `<path>` is already a **Managed Directory**
- `afs install <path>` performs an **Ownership Split** when `<path>` is inside an existing **Managed Subtree**
- `afs install <path>` excludes any existing nested **Managed Directories** from the new agent's **Managed Subtree**
- The **AFS Prompt UI** is a later interface, not part of the initial core
- `afs history <path>` shows newest-first **History Entries** with timestamp, type, short summary, affected file count, and current undoability

## Example dialogue

> **Dev:** "Does every directory get a **Directory Agent** automatically?"
> **Domain expert:** "No. Only a **Managed Directory** has one, and it becomes managed when the **Supervisor Agent** installs a **Directory Agent** there."
> **Dev:** "If `/projects` and `/projects/app` both have agents, does the `/projects` agent watch files inside `/projects/app`?"
> **Domain expert:** "No. A nested **Managed Directory** is an ownership boundary, so each file is managed by only one **Directory Agent**."
> **Dev:** "Can a **Directory Agent** fix a typo in a file it monitors?"
> **Domain expert:** "Yes, if the file is inside its **Managed Subtree**. The change is recorded in **Directory History** so it can be undone."
> **Dev:** "If I ask where my 2025 blood test PDF is, what does the system return?"
> **Domain expert:** "The **Supervisor Agent** returns a clickable **File Reference** to the PDF in the filesystem."
> **Dev:** "Can a **File Reference** point to an unmanaged file?"
> **Domain expert:** "No. If an agent found the file, it was in that agent's **Managed Subtree**."
> **Dev:** "Does a **Directory Agent** keep every file's full content in memory?"
> **Domain expert:** "No. It maintains a **Directory Index** with paths, metadata, digests, and lightweight summaries, then reads full contents on demand."
> **Dev:** "How does the **Directory Index** handle PDFs and images?"
> **Domain expert:** "It stores metadata and digests for all files and uses cheap local extraction where practical, such as PDF text extraction; richer media understanding can be added later."
> **Dev:** "Can I ask questions before indexing finishes?"
> **Domain expert:** "Yes. The **Directory Agent** can answer with partial data and direct reads, but it should include a partial-index caveat when that affects confidence."
> **Dev:** "How does a **Directory Agent** notice changes after installation?"
> **Domain expert:** "It consumes **Filesystem Events**, such as Linux inotify events; the initial system does not include polling fallback."
> **Dev:** "Does every temp file from an editor save become a separate history item?"
> **Domain expert:** "No. The agent normalizes event bursts into **Settled Changes**, usually one **External Change** for the final saved path."
> **Dev:** "What if files changed while a **Directory Agent** was crashed?"
> **Domain expert:** "On restart it performs **Startup Reconciliation** to catch missed changes before reporting healthy **Index Status**."
> **Dev:** "Does restart recovery invent separate history entries for every missed edit?"
> **Domain expert:** "No. **Startup Reconciliation** records one **External Change** batch for the files that changed while the agent was offline."
> **Dev:** "Can an agent answer questions while reconciliation is running?"
> **Domain expert:** "Yes, with a reconciling caveat, but mutation waits until **Startup Reconciliation** completes."
> **Dev:** "Does v1 support macOS or Windows?"
> **Domain expert:** "No. The initial system is a **Linux-Only Experiment** because monitoring depends on inotify-style filesystem events."
> **Dev:** "Does the **Supervisor Agent** keep one big searchable index of every managed file?"
> **Domain expert:** "No. It keeps routing metadata in the **Agent Registry**, while content knowledge stays local to **Directory Agents**."
> **Dev:** "Does a **Directory Agent** automatically index everything under its directory?"
> **Domain expert:** "No. It follows the **AFS Ignore Policy**, seeded from `.gitignore` when useful and always excluding control and separately managed areas."
> **Dev:** "If `node_modules/` is ignored, is it outside the **Managed Subtree**?"
> **Domain expert:** "No. It can still be inside the **Managed Subtree**, but the agent ignores it unless explicitly asked with a path; agent-made changes still become **History Entries**."
> **Dev:** "Will normal project answers include files inside `.afs/`?"
> **Domain expert:** "No. **Agent Home** contents are control data and are excluded unless the user explicitly asks about AFS control state."
> **Dev:** "Do **Agent Changes** become commits in my project git repo?"
> **Domain expert:** "No. **Directory History** uses a dedicated **History Backend** inside `.afs/`, separate from any user or project git repository."
> **Dev:** "When is `afs install` allowed to report success?"
> **Domain expert:** "After it creates the **Agent Home**, records the **History Baseline**, starts the **Directory Agent**, and registers it; indexing can continue asynchronously."
> **Dev:** "Are PDFs and images recorded in **Directory History**?"
> **Domain expert:** "Yes. **Directory History** tracks all managed file types, even when the **Directory Index** can only summarize some of them."
> **Dev:** "If one request changes three files, what does undo reverse?"
> **Domain expert:** "The whole **Agent Change**. All file edits from that request are rolled back together."
> **Dev:** "If I edit a managed file directly in my editor, does the **Directory Agent** ignore it?"
> **Domain expert:** "No. It records an **External Change** in **Directory History**, separate from agent-authored work."
> **Dev:** "Can I undo my own direct edit through the same history interface?"
> **Domain expert:** "Yes. Both **Agent Changes** and **External Changes** are **History Entries**, and a **History Entry** is the user-facing undo unit."
> **Dev:** "What if I save a file while a **Directory Agent** is editing it?"
> **Domain expert:** "The agent records a **Change Conflict** instead of overwriting your change, then the work can be retried from the latest history."
> **Dev:** "Can a **Directory Agent** rewrite a file just because it noticed a problem?"
> **Domain expert:** "No. It can notice and report, but mutation requires a **Task Request**."
> **Dev:** "Can an agent save generated information even if the user did not explicitly say 'save it'?"
> **Domain expert:** "Yes, while handling a **Task Request** a **Directory Agent** may decide to persist useful information; the resulting **Change Report** makes that visible and undoable."
> **Dev:** "Can a **Directory Agent** work on two mutation-capable tasks at once?"
> **Domain expert:** "Not in the initial system. Each **Directory Agent** serializes **Task Requests** and handles one active task at a time."
> **Dev:** "What happens if a second request arrives while the agent is busy?"
> **Domain expert:** "It goes into the agent's **Task Queue** and waits its turn; the initial queue is first-in, first-out."
> **Dev:** "Can a **Directory Agent** run `pdftotext` or `rg`?"
> **Domain expert:** "Yes. It can run **Agent Commands** inside its **Managed Subtree**, and its reply reports command effects and modified files."
> **Dev:** "What prevents a command from editing files outside the **Managed Subtree**?"
> **Domain expert:** "In the initial system, the agent treats that as out of scope and history checks detect managed changes; there is no OS-level sandbox yet."
> **Dev:** "Can the `/src` agent ask the `/docs` agent to update documentation?"
> **Domain expert:** "Yes. It sends a **Task Request** directly and names whether the **Reply Target** is itself or the **Supervisor Agent**."
> **Dev:** "Does the `/src` agent need to say whether `/docs` is allowed to modify files?"
> **Domain expert:** "No. Mutation is allowed by default, but every reply includes a **Change Report** so modified files are visible."
> **Dev:** "Does the `/docs` agent need a separate permission grant before accepting a mutation request from `/src`?"
> **Domain expert:** "No. There is no inter-agent permissions system in the initial design; safety comes from recording an **Agent Change** that can be reverted."
> **Dev:** "Can `/runs` ask `/health` for sleep and energy context?"
> **Domain expert:** "Yes. There are no enforced privacy ACLs in v1; transparency comes from **Conversation Receipts** and participant summaries."
> **Dev:** "Who receives a **Broadcast Request**?"
> **Domain expert:** "All registered **Directory Agents** receive it."
> **Dev:** "Does `afs ask` wait forever for every broadcast reply?"
> **Domain expert:** "No. **Broadcast Requests** use a configurable timeout, and the **Supervisor Agent** continues with agents that replied in time."
> **Dev:** "How does an agent tell the **Supervisor Agent** it may have useful information?"
> **Domain expert:** "Its broadcast response includes a **Relevance Reply** of `none`, `possible`, or `strong`, plus a short reason."
> **Dev:** "Do irrelevant agents reply to every broadcast?"
> **Domain expert:** "No. In normal operation they stay silent; diagnostic mode can ask them to reply with `none` and a reason."
> **Dev:** "Does the **Supervisor Agent** also answer a **Broadcast Request**?"
> **Domain expert:** "No. The **Supervisor Agent** routes and aggregates the broadcast; the recipients are **Directory Agents**."
> **Dev:** "How does the system answer 'find my last blood tests from 2025'?"
> **Domain expert:** "The **Supervisor Agent** can broadcast the request. Agents with relevant files reply, and the **Supervisor Agent** returns the answer with **File References**."
> **Dev:** "How does the system answer 'what is today's run workout?' when `/runs` needs health context?"
> **Domain expert:** "The **Supervisor Agent** can broadcast for run information, then ask `/runs` directly. The `/runs` agent can message `/health` directly, use that reply, and send the workout back to the **Supervisor Agent**."
> **Dev:** "If I ask about `/runs/today.md`, does the **Supervisor Agent** still broadcast first?"
> **Domain expert:** "No. An explicit path lets the **Supervisor Agent** route directly to the owning **Directory Agent**."
> **Dev:** "What if I ask about a path that no **Directory Agent** manages?"
> **Domain expert:** "The **Supervisor Agent** reports that the path is unmanaged and can suggest `afs install <path>` or a suitable parent."
> **Dev:** "What should the **Supervisor Agent** tell the user after that conversation?"
> **Domain expert:** "It returns the answer, any **File References**, the **Change Reports**, and grouped **Conversation Participants**."
> **Dev:** "Does `afs ask` stay silent until every agent is done?"
> **Domain expert:** "No. The **AFS CLI** streams **Progress Updates** while the **Supervisor Agent** waits for the final answer."
> **Dev:** "How does the **Supervisor Agent** know which **Directory Agents** exist?"
> **Domain expert:** "It uses the **Agent Registry**, populated when it installs agents and when it re-discovers them during startup."
> **Dev:** "How are agents identified in messages and summaries?"
> **Domain expert:** "Messages use the stable **Agent Identity**, while user-facing summaries show the managed path or derived display name."
> **Dev:** "Can `/health` and `/runs` behave differently?"
> **Domain expert:** "Yes. Each **Directory Agent** has **Agent Instructions** in its **Agent Home**, but those instructions do not change ownership or history rules."
> **Dev:** "What happens if `/health` is moved to `/personal/health` with its `.afs/` directory?"
> **Domain expert:** "The same **Agent Identity** follows the moved **Agent Home**, and the **Supervisor Agent** updates the **Agent Registry** when it re-discovers it."
> **Dev:** "What survives when the **Supervisor Agent** restarts?"
> **Domain expert:** "Each **Managed Directory** keeps an **Agent Home** at `.afs/`; the **Supervisor Agent** rebuilds runtime routing from those homes."
> **Dev:** "What is the agent implementation based on?"
> **Domain expert:** "Both **Directory Agents** and the **Supervisor Agent** are built on the external **Pi Agent Runtime**."
> **Dev:** "Does a **Directory Agent** start only when asked a question?"
> **Domain expert:** "No. A **Directory Agent** is a long-running **Agent Process** so it can monitor its **Managed Subtree**."
> **Dev:** "Who restarts a **Directory Agent** if its process exits?"
> **Domain expert:** "The **Supervisor Agent** owns the **Directory Agent** process lifecycle and restarts it."
> **Dev:** "Do **Directory Agents** open network sockets to talk to each other?"
> **Domain expert:** "No. The initial **Agent Transport** uses Pi RPC over local stdio, with the **Supervisor Agent** routing messages between child processes."
> **Dev:** "What happens when `/projects/app` gets its own **Directory Agent** under managed `/projects`?"
> **Domain expert:** "That is an **Ownership Split**: `/projects` stops managing the `/projects/app` subtree, and the new child agent takes over."
> **Dev:** "What happens if the `/projects/app` agent is removed?"
> **Domain expert:** "That is an **Ownership Merge**: the `/projects` agent takes over the former child subtree and absorbs its **Directory History**."
> **Dev:** "Does the parent rewrite the child's old history as if the parent made those changes?"
> **Domain expert:** "No. The parent preserves the child **Agent Changes** when possible and records a new **Agent Change** for the merge itself."
> **Dev:** "What happens to the child's `.afs/` directory after removal?"
> **Domain expert:** "When there is a parent, it is moved into an **Agent Home Archive** under the parent **Agent Home**."
> **Dev:** "What if the removed agent has no parent **Directory Agent**?"
> **Domain expert:** "Then its **Directory History** is lost unless preservation is explicitly requested before **Agent Removal** completes."
> **Dev:** "If a request changes files under `/docs` and `/src`, is that one global transaction?"
> **Domain expert:** "No. It is a non-atomic **Cross-Directory Task** with separate **Agent Changes** owned by the participating **Directory Agents**."
> **Dev:** "Where does the **Supervisor Agent** store its own state?"
> **Domain expert:** "In the **Supervisor Home**, `~/.afs/` by default, for configuration, registry data, and conversation receipts."
> **Dev:** "How does the **AFS CLI** talk to the running **Supervisor Agent**?"
> **Domain expert:** "Through the **Supervisor Socket**, a local Unix socket in the **Supervisor Home**."
> **Dev:** "What happens if I try to start a second daemon?"
> **Domain expert:** "The command fails because the initial system allows exactly one running **Supervisor Agent** per **Supervisor Home**."
> **Dev:** "Does the **Supervisor Agent** save every message agents sent each other?"
> **Domain expert:** "No. It stores a **Conversation Receipt** with the prompt, answer, participants, file references, change reports, and history entry IDs."
> **Dev:** "Can I browse **Conversation Receipts** from the CLI in v1?"
> **Domain expert:** "Not initially. **Conversation Receipts** stay internal while the conversation model settles."
> **Dev:** "How does the user talk to the **Supervisor Agent** first?"
> **Domain expert:** "Through the **AFS CLI**. A minimal **AFS Prompt UI** can come later."
> **Dev:** "What if I run `afs ask` before the daemon is running?"
> **Domain expert:** "The command fails with `daemon is not running`."
> **Dev:** "Does `afs daemon` detach into the background by default?"
> **Domain expert:** "No. It runs in the foreground by default."
> **Dev:** "What is the first complete **AFS CLI** loop?"
> **Domain expert:** "Run `afs daemon`, install agents with `afs install`, ask with `afs ask`, inspect with `afs agents` and `afs history`, remove with `afs remove`, and undo with `afs undo`."
> **Dev:** "Can I undo a **History Entry** from last week while keeping today's changes?"
> **Domain expert:** "Not in the initial system. `afs undo` only reverses the latest applicable **History Entry**."
> **Dev:** "Can `afs undo` silently undo a direct edit I made in my editor?"
> **Domain expert:** "No. Undoing an **External Change** requires confirmation, or `--yes` in scripts."
> **Dev:** "What should `afs history <path>` show by default?"
> **Domain expert:** "Newest-first **History Entries** with timestamp, type, short summary, affected file count, and whether each entry is currently undoable."
> **Dev:** "How are **Agent Instructions** created?"
> **Domain expert:** "`afs install <path>` creates defaults and can accept instructions from a file or purpose text without requiring an interactive setup."
> **Dev:** "What if I run `afs install` on a directory that is already managed?"
> **Domain expert:** "The command is idempotent: it reports the existing **Directory Agent** and makes no ownership change."
> **Dev:** "What should `afs agents` show?"
> **Domain expert:** "Each **Registered Agent**'s path, process health, **Index Status**, reconciliation state, and **Task Queue** length."
> **Dev:** "What if the new install path already contains nested **Managed Directories**?"
> **Domain expert:** "The new **Directory Agent** manages the subtree around them, but existing nested **Managed Directories** remain excluded."
> **Dev:** "Can a **Directory Agent** create a nested **Directory Agent** by itself?"
> **Domain expert:** "No. It can recommend that change, but only the **Supervisor Agent** installs or removes **Directory Agents**."

## Flagged ambiguities

- "each directory runs an agent" could mean every filesystem directory has an agent — resolved: only selected **Managed Directories** have installed **Directory Agents**.
- "manages the subtree" could imply overlapping parent and child responsibility — resolved: a **Directory Agent** manages its **Managed Subtree**, which excludes nested **Managed Directories** and already-supervised symlink targets.
- "authorized Directory Agent" implied a permissions system — resolved: the initial system has no inter-agent permissions system; every **Directory Agent** can request work from another.
- "agent is deleted" could mean stopping a process or removing directory ownership — resolved: **Agent Removal** means removal through the **Supervisor Agent**, with child history merged into a parent when one exists.
- "pi agent" could mean an AFS-native agent kind — resolved: **Pi Agent Runtime** means the external Pi coding-agent harness used as the base runtime for AFS agents.
- "Discovery Round" implied a required formal protocol — resolved: **Agent Conversation** is a looser exchange that may use broadcasts or direct messages as needed.
- "`afs ask` could auto-start the daemon" was considered — resolved: the CLI fails with `daemon is not running` instead of starting the daemon automatically.
- "multiple daemons could share one home" was considered — resolved: the initial system allows exactly one running **Supervisor Agent** per **Supervisor Home**.
