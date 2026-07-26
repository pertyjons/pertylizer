//! audio input MCP tool handlers.

use super::super::*;

#[tool_router(router = audio_input_tool_router, vis = "pub(crate)")]
impl SynthMcpServer {
    #[tool(description = "List available audio input devices (microphones, line-in, etc.).")]
    pub(crate) async fn list_input_devices(&self, _params: Parameters<NoParams>) -> String {
        match self.bridge.list_input_devices() {
            Ok(devices) => to_json(&devices),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "Get the current audio input state: monitoring status, recording status, \
                       peak level, and recording duration."
    )]
    pub(crate) async fn get_input_state(&self, _params: Parameters<NoParams>) -> String {
        match self.bridge.get_input_state() {
            Ok(state) => to_json(&state),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "Select an audio input device by id/name; pass null for the backend default."
    )]
    pub(crate) async fn set_input_device(&self, params: Parameters<SetInputDeviceParam>) -> String {
        match self.bridge.set_input_device(params.0.device_id) {
            Ok(()) => "Input device selected".to_string(),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(description = "Start audio-input monitoring and connect it to Audio Input modules.")]
    pub(crate) async fn start_monitoring(&self, _params: Parameters<NoParams>) -> String {
        match self.bridge.start_monitoring() {
            Ok(()) => "Input monitoring started".to_string(),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(description = "Stop audio-input monitoring and disconnect the engine input.")]
    pub(crate) async fn stop_monitoring(&self, _params: Parameters<NoParams>) -> String {
        match self.bridge.stop_monitoring() {
            Ok(()) => "Input monitoring stopped".to_string(),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "Start recording the monitored input on the dedicated recording-drain thread."
    )]
    pub(crate) async fn start_recording(&self, _params: Parameters<NoParams>) -> String {
        match self.bridge.start_recording() {
            Ok(()) => "Input recording started".to_string(),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(description = "Stop input recording and commit the captured audio as a library sample.")]
    pub(crate) async fn stop_recording(&self, params: Parameters<StopRecordingParam>) -> String {
        match self.bridge.stop_recording(params.0.name) {
            Ok(sample) => to_json(&sample),
            Err(e) => format!("Error: {e}"),
        }
    }

    // ========================================================================
    // DISCOVERY TOOLS
    // ========================================================================
}
