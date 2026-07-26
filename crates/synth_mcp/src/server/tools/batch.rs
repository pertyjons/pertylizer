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
                       all-or-nothing: the project is snapshotted first and restored if any operation fails."
    )]
    pub(crate) async fn batch_execute(&self, params: Parameters<BatchExecuteParam>) -> String {
        use crate::types::{BatchExecItemResult, BatchExecResult};

        let p = params.0;
        let dry_run = p.dry_run.unwrap_or(false);
        let rollback = p.rollback.unwrap_or(false) && !dry_run;
        // Rollback restores on the first failure, so executing past it is wasted
        // work that would only be undone — stop at the first error.
        let stop_on_error = p.stop_on_error.unwrap_or(false) || rollback;

        if p.operations.is_empty() {
            return "Error: operations array is empty".to_string();
        }
        if p.operations.len() > 50 {
            return format!(
                "Error: too many operations ({}). Maximum is 50 per batch.",
                p.operations.len()
            );
        }

        // Snapshot the project before mutating anything so a failed rollback
        // batch can be undone. Skipped for dry_run (nothing executes).
        if rollback && let Err(e) = self.bridge.capture_snapshot() {
            return format!("Error: could not capture rollback snapshot: {e}");
        }

        let capacity = p.operations.len();
        let mut results = Vec::with_capacity(capacity);
        let mut succeeded = 0usize;
        let mut failed = 0usize;

        for (i, op) in p.operations.into_iter().enumerate() {
            if op.tool == "batch_execute" {
                results.push(BatchExecItemResult {
                    index: i,
                    tool: op.tool,
                    success: false,
                    result: "Error: batch_execute cannot be nested".to_string(),
                });
                failed += 1;
                if stop_on_error {
                    break;
                }
                continue;
            }

            // `dispatch_tool` already classified the result (the same
            // `result_is_failure` gate that set its log severity), so use its
            // verdict directly rather than re-parsing the result string here.
            let (result, is_error) = self.dispatch_tool(&op.tool, op.params, dry_run).await;
            results.push(BatchExecItemResult {
                index: i,
                tool: op.tool,
                success: !is_error,
                result,
            });
            if is_error {
                failed += 1;
                if stop_on_error {
                    break;
                }
            } else {
                succeeded += 1;
            }
        }

        // Resolve the rollback snapshot: restore on any failure, else discard.
        let mut rolled_back = false;
        if rollback {
            if failed > 0 {
                match self.bridge.restore_snapshot() {
                    Ok(()) => rolled_back = true,
                    Err(e) => {
                        results.push(BatchExecItemResult {
                            index: results.len(),
                            tool: "<rollback>".to_string(),
                            success: false,
                            result: format!("Error: rollback failed: {e}"),
                        });
                    }
                }
            } else {
                self.bridge.clear_snapshot();
            }
        }

        to_json(&BatchExecResult {
            total: succeeded + failed,
            succeeded,
            failed,
            dry_run,
            rolled_back,
            results,
        })
    }
}
