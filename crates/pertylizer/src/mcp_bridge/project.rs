use super::*;

impl synth_mcp::bridge::ProjectBridge for AppSynthBridge {
    fn new_project(&self) -> Result<String, McpBridgeError> {
        self.do_new_project()
    }

    fn save_project(&self, path: &str) -> Result<String, McpBridgeError> {
        self.do_save_project(std::path::PathBuf::from(path))
    }

    fn save_patch(
        &self,
        instrument_id: InstrumentId,
        path: &str,
    ) -> Result<String, McpBridgeError> {
        self.validate_instrument(instrument_id)?;
        self.do_save_patch(instrument_id, std::path::PathBuf::from(path))
    }

    fn load_project(&self, path: &str) -> Result<String, McpBridgeError> {
        self.do_load_project(std::path::PathBuf::from(path))
    }

    fn optimize_project(&self) -> Result<OptimizeResult, McpBridgeError> {
        // Remove unused patterns and tracks from the song
        let (removed_patterns, removed_tracks, used_instrument_ids) = {
            let mut song = self.shared.song.write();
            song.remove_unused()
        };

        // Remove instruments not referenced by remaining tracks/notes
        let mut removed_instruments = Vec::new();
        let snapshots = self.session.list_instruments();
        for snap in &snapshots {
            if !used_instrument_ids.contains(&snap.id)
                && self.session.remove_instrument(snap.id).is_ok()
            {
                removed_instruments.push(snap.name.clone());
            }
        }

        // Drop samples no remaining Sampler references — only meaningful
        // after the instrument removal above. An empty library lets the
        // next save stay on plain JSON instead of being forced into a
        // bundle.
        let removed_samples =
            crate::project_apply::prune_unused_samples(&self.session, &self.sample_library);

        let total_removed = removed_patterns.len()
            + removed_tracks.len()
            + removed_instruments.len()
            + removed_samples.len();

        Ok(OptimizeResult {
            removed_patterns,
            removed_tracks,
            removed_instruments,
            removed_samples,
            total_removed,
        })
    }
}
