use super::*;

impl synth_mcp::bridge::DiscoveryBridge for AppSynthBridge {
    fn get_module_type_info(&self, type_key: &str) -> Result<ModuleTypeInfo, McpBridgeError> {
        use crate::module_factory::{ALL_MODULE_TYPES, get_descriptor};

        let mt = parse_module_type(type_key)
            .ok_or_else(|| McpBridgeError::InvalidModuleType(type_key.to_string()))?;

        if !ALL_MODULE_TYPES.contains(&mt) {
            return Err(McpBridgeError::InvalidModuleType(type_key.to_string()));
        }

        let desc = get_descriptor(mt)
            .ok_or_else(|| McpBridgeError::InvalidModuleType(type_key.to_string()))?;

        Ok(build_module_type_info(mt, &desc))
    }

    fn search_modules(
        &self,
        category: Option<&str>,
        has_input_type: Option<&str>,
        has_output_type: Option<&str>,
        query: Option<&str>,
    ) -> Result<ModuleSearchResult, McpBridgeError> {
        Ok(search_module_types(
            category,
            has_input_type,
            has_output_type,
            query,
        ))
    }

    fn check_connection(
        &self,
        instrument_id: InstrumentId,
        from_module: &str,
        from_port: &str,
        to_module: &str,
        to_port: &str,
    ) -> Result<ConnectionCheckResult, McpBridgeError> {
        self.validate_instrument(instrument_id)?;
        let inst_id = instrument_id;

        let from_mid: ModuleId = from_module
            .parse()
            .map_err(|_| McpBridgeError::ModuleNotFound(from_module.to_string()))?;
        let to_mid: ModuleId = to_module
            .parse()
            .map_err(|_| McpBridgeError::ModuleNotFound(to_module.to_string()))?;

        let from_desc = self
            .session
            .module_descriptor(inst_id, from_mid)
            .ok_or_else(|| McpBridgeError::ModuleNotFound(from_module.to_string()))?;
        let to_desc = self
            .session
            .module_descriptor(inst_id, to_mid)
            .ok_or_else(|| McpBridgeError::ModuleNotFound(to_module.to_string()))?;

        // Honor the same output/input aliases connect() accepts so the diagnostic agrees.
        let alias_of = |ports: &[PortDescriptor], req: &str| -> String {
            match port_alias(req) {
                Some(a) if ports.iter().any(|p| p.name.as_str() == a) => a.to_string(),
                _ => req.to_string(),
            }
        };
        let from_port = alias_of(&from_desc.ports, from_port);
        let to_port = alias_of(&to_desc.ports, to_port);

        let from_port_desc = from_desc
            .ports
            .iter()
            .find(|p| p.name == from_port.as_str());
        let to_port_desc = to_desc.ports.iter().find(|p| p.name == to_port.as_str());

        let Some(from_pd) = from_port_desc else {
            let available: Vec<&str> = from_desc.ports.iter().map(|p| p.name.as_str()).collect();
            return Ok(ConnectionCheckResult {
                valid: false,
                from_signal_type: None,
                to_signal_type: None,
                message: format!(
                    "Port '{}' not found on module '{}'.",
                    from_port, from_module
                ),
                hint: Some(format!("Available ports: {}", available.join(", "))),
            });
        };

        let Some(to_pd) = to_port_desc else {
            let available: Vec<&str> = to_desc.ports.iter().map(|p| p.name.as_str()).collect();
            return Ok(ConnectionCheckResult {
                valid: false,
                from_signal_type: None,
                to_signal_type: None,
                message: format!("Port '{}' not found on module '{}'.", to_port, to_module),
                hint: Some(format!("Available ports: {}", available.join(", "))),
            });
        };

        let from_type_str = port_type_str(from_pd.port_type);
        let to_type_str = port_type_str(to_pd.port_type);

        if from_pd.direction != PortDirection::Output {
            let outputs: Vec<&str> = from_desc
                .ports
                .iter()
                .filter(|p| p.direction == PortDirection::Output)
                .map(|p| p.name.as_str())
                .collect();
            return Ok(ConnectionCheckResult {
                valid: false,
                from_signal_type: Some(from_type_str.to_string()),
                to_signal_type: Some(to_type_str.to_string()),
                message: format!(
                    "'{}' on '{}' is an input port, not an output.",
                    from_port, from_module
                ),
                hint: Some(format!(
                    "Output ports on '{}': {}",
                    from_module,
                    if outputs.is_empty() {
                        "(none)".to_string()
                    } else {
                        outputs.join(", ")
                    }
                )),
            });
        };

        if to_pd.direction != PortDirection::Input {
            let inputs: Vec<&str> = to_desc
                .ports
                .iter()
                .filter(|p| p.direction == PortDirection::Input)
                .map(|p| p.name.as_str())
                .collect();
            return Ok(ConnectionCheckResult {
                valid: false,
                from_signal_type: Some(from_type_str.to_string()),
                to_signal_type: Some(to_type_str.to_string()),
                message: format!(
                    "'{}' on '{}' is an output port, not an input.",
                    to_port, to_module
                ),
                hint: Some(format!(
                    "Input ports on '{}': {}",
                    to_module,
                    if inputs.is_empty() {
                        "(none)".to_string()
                    } else {
                        inputs.join(", ")
                    }
                )),
            });
        };

        let compatible = from_pd.port_type.can_drive(to_pd.port_type);

        if compatible {
            let note = if from_pd.port_type != to_pd.port_type {
                format!(
                    " (cross-type: {} → {}, signal will be interpreted as {})",
                    from_type_str, to_type_str, to_type_str
                )
            } else {
                String::new()
            };
            Ok(ConnectionCheckResult {
                valid: true,
                from_signal_type: Some(from_type_str.to_string()),
                to_signal_type: Some(to_type_str.to_string()),
                message: format!(
                    "Valid connection: {}:{} → {}:{}{}",
                    from_module, from_port, to_module, to_port, note
                ),
                hint: None,
            })
        } else {
            Ok(ConnectionCheckResult {
                valid: false,
                from_signal_type: Some(from_type_str.to_string()),
                to_signal_type: Some(to_type_str.to_string()),
                message: format!(
                    "Incompatible signal types: {} output → {} input.",
                    from_type_str, to_type_str
                ),
                hint: Some(format!(
                    "{} ports can connect to: {}",
                    from_type_str,
                    compatible_types_hint(from_pd.port_type)
                )),
            })
        }
    }
}
