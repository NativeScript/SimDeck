use crate::error::AppError;
use crate::metrics::counters::Metrics;
use crate::native::bridge::{NativeBridge, Simulator};
use crate::simulators::session::SimulatorSession;
use serde_json::json;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::task;

#[derive(Clone)]
pub struct SessionRegistry {
    bridge: NativeBridge,
    metrics: Arc<Metrics>,
    sessions: Arc<Mutex<HashMap<String, SimulatorSession>>>,
}

impl SessionRegistry {
    pub fn new(bridge: NativeBridge, metrics: Arc<Metrics>) -> Self {
        Self {
            bridge,
            metrics,
            sessions: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn bridge(&self) -> &NativeBridge {
        &self.bridge
    }

    pub fn get_or_create(&self, udid: &str) -> Result<SimulatorSession, AppError> {
        let mut guard = self.sessions.lock().unwrap();
        if let Some(session) = guard.get(udid) {
            return Ok(session.clone());
        }
        let session = SimulatorSession::new(&self.bridge, udid.to_owned(), self.metrics.clone())?;
        guard.insert(udid.to_owned(), session.clone());
        Ok(session)
    }

    pub async fn get_or_create_async(&self, udid: &str) -> Result<SimulatorSession, AppError> {
        {
            let guard = self.sessions.lock().unwrap();
            if let Some(session) = guard.get(udid) {
                return Ok(session.clone());
            }
        }

        let udid_owned = udid.to_owned();
        let bridge = self.bridge.clone();
        let metrics = self.metrics.clone();
        let session = task::spawn_blocking(move || {
            SimulatorSession::new(&bridge, udid_owned.clone(), metrics)
        })
        .await
        .map_err(|error| {
            AppError::internal(format!("Failed to join session creation task: {error}"))
        })??;

        let mut guard = self.sessions.lock().unwrap();
        if let Some(existing) = guard.get(udid) {
            return Ok(existing.clone());
        }
        guard.insert(udid.to_owned(), session.clone());
        Ok(session)
    }

    pub fn remove(&self, udid: &str) {
        self.sessions.lock().unwrap().remove(udid);
    }

    pub fn enrich_simulators(&self, simulators: Vec<Simulator>) -> Vec<serde_json::Value> {
        let guard = self.sessions.lock().unwrap();
        simulators
            .into_iter()
            .map(|simulator| {
                let private_display = guard
                    .get(&simulator.udid)
                    .map(|session| session.snapshot())
                    .unwrap_or_else(|| {
                        json!({
                            "displayReady": false,
                            "displayStatus": if simulator.is_booted { "Detached" } else { "Boot required" },
                            "displayWidth": 0,
                            "displayHeight": 0,
                            "frameSequence": 0,
                        })
                    });
                json!({
                    "udid": simulator.udid,
                    "name": simulator.name,
                    "state": simulator.state,
                    "isBooted": simulator.is_booted,
                    "isAvailable": simulator.is_available,
                    "lastBootedAt": simulator.last_booted_at,
                    "dataPath": simulator.data_path,
                    "logPath": simulator.log_path,
                    "deviceTypeIdentifier": simulator.device_type_identifier,
                    "deviceTypeName": simulator.device_type_name,
                    "runtimeIdentifier": simulator.runtime_identifier,
                    "runtimeName": simulator.runtime_name,
                    "privateDisplay": private_display,
                })
            })
            .collect()
    }
}
