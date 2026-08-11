//! project MCP tool handlers.

use super::super::*;

#[tool_router(router = project_tool_router, vis = "pub(crate)")]
impl SynthMcpServer {
    #[tool(
        output_schema = action_output_schema(),
        description = "Reset to a new empty project, clearing all instruments and song data.",
        annotations(destructive_hint = true)
    )]
    pub(crate) async fn new_project(&self, _params: Parameters<NoParams>) -> CallToolResult {
        match self.bridge.new_project() {
            Ok(msg) => action_ok(format!("OK: {msg}")),
            Err(e) => action_failed(format!("Error: {e}")),
        }
    }

    #[tool(
        output_schema = action_output_schema(),
        description = "Save the current project (all instruments, patches, song, arrangement). A caller-supplied `.ptz` (recommended) or `.json` path is preserved as-is; any other extension is normalized to `.ptz`. If the project embeds samples it is written as a `.ptz.zip` bundle instead (the filename must tell the truth about the format). The returned message reports the exact path written.",
        annotations(destructive_hint = true)
    )]
    pub(crate) async fn save_project(
        &self,
        params: Parameters<ProjectPathParam>,
    ) -> CallToolResult {
        if let Err(e) = validate_file_path(&params.0.path) {
            return action_rejected(e);
        }
        tokio::task::block_in_place(|| match self.bridge.save_project(&params.0.path) {
            Ok(msg) => action_ok(format!("OK: {msg}")),
            Err(e) => action_failed(format!("Error: {e}")),
        })
    }

    #[tool(
        output_schema = action_output_schema(),
        description = "Save a single instrument as a standalone patch file (its modules, \
        connections, and patch metadata only — no song or other instruments). This is the \
        single-instrument format that load_project reads back, distinct from save_project which \
        writes the whole project. It waits (bounded) for graph mutations queued earlier in the \
        SAME batch_execute (add_module/connect) to be applied before reading the graph, so an \
        in-batch build-then-save captures the freshly-added modules/connections.",
        annotations(destructive_hint = true)
    )]
    pub(crate) async fn save_patch(&self, params: Parameters<SavePatchParam>) -> CallToolResult {
        if let Err(e) = validate_file_path(&params.0.path) {
            return action_rejected(e);
        }
        tokio::task::block_in_place(|| {
            match self
                .bridge
                .save_patch(params.0.instrument_id, &params.0.path)
            {
                Ok(msg) => action_ok(format!("OK: {msg}")),
                Err(e) => action_failed(format!("Error: {e}")),
            }
        })
    }

    #[tool(
        output_schema = action_output_schema(),
        description = "Load a project or patch file, replacing all current state. Supports both project files and single patch files.",
        annotations(destructive_hint = true)
    )]
    pub(crate) async fn load_project(
        &self,
        params: Parameters<ProjectPathParam>,
    ) -> CallToolResult {
        if let Err(e) = validate_file_path(&params.0.path) {
            return action_rejected(e);
        }
        tokio::task::block_in_place(|| match self.bridge.load_project(&params.0.path) {
            Ok(msg) => action_ok(format!("OK: {msg}")),
            Err(e) => action_failed(format!("Error: {e}")),
        })
    }

    #[tool(
        description = "Optimize the project by removing unused patterns (not placed in arrangement), \
                       unused tracks (no placements), unused instruments (not referenced by any track or note), \
                       and unused samples (no `Sampler` module's `sample_select` references them). Pruning samples \
                       keeps the sample library empty when nothing uses it, which lets the next save stay on plain \
                       JSON instead of being forced into bundle format. Returns a summary of what was removed.",
        annotations(destructive_hint = true)
    )]
    pub(crate) async fn optimize_project(
        &self,
        _params: Parameters<NoParams>,
    ) -> Result<Json<OptimizeResult>, String> {
        match self.bridge.optimize_project() {
            Ok(result) => Ok(Json(result)),
            Err(e) => Err(format!("Error: {e}")),
        }
    }

    // ========================================================================
    // SAMPLE LIBRARY TOOLS
    // ========================================================================
}
