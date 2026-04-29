/**
 * AFS structured-output Pi extension.
 *
 * Vendored by the AFS supervisor into every Agent Home and loaded
 * through `pi --mode rpc -e <path>`. Defines the single tool every
 * directory agent must call to terminate a turn. AFS reads the
 * args from the `tool_execution_end` event on Pi's stdout.
 *
 * The schema is mirrored in `src/agent_rpc.rs` (Rust deserializer)
 * and pinned by `schema_version: 1`. Bumping the schema requires
 * coordinated changes in both files.
 *
 * See `docs/prd/agentic-file-system-v2-pi-rpc-rewrite.md` for the
 * decision record.
 */

import { defineTool, type ExtensionAPI } from "@mariozechner/pi-coding-agent";
import { StringEnum } from "@mariozechner/pi-ai";
import { Type } from "typebox";

const afsReplyTool = defineTool({
	name: "afs_reply",
	label: "AFS Reply",
	description:
		"Return the final structured answer for this AFS turn. Always call this tool exactly once at the end of every turn, including when the directory is not relevant (set relevance to 'none' and leave answer empty).",
	promptSnippet:
		"End every AFS turn with a single afs_reply tool call carrying the structured reply.",
	promptGuidelines: [
		"Call afs_reply exactly once as the final action of every turn.",
		"Set relevance to 'strong' when the managed directory is the right place to answer, 'possible' when it might be, 'none' when it is not. When relevance is 'none', leave answer empty and delegates empty.",
		"Put any cross-directory questions you want another agent to answer into delegates[] with target=<peer agent identity or absolute path>, reply_target='delegator' (the reply comes back to you so you can refine your own answer) or 'supervisor' (the supervisor consumes the reply directly).",
		"Populate file_references with managed-subtree paths the user may want to inspect. Populate changed_files and history_entries when the turn modified the managed directory.",
		"Never emit another assistant message in the same turn after afs_reply.",
	],
	parameters: Type.Object({
		schema_version: Type.Literal(1, {
			description: "Schema version pin. Always 1 in this extension.",
		}),
		relevance: StringEnum(["none", "possible", "strong"], {
			description: "Whether this managed directory is relevant to the prompt.",
		}),
		reason: Type.String({
			description: "Short rationale for the chosen relevance level.",
		}),
		answer: Type.String({
			description: "The user-visible answer for this turn. Empty when relevance is 'none'.",
		}),
		file_references: Type.Array(Type.String(), {
			description: "Managed-subtree paths that support the answer.",
		}),
		changed_files: Type.Array(Type.String(), {
			description: "Managed-subtree paths the agent modified during this turn.",
		}),
		history_entries: Type.Array(Type.String(), {
			description: "AFS history entry identifiers produced during this turn.",
		}),
		delegates: Type.Array(
			Type.Object({
				target: Type.String({
					description: "Target agent identity or absolute managed-directory path.",
				}),
				reply_target: StringEnum(["delegator", "supervisor"], {
					description: "Where the delegated reply should be routed.",
				}),
				prompt: Type.String({
					description: "Prompt for the delegated agent.",
				}),
			}),
			{
				description:
					"Zero or more delegation requests the supervisor should fan out before this turn closes.",
			},
		),
	}),

	async execute(_toolCallId, params) {
		return {
			content: [{ type: "text", text: "afs_reply recorded" }],
			details: params,
			terminate: true,
		};
	},
});

export default function (pi: ExtensionAPI) {
	pi.registerTool(afsReplyTool);
}
