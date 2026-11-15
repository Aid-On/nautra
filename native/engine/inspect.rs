// SPDX-License-Identifier: Apache-2.0
//! Reading what a `.wasm` file is without compiling it: whether it is a core
//! module or a component, and the names it imports and exports. What a
//! workload imports is the outer bound of what it can ever ask the host for.

use serde_json::json;
use wasmparser::{Parser, Payload};

/// A component is a core module with one more layer; the header tells them
/// apart. After the magic come a 16-bit version (0x000d for components) and a
/// 16-bit layer (1 = component).
pub(super) fn is_component(bytes: &[u8]) -> bool {
    bytes.len() >= 8 && bytes[..4] == *b"\0asm" && bytes[4..8] == [0x0d, 0x00, 0x01, 0x00]
}

/// `{"kind": "module" | "component", "imports": […], "exports": […], "size": N}`
/// or `{"error": …}`. Module imports are spelled `module.name`.
pub fn nt_inspect(path: &str) -> String {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) => return json!({ "error": format!("cannot read {path}: {error}") }).to_string(),
    };
    match surface(&bytes) {
        Ok((imports, exports)) => json!({
            "kind": if is_component(&bytes) { "component" } else { "module" },
            "imports": imports,
            "exports": exports,
            "size": bytes.len(),
        })
        .to_string(),
        Err(reason) => json!({ "error": format!("{path} is not WebAssembly: {reason}") }).to_string(),
    }
}

/// The imports and exports of the outermost module or component. Nested ones
/// are walked past: only the outer surface meets the host.
fn surface(bytes: &[u8]) -> Result<(Vec<String>, Vec<String>), String> {
    if bytes.len() < 8 || bytes[..4] != *b"\0asm" {
        return Err("missing the \\0asm header".into());
    }
    let mut imports = Vec::new();
    let mut exports = Vec::new();
    let mut depth = 0usize;
    for payload in Parser::new(0).parse_all(bytes) {
        match payload.map_err(|error| error.to_string())? {
            Payload::ModuleSection { .. } | Payload::ComponentSection { .. } => depth += 1,
            Payload::End(_) => depth = depth.saturating_sub(1),
            Payload::ImportSection(reader) if depth == 0 => {
                for group in reader.into_imports() {
                    let import = group.map_err(|error| error.to_string())?;
                    imports.push(format!("{}.{}", import.module, import.name));
                }
            }
            Payload::ExportSection(reader) if depth == 0 => {
                for export in reader {
                    exports.push(export.map_err(|error| error.to_string())?.name.to_string());
                }
            }
            Payload::ComponentImportSection(reader) if depth == 0 => {
                for import in reader {
                    imports.push(import.map_err(|error| error.to_string())?.name.name.to_string());
                }
            }
            Payload::ComponentExportSection(reader) if depth == 0 => {
                for export in reader {
                    exports.push(export.map_err(|error| error.to_string())?.name.name.to_string());
                }
            }
            _ => {}
        }
    }
    Ok((imports, exports))
}
