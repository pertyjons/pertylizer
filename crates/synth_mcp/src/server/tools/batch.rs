//! batch MCP tool handlers.

use super::super::*;

#[tool_router(router = batch_tool_router, vis = "pub(crate)")]
impl SynthMcpServer {
    #[tool(
        description = "Execute multiple tool calls in a single request to reduce round-trip latency. \
                       Operations run sequentially. Max 50 operations per batch. \
                       Cannot nest batch_execute inside a batch. \
                       Set `dry_run: true` to validate every operation (tool name + params) without \
                       executing any — nothing is mutated. Set `rollback: true` to make the batch \
                       all-or-nothing: the project is snapshotted first and restored unless every \
                       operation completes — a `partial` op counts as not completing, since it leaves \
                       the project half-changed. LIMITS: the snapshot is rebuilt from engine state, \
                       so it restores what a project *save* captures — files already written \
                       (save_project, export_sample, render_to_wav), transport and recording side \
                       effects, and GUI-only fields (glide time, octave offset, module layout, which \
                       instrument is active) are not restored. It is also not isolated from other \
                       clients: if any other call writes while the batch runs, the restore is \
                       REFUSED (reported in `rollback_error`) rather than reverting over that \
                       write, and this batch's own writes stand. Note this triggers on *any* \
                       overlapping call, including your own if you send requests without waiting \
                       for replies — the batch cannot tell whose write it would be undoing. \
                       `rollback_error` means the restore itself went wrong: with \
                       `rolled_back: false` nothing was put back and the batch's changes stand, \
                       with `rolled_back: true` the project was replaced but does not match the \
                       snapshot exactly. \
                       Each op reports `status` (success / partial / failure), `structured` — the same \
                       payload a direct call puts in `structuredContent`, so a list tool's items are at \
                       `structured.items` and a mutation's per-item results at `structured.items[].error` \
                       — and `message`, its sentence or error (absent for a reader, whose answer is \
                       `structured`). Counts: succeeded / partial / failed / skipped, against `total` \
                       requested.",
        // Not `destructive_hint = false`: this tool executes whatever it is given,
        // including `delete_*`, `new_project` and `load_project`. A client that
        // gates confirmation on the hint would wave those through.
        annotations(destructive_hint = true)
    )]
    pub(crate) async fn batch_execute(
        &self,
        params: Parameters<BatchExecuteParam>,
    ) -> Result<Json<BatchExecResult>, String> {
        use crate::types::{BatchExecItemResult, ToolOutcome};

        let p = params.0;
        let dry_run = p.dry_run.unwrap_or(false);
        let rollback = p.rollback.unwrap_or(false) && !dry_run;
        // Rollback restores on the first failure, so executing past it is wasted
        // work that would only be undone — stop at the first error.
        let stop_on_error = p.stop_on_error.unwrap_or(false) || rollback;

        if p.operations.is_empty() {
            return Err("Error: operations array is empty".to_string());
        }
        if p.operations.len() > 50 {
            return Err(format!(
                "Error: too many operations ({}). Maximum is 50 per batch.",
                p.operations.len()
            ));
        }

        // Snapshot the project before mutating anything so a failed rollback
        // batch can be undone. Skipped for dry_run (nothing executes).
        if rollback && let Err(e) = self.bridge.capture_snapshot() {
            return Err(format!("Error: could not capture rollback snapshot: {e}"));
        }
        // Where the shared mutation counter stood before this batch wrote anything,
        // plus one per mutating operation this batch goes on to run. *Predicted*,
        // not observed: reading the counter back after each op would absorb any
        // increment another client made in between, which is precisely what this
        // is here to notice.
        let mut expected_seq = self.bridge.mutation_seq();

        let total = p.operations.len();
        let mut results = Vec::with_capacity(total);

        for (i, op) in p.operations.into_iter().enumerate() {
            if op.tool == "batch_execute" {
                results.push(BatchExecItemResult {
                    index: i,
                    tool: op.tool,
                    status: ToolOutcome::Failure,
                    structured: None,
                    message: Some("Error: batch_execute cannot be nested".to_string()),
                });
                if stop_on_error {
                    break;
                }
                continue;
            }

            // The handler stated this op's verdict; nothing here re-reads the
            // result to work out what happened.
            // Counted by the same predicate `dispatch_tool` bumps on, so the two
            // cannot drift.
            if !dry_run && !self.is_read_only_tool(&op.tool) {
                expected_seq += 1;
            }
            let dispatched = self.dispatch_tool(&op.tool, op.params, dry_run).await;
            let status = dispatched.outcome();
            results.push(dispatched.into_item(i, op.tool));
            // "Did everything it was asked to" is the bar, not "did not fail
            // outright". A partial op has left some of the request undone, and a
            // caller who asked for all-or-nothing is owed a stop and a restore —
            // undoing the half that landed is the point of asking.
            if status != ToolOutcome::Success && stop_on_error {
                break;
            }
        }

        let tally = |wanted: ToolOutcome| results.iter().filter(|op| op.status == wanted).count();
        let (succeeded, partial, failed) = (
            tally(ToolOutcome::Success),
            tally(ToolOutcome::Partial),
            tally(ToolOutcome::Failure),
        );
        // Only `stop_on_error` leaves operations unattempted, and it breaks out,
        // so everything past the last result was skipped.
        let skipped = total - results.len();

        // Resolve the rollback snapshot: restore unless the batch completed.
        let completed = succeeded == total;
        let mut rolled_back = false;
        let mut rollback_error: Option<String> = None;
        if rollback {
            let concurrent = self.bridge.mutation_seq() != expected_seq;
            if completed {
                self.bridge.clear_snapshot();
            } else if concurrent {
                // Refuse rather than clobber. The snapshot predates the other
                // client's write, so applying it would delete work somebody was
                // told had succeeded — a worse outcome than this batch's own
                // changes standing, which is at least a state the caller knows
                // about and can address.
                self.bridge.clear_snapshot();
                rollback_error = Some(
                    "Error: not restored — another client modified the project during this \
                     batch, and the snapshot predates that change. This batch's writes stand; \
                     undo them deliberately rather than reverting over someone else's work."
                        .to_string(),
                );
            } else {
                match self.bridge.restore_snapshot() {
                    // The project was put back, so `rolled_back` says so even when
                    // the reconstruction was imperfect: the batch's writes are gone
                    // either way, and it is `rollback_error` that says the restored
                    // state is not exactly the snapshot.
                    Ok(unreconstructed) => {
                        rolled_back = true;
                        if !unreconstructed.is_empty() {
                            rollback_error = Some(format!(
                                "Error: the project was restored incompletely ({} item(s) \
                                 could not be reconstructed): {}",
                                unreconstructed.len(),
                                unreconstructed.join("; ")
                            ));
                        }
                    }
                    // Nothing was put back at all, so the batch's changes stand.
                    // Reported as what it is — a failure of the batch machinery,
                    // not a phantom operation the caller never requested. As an
                    // item it also collided with the index of a skipped one.
                    Err(e) => rollback_error = Some(format!("Error: rollback failed: {e}")),
                }
            }
        }

        Ok(Json(BatchExecResult {
            total,
            succeeded,
            partial,
            failed,
            skipped,
            dry_run,
            rolled_back,
            rollback_error,
            results,
        }))
    }
}
