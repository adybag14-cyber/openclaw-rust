use std::time::Instant;

use anyhow::{Context, Result};
use serde_json::{json, Value};
use wasmtime::component::Component;
use wasmtime::{
    Config as WasmtimeConfig, ExternType, Linker, Module, Store, StoreLimits, StoreLimitsBuilder,
    Val, ValType,
};

use crate::config::{ToolRuntimeWasmMode, ToolRuntimeWasmPolicyConfig};
use crate::wasm_sandbox::WasmInspection;

#[derive(Debug)]
struct WasmStoreState {
    limits: StoreLimits,
}

#[derive(Clone)]
pub struct WasmRuntime {
    cfg: ToolRuntimeWasmPolicyConfig,
    engine: wasmtime::Engine,
}

impl WasmRuntime {
    pub fn new(cfg: ToolRuntimeWasmPolicyConfig) -> Result<Self> {
        let mut engine_config = WasmtimeConfig::new();
        engine_config.consume_fuel(true);
        engine_config.wasm_component_model(true);
        let engine = wasmtime::Engine::new(&engine_config)
            .context("failed creating wasmtime engine for wasm runtime")?;
        Ok(Self { cfg, engine })
    }

    pub fn mode(&self) -> ToolRuntimeWasmMode {
        self.cfg.tool_runtime_mode
    }

    pub fn execute(
        &self,
        inspection: &WasmInspection,
        operation: &str,
        payload: &Value,
    ) -> Result<Value> {
        match self.cfg.tool_runtime_mode {
            ToolRuntimeWasmMode::InspectionStub => {
                Ok(self.stub_response(inspection, operation, payload))
            }
            ToolRuntimeWasmMode::WasmSandbox => {
                self.execute_wasmtime(inspection, operation, payload)
            }
        }
    }

    fn execute_wasmtime(
        &self,
        inspection: &WasmInspection,
        operation: &str,
        payload: &Value,
    ) -> Result<Value> {
        let started = Instant::now();
        let module_bytes = std::fs::read(&inspection.module_path).with_context(|| {
            format!(
                "failed reading wasm module bytes from {}",
                inspection.module_path
            )
        })?;

        match self.execute_core_module(inspection, operation, payload, &module_bytes) {
            Ok(core_result) => {
                let elapsed_ms = started.elapsed().as_millis() as u64;
                Ok(json!({
                    "status": "completed",
                    "engine": "wasmtime",
                    "runtimeMode": "wasm_sandbox",
                    "componentModelReady": true,
                    "operation": operation,
                    "module": inspection.module,
                    "modulePath": inspection.module_path,
                    "sha256": inspection.module_sha256,
                    "capabilitiesGranted": inspection.granted_capabilities,
                    "fuelLimit": inspection.fuel_limit,
                    "memoryLimitBytes": inspection.memory_limit_bytes,
                    "durationMs": elapsed_ms,
                    "result": core_result,
                    "aggregated": format!("wasm operation `{operation}` completed")
                }))
            }
            Err(core_error) => {
                let _component = Component::new(&self.engine, &module_bytes).with_context(|| {
                    format!(
                        "wasm module could not run as core module and is not a valid component: {core_error:#}"
                    )
                })?;
                let elapsed_ms = started.elapsed().as_millis() as u64;
                Ok(json!({
                    "status": "completed",
                    "engine": "wasmtime",
                    "runtimeMode": "wasm_sandbox",
                    "componentModelReady": true,
                    "componentValidated": true,
                    "operation": operation,
                    "module": inspection.module,
                    "modulePath": inspection.module_path,
                    "sha256": inspection.module_sha256,
                    "capabilitiesGranted": inspection.granted_capabilities,
                    "fuelLimit": inspection.fuel_limit,
                    "memoryLimitBytes": inspection.memory_limit_bytes,
                    "durationMs": elapsed_ms,
                    "result": {
                        "note": "component-model binary validated by wasmtime; dynamic function invocation requires typed canonical ABI bindings",
                        "input": payload
                    },
                    "aggregated": format!("wasm component `{}` validated (execution deferred to typed host adapter)", inspection.module)
                }))
            }
        }
    }

    fn execute_core_module(
        &self,
        inspection: &WasmInspection,
        requested_operation: &str,
        payload: &Value,
        module_bytes: &[u8],
    ) -> Result<Value> {
        let module = Module::new(&self.engine, module_bytes)
            .context("failed compiling wasm core module with wasmtime")?;
        let mut store = self.new_store(
            inspection.fuel_limit,
            inspection.memory_limit_bytes as usize,
        )?;
        let linker = Linker::<WasmStoreState>::new(&self.engine);
        let instance = linker
            .instantiate(&mut store, &module)
            .context("failed instantiating wasm core module (imports may be missing)")?;
        let (operation, func) =
            resolve_operation_and_function(&module, &instance, &mut store, requested_operation)?;
        let function_ty = func.ty(&store);
        let param_types = function_ty.params().collect::<Vec<_>>();
        let result_types = function_ty.results().collect::<Vec<_>>();

        if result_types.iter().any(|ty| !supported_val_type(ty)) {
            let unsupported = result_types
                .iter()
                .filter(|ty| !supported_val_type(ty))
                .map(describe_val_type)
                .collect::<Vec<_>>()
                .join(", ");
            anyhow::bail!(
                "wasm function `{operation}` returns unsupported result types for dynamic runtime: {unsupported}"
            );
        }

        let params = payload_to_vals(payload, &param_types)?;
        let mut results = result_types
            .iter()
            .map(default_result_val)
            .collect::<Result<Vec<_>>>()?;
        func.call(&mut store, &params, &mut results)
            .with_context(|| format!("wasm function call failed for operation `{operation}`"))?;

        let fuel_remaining = store.get_fuel().unwrap_or_default();
        let fuel_consumed = inspection.fuel_limit.saturating_sub(fuel_remaining);
        let json_results = results.iter().map(val_to_json).collect::<Vec<_>>();

        Ok(json!({
            "kind": "core_module",
            "operation": operation,
            "params": params.iter().map(val_to_json).collect::<Vec<_>>(),
            "results": json_results,
            "fuelConsumed": fuel_consumed
        }))
    }

    fn new_store(
        &self,
        fuel_limit: u64,
        memory_limit_bytes: usize,
    ) -> Result<Store<WasmStoreState>> {
        let limits = StoreLimitsBuilder::new()
            .memory_size(memory_limit_bytes)
            .build();
        let mut store = Store::new(&self.engine, WasmStoreState { limits });
        store.limiter(|state| &mut state.limits);
        store
            .set_fuel(fuel_limit)
            .context("failed applying wasm store fuel limit")?;
        Ok(store)
    }

    fn stub_response(
        &self,
        inspection: &WasmInspection,
        operation: &str,
        payload: &Value,
    ) -> Value {
        json!({
            "status": "completed",
            "engine": "wasmtime",
            "runtimeMode": "inspection_stub",
            "componentModelReady": true,
            "operation": operation,
            "module": inspection.module,
            "modulePath": inspection.module_path,
            "sha256": inspection.module_sha256,
            "capabilitiesGranted": inspection.granted_capabilities,
            "fuelLimit": inspection.fuel_limit,
            "memoryLimitBytes": inspection.memory_limit_bytes,
            "result": {
                "note": "inspection-only runtime mode selected; no guest code executed",
                "input": payload
            },
            "aggregated": format!("wasm inspection stub mode for module `{}`", inspection.module)
        })
    }
}

fn resolve_operation_and_function(
    module: &Module,
    instance: &wasmtime::Instance,
    store: &mut Store<WasmStoreState>,
    requested_operation: &str,
) -> Result<(String, wasmtime::Func)> {
    if let Some(func) = instance.get_func(&mut *store, requested_operation) {
        return Ok((requested_operation.to_owned(), func));
    }

    if requested_operation.eq_ignore_ascii_case("execute") {
        for fallback in ["run", "_start"] {
            if let Some(func) = instance.get_func(&mut *store, fallback) {
                return Ok((fallback.to_owned(), func));
            }
        }
    }

    for export in module.exports() {
        if matches!(export.ty(), ExternType::Func(_)) {
            let name = export.name().to_owned();
            if let Some(func) = instance.get_func(&mut *store, &name) {
                return Ok((name, func));
            }
        }
    }

    anyhow::bail!(
        "no callable wasm export found (requested operation `{requested_operation}` was unavailable)"
    )
}

fn payload_to_vals(payload: &Value, types: &[ValType]) -> Result<Vec<Val>> {
    let args = match payload {
        Value::Object(map) => match map.get("params") {
            Some(Value::Array(items)) => items.to_vec(),
            Some(other) => vec![other.clone()],
            None => Vec::new(),
        },
        Value::Array(items) => items.to_vec(),
        Value::Null => Vec::new(),
        other => vec![other.clone()],
    };

    if args.len() != types.len() {
        anyhow::bail!(
            "wasm call argument mismatch: expected {} argument(s), got {}",
            types.len(),
            args.len()
        );
    }

    let mut out = Vec::with_capacity(types.len());
    for (index, (value, ty)) in args.iter().zip(types.iter()).enumerate() {
        out.push(value_to_wasm_val(value, ty).with_context(|| {
            format!(
                "failed converting argument {} for wasm type {}",
                index,
                describe_val_type(ty)
            )
        })?);
    }
    Ok(out)
}

fn value_to_wasm_val(value: &Value, ty: &ValType) -> Result<Val> {
    match ty {
        ValType::I32 => {
            if let Some(v) = value.as_i64() {
                return Ok(Val::I32(v as i32));
            }
            if let Some(v) = value.as_u64() {
                return Ok(Val::I32(v as i32));
            }
            if let Some(v) = value.as_bool() {
                return Ok(Val::I32(if v { 1 } else { 0 }));
            }
            anyhow::bail!("expected i32-compatible value")
        }
        ValType::I64 => {
            if let Some(v) = value.as_i64() {
                return Ok(Val::I64(v));
            }
            if let Some(v) = value.as_u64() {
                return Ok(Val::I64(v as i64));
            }
            if let Some(v) = value.as_bool() {
                return Ok(Val::I64(if v { 1 } else { 0 }));
            }
            anyhow::bail!("expected i64-compatible value")
        }
        ValType::F32 => {
            let number = value
                .as_f64()
                .or_else(|| value.as_i64().map(|v| v as f64))
                .or_else(|| value.as_u64().map(|v| v as f64))
                .ok_or_else(|| anyhow::anyhow!("expected f32-compatible number"))?;
            Ok(Val::F32((number as f32).to_bits()))
        }
        ValType::F64 => {
            let number = value
                .as_f64()
                .or_else(|| value.as_i64().map(|v| v as f64))
                .or_else(|| value.as_u64().map(|v| v as f64))
                .ok_or_else(|| anyhow::anyhow!("expected f64-compatible number"))?;
            Ok(Val::F64(number.to_bits()))
        }
        _ => anyhow::bail!("unsupported wasm value type {}", describe_val_type(ty)),
    }
}

fn default_result_val(ty: &ValType) -> Result<Val> {
    match ty {
        ValType::I32 => Ok(Val::I32(0)),
        ValType::I64 => Ok(Val::I64(0)),
        ValType::F32 => Ok(Val::F32(0f32.to_bits())),
        ValType::F64 => Ok(Val::F64(0f64.to_bits())),
        _ => anyhow::bail!("unsupported wasm result type {}", describe_val_type(ty)),
    }
}

fn supported_val_type(ty: &ValType) -> bool {
    matches!(
        ty,
        ValType::I32 | ValType::I64 | ValType::F32 | ValType::F64
    )
}

fn describe_val_type(ty: &ValType) -> &'static str {
    match ty {
        ValType::I32 => "i32",
        ValType::I64 => "i64",
        ValType::F32 => "f32",
        ValType::F64 => "f64",
        ValType::V128 => "v128",
        ValType::Ref(_) => "ref",
    }
}

fn val_to_json(value: &Val) -> Value {
    match value {
        Val::I32(v) => json!(v),
        Val::I64(v) => json!(v),
        Val::F32(v) => json!(f32::from_bits(*v)),
        Val::F64(v) => json!(f64::from_bits(*v)),
        Val::V128(v) => json!(v.as_u128()),
        Val::ExternRef(reference) => json!(reference.is_some()),
        Val::FuncRef(reference) => json!(reference.is_some()),
        Val::AnyRef(reference) => json!(reference.is_some()),
    }
}
