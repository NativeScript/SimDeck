use crate::error::AppError;
use crate::metrics::counters::Metrics;
use crate::native::bridge::{NativeBridge, Simulator};
use crate::simulators::session::SimulatorSession;
use serde_json::json;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::task;

type SessionFactory<T> =
    Arc<dyn Fn(&NativeBridge, String, Arc<Metrics>) -> Result<T, AppError> + Send + Sync>;

#[derive(Clone)]
struct SessionStore<T> {
    sessions: Arc<Mutex<HashMap<String, T>>>,
}

impl<T> Default for SessionStore<T> {
    fn default() -> Self {
        Self {
            sessions: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

impl<T: Clone> SessionStore<T> {
    fn get(&self, key: &str) -> Option<T> {
        self.sessions.lock().unwrap().get(key).cloned()
    }

    fn get_or_create<F, E>(&self, key: &str, create: F) -> Result<T, E>
    where
        F: FnOnce() -> Result<T, E>,
    {
        let mut guard = self.sessions.lock().unwrap();
        if let Some(value) = guard.get(key) {
            return Ok(value.clone());
        }
        let value = create()?;
        guard.insert(key.to_owned(), value.clone());
        Ok(value)
    }

    fn remove(&self, key: &str) {
        self.sessions.lock().unwrap().remove(key);
    }

    fn forget(&self, key: &str) {
        if let Some(value) = self.sessions.lock().unwrap().remove(key) {
            std::mem::forget(value);
        }
    }

    fn values(&self) -> Vec<T>
    where
        T: Clone,
    {
        self.sessions.lock().unwrap().values().cloned().collect()
    }
}

#[derive(Clone)]
pub struct SessionRegistry<T = SimulatorSession> {
    bridge: NativeBridge,
    metrics: Arc<Metrics>,
    store: SessionStore<T>,
    session_factory: SessionFactory<T>,
}

impl SessionRegistry<SimulatorSession> {
    pub fn new(bridge: NativeBridge, metrics: Arc<Metrics>) -> Self {
        Self::with_factory(bridge, metrics, Arc::new(SimulatorSession::new))
    }
}

impl<T: Clone + Send + 'static> SessionRegistry<T> {
    fn with_factory(
        bridge: NativeBridge,
        metrics: Arc<Metrics>,
        session_factory: SessionFactory<T>,
    ) -> Self {
        Self {
            bridge,
            metrics,
            store: SessionStore::default(),
            session_factory,
        }
    }

    #[cfg(test)]
    fn new_for_tests(
        bridge: NativeBridge,
        metrics: Arc<Metrics>,
        session_factory: SessionFactory<T>,
    ) -> Self {
        Self::with_factory(bridge, metrics, session_factory)
    }

    pub fn bridge(&self) -> &NativeBridge {
        &self.bridge
    }

    pub fn get_or_create(&self, udid: &str) -> Result<T, AppError> {
        self.store.get_or_create(udid, || {
            (self.session_factory)(&self.bridge, udid.to_owned(), self.metrics.clone())
        })
    }

    pub async fn get_or_create_async(&self, udid: &str) -> Result<T, AppError> {
        let registry = self.clone();
        let udid_owned = udid.to_owned();
        task::spawn_blocking(move || registry.get_or_create(&udid_owned))
            .await
            .map_err(|error| {
                AppError::internal(format!("Failed to join session creation task: {error}"))
            })?
    }

    pub fn remove(&self, udid: &str) {
        self.store.remove(udid);
    }

    pub fn forget(&self, udid: &str) {
        self.store.forget(udid);
    }
}

impl SessionRegistry<SimulatorSession> {
    pub fn reconfigure_video_encoders(&self) {
        for session in self.store.values() {
            session.reconfigure_video_encoder();
        }
    }

    pub fn enrich_simulators(&self, simulators: Vec<Simulator>) -> Vec<serde_json::Value> {
        simulators
            .into_iter()
            .map(|simulator| {
                let private_display = self
                    .store
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

#[cfg(test)]
mod tests {
    use super::{SessionFactory, SessionRegistry};
    use crate::error::AppError;
    use crate::metrics::counters::Metrics;
    use crate::native::bridge::NativeBridge;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::thread;
    use std::time::Duration;
    use tokio::sync::Barrier;

    #[derive(Clone, Debug, PartialEq, Eq)]
    struct TestSession(usize);

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn concurrent_get_or_create_async_only_runs_factory_once() {
        let calls = Arc::new(AtomicUsize::new(0));
        let barrier = Arc::new(Barrier::new(17));
        let factory: SessionFactory<TestSession> = Arc::new({
            let calls = calls.clone();
            move |_, _, _| {
                let call = calls.fetch_add(1, Ordering::SeqCst) + 1;
                thread::sleep(Duration::from_millis(20));
                Ok::<_, AppError>(TestSession(call))
            }
        });
        let registry =
            SessionRegistry::new_for_tests(NativeBridge, Arc::new(Metrics::default()), factory);

        let mut handles = Vec::new();
        for _ in 0..16 {
            let registry = registry.clone();
            let barrier = barrier.clone();
            handles.push(tokio::spawn(async move {
                barrier.wait().await;
                registry.get_or_create_async("booted-sim").await
            }));
        }

        barrier.wait().await;

        let mut sessions = Vec::new();
        for handle in handles {
            sessions.push(handle.await.unwrap().unwrap());
        }

        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert!(sessions.iter().all(|session| *session == TestSession(1)));
    }

    #[tokio::test]
    async fn remove_allows_a_fresh_session_to_be_created() {
        let calls = Arc::new(AtomicUsize::new(0));
        let factory: SessionFactory<TestSession> = Arc::new({
            let calls = calls.clone();
            move |_, _, _| {
                let call = calls.fetch_add(1, Ordering::SeqCst) + 1;
                Ok::<_, AppError>(TestSession(call))
            }
        });
        let registry =
            SessionRegistry::new_for_tests(NativeBridge, Arc::new(Metrics::default()), factory);

        let first = registry.get_or_create_async("booted-sim").await.unwrap();
        registry.remove("booted-sim");
        let second = registry.get_or_create_async("booted-sim").await.unwrap();

        assert_eq!(calls.load(Ordering::SeqCst), 2);
        assert_eq!(first, TestSession(1));
        assert_eq!(second, TestSession(2));
    }
}
