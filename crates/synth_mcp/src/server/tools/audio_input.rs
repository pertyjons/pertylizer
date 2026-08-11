//! audio input MCP tool handlers.

use super::super::*;

#[tool_router(router = audio_input_tool_router, vis = "pub(crate)")]
impl SynthMcpServer {
    #[tool(
        description = "List available audio input devices (microphones, line-in, etc.).",
        annotations(read_only_hint = true)
    )]
    pub(crate) async fn list_input_devices(
        &self,
        _params: Parameters<NoParams>,
    ) -> Result<Json<Listing<InputDeviceInfo>>, String> {
        match self.bridge.list_input_devices() {
            Ok(devices) => Ok(Json(devices.into())),
            Err(e) => Err(format!("Error: {e}")),
        }
    }

    #[tool(
        description = "Get the current audio input state: monitoring status, recording status, \
                       peak level, and recording duration.",
        annotations(read_only_hint = true)
    )]
    pub(crate) async fn get_input_state(
        &self,
        _params: Parameters<NoParams>,
    ) -> Result<Json<InputStateInfo>, String> {
        match self.bridge.get_input_state() {
            Ok(state) => Ok(Json(state)),
            Err(e) => Err(format!("Error: {e}")),
        }
    }

    #[tool(
        output_schema = action_output_schema(),
        description = "Select an audio input device by id/name; pass null for the backend default.",
        annotations(destructive_hint = false, idempotent_hint = true)
    )]
    pub(crate) async fn set_input_device(
        &self,
        params: Parameters<SetInputDeviceParam>,
    ) -> CallToolResult {
        match self.bridge.set_input_device(params.0.device_id) {
            Ok(()) => action_ok("Input device selected".to_string()),
            Err(e) => action_failed(format!("Error: {e}")),
        }
    }

    #[tool(
        output_schema = action_output_schema(),
        description = "Start audio-input monitoring and connect it to Audio Input modules.",
        annotations(destructive_hint = false)
    )]
    pub(crate) async fn start_monitoring(&self, _params: Parameters<NoParams>) -> CallToolResult {
        match self.bridge.start_monitoring() {
            Ok(()) => action_ok("Input monitoring started".to_string()),
            Err(e) => action_failed(format!("Error: {e}")),
        }
    }

    #[tool(
        output_schema = action_output_schema(),
        description = "Stop audio-input monitoring and disconnect the engine input.",
        annotations(destructive_hint = false, idempotent_hint = true)
    )]
    pub(crate) async fn stop_monitoring(&self, _params: Parameters<NoParams>) -> CallToolResult {
        match self.bridge.stop_monitoring() {
            Ok(()) => action_ok("Input monitoring stopped".to_string()),
            Err(e) => action_failed(format!("Error: {e}")),
        }
    }

    #[tool(
        output_schema = action_output_schema(),
        description = "Start recording the monitored input on the dedicated recording-drain thread.",
        annotations(destructive_hint = false)
    )]
    pub(crate) async fn start_recording(&self, _params: Parameters<NoParams>) -> CallToolResult {
        match self.bridge.start_recording() {
            Ok(()) => action_ok("Input recording started".to_string()),
            Err(e) => action_failed(format!("Error: {e}")),
        }
    }

    #[tool(
        description = "Stop input recording and commit the captured audio as a library sample.",
        annotations(destructive_hint = false, idempotent_hint = true)
    )]
    pub(crate) async fn stop_recording(
        &self,
        params: Parameters<StopRecordingParam>,
    ) -> Result<Json<SampleInfo>, String> {
        match self.bridge.stop_recording(params.0.name) {
            Ok(sample) => Ok(Json(sample)),
            Err(e) => Err(format!("Error: {e}")),
        }
    }

    // ========================================================================
    // DISCOVERY TOOLS
    // ========================================================================
}
